use ashpd::desktop::{
    screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
    PersistMode,
};
use image::{codecs::jpeg::JpegEncoder, ColorType};
use libc::{c_void, IPC_CREAT, IPC_PRIVATE, IPC_RMID};
use pipewire as pw;
use pw::{properties::properties, spa};
use std::error::Error;
use std::ffi::c_char;
use std::io::Cursor;
use std::os::fd::OwnedFd;
use std::os::raw::c_int;
use std::ptr::{null, null_mut};
use std::sync::mpsc::SyncSender;
use std::time::{SystemTime, UNIX_EPOCH};
use x11_dl::{xlib, xshm};

pub const DEFAULT_FPS: u32 = 30;
pub const DEFAULT_MAX_WIDTH: u32 = 1280;
pub const DEFAULT_MAX_HEIGHT: u32 = 720;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackendKind {
    WaylandPipeWire,
    X11XShm,
}

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    pub max_width: u32,
    pub max_height: u32,
    pub target_fps: u32,
    pub jpeg_quality: u8,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            max_width: DEFAULT_MAX_WIDTH,
            max_height: DEFAULT_MAX_HEIGHT,
            target_fps: DEFAULT_FPS,
            jpeg_quality: 75,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub sequence: u64,
    pub captured_at_micros: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub backend: CaptureBackendKind,
}

impl CapturedFrame {
    pub fn jpeg(&self, quality: u8) -> Result<Vec<u8>, image::ImageError> {
        let rgba = image::RgbaImage::from_raw(self.width, self.height, self.rgba.clone())
            .ok_or_else(|| {
                image::ImageError::Limits(image::error::LimitError::from_kind(
                    image::error::LimitErrorKind::DimensionError,
                ))
            })?;
        let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
        let mut encoded = Vec::new();
        let mut encoder =
            JpegEncoder::new_with_quality(Cursor::new(&mut encoded), quality.clamp(1, 100));
        encoder.encode(&rgb, self.width, self.height, ColorType::Rgb8.into())?;
        Ok(encoded)
    }
}

pub trait FrameSource {
    fn capture_frame(&mut self) -> Result<CapturedFrame, Box<dyn Error + Send + Sync>>;
}

fn timestamp_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

unsafe extern "C" fn ignore_x11_error(
    _display: *mut xlib::Display,
    _error: *mut xlib::XErrorEvent,
) -> c_int {
    0
}

pub struct XShmCapture {
    xlib: xlib::Xlib,
    xext: xshm::Xext,
    previous_error_handler:
        Option<unsafe extern "C" fn(*mut xlib::Display, *mut xlib::XErrorEvent) -> c_int>,
    display: *mut xlib::Display,
    root: xlib::Drawable,
    image: *mut xlib::XImage,
    shm_info: xshm::XShmSegmentInfo,
    width: u32,
    height: u32,
    sequence: u64,
}

impl XShmCapture {
    pub fn new() -> Result<Self, Box<dyn Error + Send + Sync>> {
        unsafe {
            let xlib = xlib::Xlib::open()?;
            let xext = xshm::Xext::open()?;
            let display = (xlib.XOpenDisplay)(null());
            if display.is_null() {
                return Err("não foi possível abrir o DISPLAY X11".into());
            }

            if (xext.XShmQueryExtension)(display) == 0 {
                (xlib.XCloseDisplay)(display);
                return Err("o servidor X11 não anuncia a extensão MIT-SHM/XShm".into());
            }

            let previous_error_handler = (xlib.XSetErrorHandler)(Some(ignore_x11_error));
            let screen = (xlib.XDefaultScreen)(display);
            let width = (xlib.XDisplayWidth)(display, screen);
            let height = (xlib.XDisplayHeight)(display, screen);
            let root = (xlib.XDefaultRootWindow)(display);
            let visual = (xlib.XDefaultVisual)(display, screen);
            let depth = (xlib.XDefaultDepth)(display, screen);
            if width <= 0 || height <= 0 || visual.is_null() || depth <= 0 {
                (xlib.XCloseDisplay)(display);
                return Err("dimensões ou visual X11 inválidos".into());
            }

            let mut shm_info: xshm::XShmSegmentInfo = std::mem::zeroed();
            let image = (xext.XShmCreateImage)(
                display,
                visual,
                depth as u32,
                xlib::ZPixmap,
                null_mut(),
                &mut shm_info,
                width as u32,
                height as u32,
            );
            if image.is_null() {
                (xlib.XCloseDisplay)(display);
                return Err("XShmCreateImage falhou".into());
            }

            let bytes_per_line = (*image).bytes_per_line;
            let size = (bytes_per_line as usize)
                .checked_mul(height as usize)
                .ok_or("tamanho de framebuffer X11 excede o limite")?;
            let shmid = libc::shmget(IPC_PRIVATE, size, IPC_CREAT | 0o600);
            if shmid < 0 {
                (xlib.XDestroyImage)(image);
                (xlib.XCloseDisplay)(display);
                return Err(std::io::Error::last_os_error().into());
            }

            let shmaddr = libc::shmat(shmid, null(), 0);
            if shmaddr as isize == -1 {
                libc::shmctl(shmid, IPC_RMID, null_mut());
                (xlib.XDestroyImage)(image);
                (xlib.XCloseDisplay)(display);
                return Err(std::io::Error::last_os_error().into());
            }

            shm_info.shmid = shmid;
            shm_info.shmaddr = shmaddr as *mut c_char;
            shm_info.readOnly = 0;
            (*image).data = shmaddr as *mut c_char;

            if (xext.XShmAttach)(display, &mut shm_info) == 0 {
                libc::shmdt(shmaddr);
                libc::shmctl(shmid, IPC_RMID, null_mut());
                (*image).data = null_mut();
                (xlib.XDestroyImage)(image);
                (xlib.XCloseDisplay)(display);
                return Err("XShmAttach falhou".into());
            }
            // Mantemos o segmento até o Drop: alguns servidores X acessam o
            // identificador novamente durante a primeira captura.
            (xlib.XSync)(display, 0);

            Ok(Self {
                xlib,
                xext,
                previous_error_handler,
                display,
                root,
                image,
                shm_info,
                width: width as u32,
                height: height as u32,
                sequence: 0,
            })
        }
    }

    fn component(pixel: u32, mask: u64) -> u8 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let max = mask >> shift;
        let value = ((pixel as u64 & mask) >> shift) as u64;
        ((value * 255 + max / 2) / max).min(255) as u8
    }

    unsafe fn convert_to_rgba(&self, image_ptr: *const xlib::XImage) -> Vec<u8> {
        let image = &*image_ptr;
        let bpp = ((image.bits_per_pixel / 8).max(1)) as usize;
        let stride = image.bytes_per_line as usize;
        let total = (self.width * self.height * 4) as usize;
        let mut rgba = Vec::with_capacity(total);
        let base = image.data as *const u8;

        for y in 0..self.height as usize {
            let row = base.add(y * stride);
            for x in 0..self.width as usize {
                let pixel_ptr = row.add(x * bpp);
                let mut pixel = 0u32;
                if image.byte_order == xlib::LSBFirst {
                    for i in 0..bpp.min(4) {
                        pixel |= (*pixel_ptr.add(i) as u32) << (i * 8);
                    }
                } else {
                    for i in 0..bpp.min(4) {
                        pixel = (pixel << 8) | *pixel_ptr.add(i) as u32;
                    }
                }
                rgba.push(Self::component(pixel, image.red_mask));
                rgba.push(Self::component(pixel, image.green_mask));
                rgba.push(Self::component(pixel, image.blue_mask));
                rgba.push(255);
            }
        }
        rgba
    }
}

impl FrameSource for XShmCapture {
    fn capture_frame(&mut self) -> Result<CapturedFrame, Box<dyn Error + Send + Sync>> {
        unsafe {
            let rgba = if (self.xext.XShmGetImage)(
                self.display,
                self.root,
                self.image,
                0,
                0,
                (self.xlib.XAllPlanes)() as u32,
            ) != 0
            {
                self.convert_to_rgba(self.image)
            } else {
                // Alguns servidores anunciam MIT-SHM, mas recusam a imagem
                // compartilhada por restrições de sandbox. Use XGetImage como
                // fallback explícito em vez de encerrar a sessão inteira.
                let fallback = (self.xlib.XGetImage)(
                    self.display,
                    self.root,
                    0,
                    0,
                    self.width,
                    self.height,
                    (self.xlib.XAllPlanes)(),
                    xlib::ZPixmap,
                );
                if fallback.is_null() {
                    return Err("XShmGetImage e XGetImage falharam".into());
                }
                let rgba = self.convert_to_rgba(fallback);
                (self.xlib.XDestroyImage)(fallback);
                rgba
            };
            self.sequence = self.sequence.wrapping_add(1);
            Ok(CapturedFrame {
                sequence: self.sequence,
                captured_at_micros: timestamp_micros(),
                width: self.width,
                height: self.height,
                rgba,
                backend: CaptureBackendKind::X11XShm,
            })
        }
    }
}

impl Drop for XShmCapture {
    fn drop(&mut self) {
        unsafe {
            if !self.display.is_null() {
                (self.xext.XShmDetach)(self.display, &mut self.shm_info);
                (self.xlib.XSync)(self.display, 0);
            }
            if !self.shm_info.shmaddr.is_null() {
                libc::shmdt(self.shm_info.shmaddr as *const c_void);
            }
            if self.shm_info.shmid >= 0 {
                libc::shmctl(self.shm_info.shmid, IPC_RMID, null_mut());
            }
            if !self.image.is_null() {
                (*self.image).data = null_mut();
                (self.xlib.XDestroyImage)(self.image);
            }
            if !self.display.is_null() {
                (self.xlib.XSetErrorHandler)(self.previous_error_handler);
                (self.xlib.XCloseDisplay)(self.display);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct PortalStreamInfo {
    pub node_id: u32,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

struct PipeWireUserData {
    format: spa::param::video::VideoInfoRaw,
    tx: SyncSender<CapturedFrame>,
    sequence: u64,
}

fn pipewire_format_to_rgba(
    bytes: &[u8],
    format: spa::param::video::VideoFormat,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let pixels = (width as usize).checked_mul(height as usize)?;
    let mut rgba = Vec::with_capacity(pixels.checked_mul(4)?);
    match format {
        spa::param::video::VideoFormat::RGBA => {
            if bytes.len() < pixels * 4 {
                return None;
            }
            rgba.extend_from_slice(&bytes[..pixels * 4]);
        }
        spa::param::video::VideoFormat::RGBx | spa::param::video::VideoFormat::BGRx => {
            if bytes.len() < pixels * 4 {
                return None;
            }
            for chunk in bytes[..pixels * 4].chunks_exact(4) {
                if matches!(format, spa::param::video::VideoFormat::BGRx) {
                    rgba.extend_from_slice(&[chunk[2], chunk[1], chunk[0], 255]);
                } else {
                    rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
                }
            }
        }
        spa::param::video::VideoFormat::RGB => {
            if bytes.len() < pixels * 3 {
                return None;
            }
            for chunk in bytes[..pixels * 3].chunks_exact(3) {
                rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
        }
        _ => return None,
    }
    Some(rgba)
}

fn start_pipewire_stream(
    node_id: u32,
    fd: OwnedFd,
    tx: SyncSender<CapturedFrame>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopBox::new(None)?;
    let context = pw::context::ContextBox::new(mainloop.loop_(), None)?;
    let core = context.connect_fd(fd, None)?;
    let data = PipeWireUserData {
        format: Default::default(),
        tx,
        sequence: 0,
    };

    let stream = pw::stream::StreamBox::new(
        &core,
        "pacord-screen-capture",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;

    let _listener = stream
        .add_local_listener_with_user_data(data)
        .param_changed(|_, user_data, id, param| {
            let Some(param) = param else {
                return;
            };
            if id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Ok((media_type, media_subtype)) = pw::spa::param::format_utils::parse_format(param)
            else {
                return;
            };
            if media_type != pw::spa::param::format::MediaType::Video
                || media_subtype != pw::spa::param::format::MediaSubtype::Raw
            {
                return;
            }
            let _ = user_data.format.parse(param);
        })
        .process(|stream, user_data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let size = data.chunk().size() as usize;
            let Some(bytes) = data.data() else {
                return;
            };
            let bytes = &bytes[..size.min(bytes.len())];
            let format = user_data.format.format();
            let size = user_data.format.size();
            let Some(rgba) = pipewire_format_to_rgba(bytes, format, size.width, size.height) else {
                return;
            };
            user_data.sequence = user_data.sequence.wrapping_add(1);
            let _ = user_data.tx.try_send(CapturedFrame {
                sequence: user_data.sequence,
                captured_at_micros: timestamp_micros(),
                width: size.width,
                height: size.height,
                rgba,
                backend: CaptureBackendKind::WaylandPipeWire,
            });
        })
        .register()?;

    let obj = pw::spa::pod::object!(
        pw::spa::utils::SpaTypes::ObjectParamFormat,
        pw::spa::param::ParamType::EnumFormat,
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::MediaType,
            Id,
            pw::spa::param::format::MediaType::Video
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::MediaSubtype,
            Id,
            pw::spa::param::format::MediaSubtype::Raw
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            pw::spa::param::video::VideoFormat::RGBA,
            pw::spa::param::video::VideoFormat::RGBx,
            pw::spa::param::video::VideoFormat::BGRx,
            pw::spa::param::video::VideoFormat::RGB
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            pw::spa::utils::Rectangle {
                width: DEFAULT_MAX_WIDTH,
                height: DEFAULT_MAX_HEIGHT
            },
            pw::spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            pw::spa::utils::Rectangle {
                width: 3840,
                height: 2160
            }
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            pw::spa::utils::Fraction {
                num: DEFAULT_FPS,
                denom: 1
            },
            pw::spa::utils::Fraction { num: 1, denom: 1 },
            pw::spa::utils::Fraction { num: 60, denom: 1 }
        )
    );
    let values = pw::spa::pod::serialize::PodSerializer::serialize(
        Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(obj),
    )?
    .0
    .into_inner();
    let mut params = [spa::pod::Pod::from_bytes(&values)
        .ok_or("não foi possível serializar o formato PipeWire")?];
    stream.connect(
        spa::utils::Direction::Input,
        Some(node_id),
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;
    mainloop.run();
    Ok(())
}

pub async fn run_wayland_capture(
    tx: SyncSender<CapturedFrame>,
) -> Result<PortalStreamInfo, Box<dyn Error + Send + Sync>> {
    let proxy = Screencast::new().await?;
    let session = proxy.create_session(Default::default()).await?;
    let cursor_mode = proxy
        .available_cursor_modes()
        .await
        .ok()
        .and_then(|modes| {
            if modes.contains(CursorMode::Embedded) {
                Some(CursorMode::Embedded)
            } else {
                Some(CursorMode::Hidden)
            }
        })
        .unwrap_or(CursorMode::Hidden);

    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(cursor_mode)
                .set_sources(SourceType::Monitor | SourceType::Window)
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await?;
    let response = proxy
        .start(&session, None, Default::default())
        .await?
        .response()?;
    let stream = response
        .streams()
        .first()
        .ok_or("o portal não retornou uma fonte de captura")?;
    let info = PortalStreamInfo {
        node_id: stream.pipe_wire_node_id(),
        width: stream.size().map(|size| size.0 as u32),
        height: stream.size().map(|size| size.1 as u32),
    };
    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await?;
    let node_id = info.node_id;
    tokio::task::spawn_blocking(move || start_pipewire_stream(node_id, fd, tx)).await??;
    Ok(info)
}
