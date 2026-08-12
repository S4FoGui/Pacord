use crate::capture::{CaptureConfig, CapturedFrame, FrameSource, XShmCapture};
use crate::input::InputManager;
use crate::transport::{FramePacket, FrameServer};
use image::{imageops::FilterType, RgbaImage};
use std::error::Error;
use std::net::SocketAddr;
use std::sync::{mpsc::sync_channel, Arc};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

fn encode_frame(
    frame: CapturedFrame,
    config: &CaptureConfig,
) -> Result<FramePacket, Box<dyn Error + Send + Sync>> {
    let source = RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
        .ok_or("frame RGBA possui tamanho inconsistente")?;
    let (image, width, height) =
        if frame.width > config.max_width || frame.height > config.max_height {
            let scale = (config.max_width as f32 / frame.width as f32)
                .min(config.max_height as f32 / frame.height as f32);
            let width = ((frame.width as f32 * scale).round() as u32).max(1);
            let height = ((frame.height as f32 * scale).round() as u32).max(1);
            (
                image::imageops::resize(&source, width, height, FilterType::Triangle),
                width,
                height,
            )
        } else {
            (source, frame.width, frame.height)
        };

    let rgb = image::DynamicImage::ImageRgba8(image).to_rgb8();
    let mut encoded = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
        std::io::Cursor::new(&mut encoded),
        config.jpeg_quality.clamp(1, 100),
    );
    encoder.encode(&rgb, width, height, image::ExtendedColorType::Rgb8)?;
    Ok(FramePacket {
        sequence: frame.sequence,
        captured_at_micros: frame.captured_at_micros,
        width,
        height,
        jpeg: encoded,
    })
}

pub async fn run_x11_host(
    bind_addr: SocketAddr,
    secret: Vec<u8>,
    config: CaptureConfig,
    input_manager: Arc<InputManager>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (frames, _) = broadcast::channel::<FramePacket>(2);
    let server = FrameServer::new(bind_addr, secret)?.with_input_manager(input_manager);
    let server_frames = frames.clone();
    let server_task = tokio::spawn(async move { server.run(server_frames).await });

    let capture_frames = frames.clone();
    let capture_task =
        tokio::task::spawn_blocking(move || -> Result<(), Box<dyn Error + Send + Sync>> {
            let mut capture = XShmCapture::new()?;
            let frame_interval = Duration::from_secs_f64(1.0 / config.target_fps.max(1) as f64);
            loop {
                let started = Instant::now();
                let frame = capture.capture_frame()?;
                let packet = encode_frame(frame, &config)?;
                let _ = capture_frames.send(packet);
                if let Some(remaining) = frame_interval.checked_sub(started.elapsed()) {
                    std::thread::sleep(remaining);
                }
            }
        });

    tokio::select! {
        result = server_task => { result??; }
        result = capture_task => { result??; }
    }
    Ok(())
}

pub async fn run_wayland_host(
    bind_addr: SocketAddr,
    secret: Vec<u8>,
    config: CaptureConfig,
    input_manager: Arc<InputManager>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (frames, _) = broadcast::channel::<FramePacket>(2);
    let server = FrameServer::new(bind_addr, secret)?.with_input_manager(input_manager);
    let server_frames = frames.clone();
    let server_task = tokio::spawn(async move { server.run(server_frames).await });

    let (capture_sender, capture_receiver) = sync_channel::<CapturedFrame>(2);
    let portal_task = tokio::spawn(crate::capture::run_wayland_capture(capture_sender));
    let publish_frames = frames.clone();
    let publish_task = tokio::task::spawn_blocking(move || {
        while let Ok(frame) = capture_receiver.recv() {
            match encode_frame(frame, &config) {
                Ok(packet) => {
                    let _ = publish_frames.send(packet);
                }
                Err(error) => log::warn!("frame descartado: {error}"),
            }
        }
    });

    tokio::select! {
        result = server_task => { result??; }
        result = portal_task => { result??; }
        result = publish_task => { result?; }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{encode_frame, CaptureConfig};
    use crate::capture::{CaptureBackendKind, CapturedFrame};

    #[test]
    fn encode_frame_resizes_to_configured_bound() {
        let frame = CapturedFrame {
            sequence: 1,
            captured_at_micros: 10,
            width: 4,
            height: 2,
            rgba: vec![255; 4 * 2 * 4],
            backend: CaptureBackendKind::X11XShm,
        };
        let config = CaptureConfig {
            max_width: 2,
            max_height: 2,
            target_fps: 30,
            jpeg_quality: 75,
        };
        let packet = encode_frame(frame, &config).expect("frame deve codificar");
        assert_eq!((packet.width, packet.height), (2, 1));
        assert!(!packet.jpeg.is_empty());
    }
}
