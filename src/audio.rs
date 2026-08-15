use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const MAX_PLAYBACK_SAMPLES: usize = 48_000 * 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoicePacket {
    pub sequence: u64,
    pub source_client_id: usize,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl VoicePacket {
    pub fn is_valid(&self) -> bool {
        self.sample_rate > 0
            && self.sample_rate <= 192_000
            && self.channels > 0
            && self.channels <= 8
            && !self.samples.is_empty()
            && self.samples.len() % self.channels as usize == 0
            && self.samples.len() <= 48_000 * 8
    }
}

#[derive(Debug, Clone)]
pub struct AudioError(String);

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AudioError {}

struct AudioInput {
    _stream: cpal::Stream,
}

struct AudioOutput {
    _stream: cpal::Stream,
    queue: Arc<Mutex<VecDeque<i16>>>,
    sample_rate: u32,
    channels: u16,
}

pub struct VoiceRuntime {
    input: Option<AudioInput>,
    output: Option<AudioOutput>,
}

impl VoiceRuntime {
    pub fn start(
        source_client_id: usize,
        sender: Arc<dyn Fn(VoicePacket) + Send + Sync>,
    ) -> Result<Self, AudioError> {
        let input = AudioInput::start(source_client_id, sender)?;
        let output = AudioOutput::start()?;
        Ok(Self {
            input: Some(input),
            output: Some(output),
        })
    }

    pub fn start_best_effort(
        source_client_id: usize,
        sender: Arc<dyn Fn(VoicePacket) + Send + Sync>,
    ) -> Self {
        let input = match AudioInput::start(source_client_id, sender) {
            Ok(input) => Some(input),
            Err(error) => {
                log::warn!("captura de voz indisponível: {error}");
                None
            }
        };
        let output = match AudioOutput::start() {
            Ok(output) => Some(output),
            Err(error) => {
                log::warn!("reprodução de voz indisponível: {error}");
                None
            }
        };
        Self { input, output }
    }

    pub fn push(&self, packet: &VoicePacket) {
        if let Some(output) = &self.output {
            output.push(packet);
        }
    }

    pub fn has_input(&self) -> bool {
        self.input.is_some()
    }

    pub fn has_output(&self) -> bool {
        self.output.is_some()
    }
}

impl AudioInput {
    fn start(
        source_client_id: usize,
        sender: Arc<dyn Fn(VoicePacket) + Send + Sync>,
    ) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| AudioError("nenhum microfone padrão foi encontrado".into()))?;
        let supported = device
            .default_input_config()
            .map_err(|error| AudioError(format!("não foi possível abrir o microfone: {error}")))?;
        let config: cpal::StreamConfig = supported.clone().into();
        let channels = config.channels;
        let sample_rate = config.sample_rate;
        let sequence = Arc::new(AtomicU64::new(0));
        let error_callback = |error| log::warn!("erro no microfone PACORD: {error}");

        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => {
                let sequence = sequence.clone();
                let sender = sender.clone();
                device.build_input_stream(
                    config.clone(),
                    move |data: &[f32], _| {
                        let samples = data
                            .iter()
                            .map(|sample| (*sample * 32767.0).clamp(-32768.0, 32767.0) as i16)
                            .collect();
                        sender(VoicePacket {
                            sequence: sequence.fetch_add(1, Ordering::Relaxed),
                            source_client_id,
                            sample_rate,
                            channels,
                            samples,
                        });
                    },
                    error_callback,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let sequence = sequence.clone();
                let sender = sender.clone();
                device.build_input_stream(
                    config.clone(),
                    move |data: &[i16], _| {
                        sender(VoicePacket {
                            sequence: sequence.fetch_add(1, Ordering::Relaxed),
                            source_client_id,
                            sample_rate,
                            channels,
                            samples: data.to_vec(),
                        });
                    },
                    error_callback,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let sequence = sequence.clone();
                let sender = sender.clone();
                device.build_input_stream(
                    config.clone(),
                    move |data: &[u16], _| {
                        let samples = data
                            .iter()
                            .map(|sample| (*sample as i32 - 32768) as i16)
                            .collect();
                        sender(VoicePacket {
                            sequence: sequence.fetch_add(1, Ordering::Relaxed),
                            source_client_id,
                            sample_rate,
                            channels,
                            samples,
                        });
                    },
                    error_callback,
                    None,
                )
            }
            format => {
                return Err(AudioError(format!(
                    "formato de microfone não suportado: {format:?}"
                )))
            }
        }
        .map_err(|error| {
            AudioError(format!(
                "não foi possível criar o fluxo do microfone: {error}"
            ))
        })?;
        stream.play().map_err(|error| {
            AudioError(format!("não foi possível iniciar o microfone: {error}"))
        })?;
        Ok(Self { _stream: stream })
    }
}

impl AudioOutput {
    fn start() -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or_else(|| {
            AudioError("nenhum dispositivo de saída padrão foi encontrado".into())
        })?;
        let supported = device.default_output_config().map_err(|error| {
            AudioError(format!("não foi possível abrir a saída de áudio: {error}"))
        })?;
        let config: cpal::StreamConfig = supported.clone().into();
        let sample_rate = config.sample_rate;
        let channels = config.channels;
        let queue = Arc::new(Mutex::new(VecDeque::<i16>::with_capacity(
            MAX_PLAYBACK_SAMPLES,
        )));
        let callback_queue = queue.clone();
        let error_callback = |error| log::warn!("erro na saída de voz PACORD: {error}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                config.clone(),
                move |data: &mut [f32], _| fill_f32(data, &callback_queue),
                error_callback,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                config.clone(),
                move |data: &mut [i16], _| fill_i16(data, &callback_queue),
                error_callback,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_output_stream(
                config.clone(),
                move |data: &mut [u16], _| fill_u16(data, &callback_queue),
                error_callback,
                None,
            ),
            format => {
                return Err(AudioError(format!(
                    "formato de saída não suportado: {format:?}"
                )))
            }
        }
        .map_err(|error| AudioError(format!("não foi possível criar a saída de voz: {error}")))?;
        stream.play().map_err(|error| {
            AudioError(format!("não foi possível iniciar a saída de voz: {error}"))
        })?;
        Ok(Self {
            _stream: stream,
            queue,
            sample_rate,
            channels,
        })
    }

    fn push(&self, packet: &VoicePacket) {
        if !packet.is_valid() {
            return;
        }
        let source_channels = packet.channels as usize;
        let source_frames = packet.samples.len() / source_channels;
        let target_frames = ((source_frames as f64 * self.sample_rate as f64
            / packet.sample_rate as f64)
            .round() as usize)
            .max(1);
        let mut queue = self.queue.lock().expect("fila de áudio PACORD");
        for target_frame in 0..target_frames {
            let source_frame = ((target_frame as f64 * packet.sample_rate as f64
                / self.sample_rate as f64)
                .floor() as usize)
                .min(source_frames - 1);
            let base = source_frame * source_channels;
            let sample = if source_channels == 1 {
                packet.samples[base]
            } else {
                let sum: i32 = packet.samples[base..base + source_channels]
                    .iter()
                    .map(|sample| *sample as i32)
                    .sum();
                (sum / source_channels as i32) as i16
            };
            for _ in 0..self.channels {
                if queue.len() >= MAX_PLAYBACK_SAMPLES {
                    queue.pop_front();
                }
                queue.push_back(sample);
            }
        }
    }
}

fn pop_sample(queue: &Arc<Mutex<VecDeque<i16>>>) -> i16 {
    queue
        .lock()
        .expect("fila de áudio PACORD")
        .pop_front()
        .unwrap_or(0)
}

fn fill_f32(data: &mut [f32], queue: &Arc<Mutex<VecDeque<i16>>>) {
    for sample in data {
        *sample = pop_sample(queue) as f32 / 32768.0;
    }
}

fn fill_i16(data: &mut [i16], queue: &Arc<Mutex<VecDeque<i16>>>) {
    for sample in data {
        *sample = pop_sample(queue);
    }
}

fn fill_u16(data: &mut [u16], queue: &Arc<Mutex<VecDeque<i16>>>) {
    for sample in data {
        *sample = (pop_sample(queue) as i32 + 32768).clamp(0, 65535) as u16;
    }
}

#[cfg(test)]
mod tests {
    use super::VoicePacket;

    #[test]
    fn voice_packet_validation_rejects_malformed_audio() {
        let packet = VoicePacket {
            sequence: 1,
            source_client_id: 2,
            sample_rate: 48_000,
            channels: 1,
            samples: vec![0; 960],
        };
        assert!(packet.is_valid());
        assert!(!VoicePacket {
            samples: vec![0, 1],
            channels: 3,
            ..packet
        }
        .is_valid());
    }
}

#[allow(dead_code)]
fn _audio_streams_are_kept_alive(input: &AudioInput, output: &AudioOutput) {
    let _ = (&input._stream, &output._stream);
}
