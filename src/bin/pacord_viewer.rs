use eframe::egui;
use gilrs::{Axis, Button, EventType, Gilrs};
use pacord::input::{GamepadAxis, InputEventPacket, InputPermissions};
use pacord::transport::{
    connect_input_client, InputClientHandle, InputOverlayPacket, ServerEvent, TransportError,
};
use std::env;
use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver};
use std::thread;

#[derive(Debug)]
enum ViewerMessage {
    Connected(InputClientHandle),
    Event(Result<ServerEvent, TransportError>),
    Error(String),
}

struct ViewerApp {
    messages: Receiver<ViewerMessage>,
    input_handle: Option<InputClientHandle>,
    texture: Option<egui::TextureHandle>,
    overlays: Vec<InputOverlayPacket>,
    permissions: InputPermissions,
    status: String,
    last_sequence: u64,
    control_enabled: bool,
    gilrs: Option<Gilrs>,
}

impl ViewerApp {
    fn new(messages: Receiver<ViewerMessage>) -> Self {
        Self {
            messages,
            input_handle: None,
            texture: None,
            overlays: Vec::new(),
            permissions: InputPermissions::none(),
            status: "Aguardando autorização do host…".to_string(),
            last_sequence: 0,
            control_enabled: false,
            gilrs: Gilrs::new().ok(),
        }
    }

    fn send(&self, event: InputEventPacket) {
        if !self.control_enabled {
            return;
        }
        if let Some(handle) = &self.input_handle {
            if let Err(error) = handle.try_send_input(event) {
                log::debug!("evento local não enviado: {error}");
            }
        }
    }

    fn drain_network(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.messages.try_recv() {
            match message {
                ViewerMessage::Connected(handle) => {
                    self.input_handle = Some(handle);
                    self.status = "Conectado; aguardando permissões do host".into();
                }
                ViewerMessage::Event(Ok(ServerEvent::Frame(packet))) => {
                    match image::load_from_memory(&packet.jpeg) {
                        Ok(image) => {
                            let rgba = image.to_rgba8();
                            let size = [rgba.width() as usize, rgba.height() as usize];
                            let color_image =
                                egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                            self.texture = Some(ctx.load_texture(
                                "pacord-remote-screen",
                                color_image,
                                egui::TextureOptions::LINEAR,
                            ));
                            self.last_sequence = packet.sequence;
                            self.status = format!(
                                "Recebendo {}×{} — frame {}",
                                packet.width, packet.height, packet.sequence
                            );
                        }
                        Err(error) => self.status = format!("Frame inválido: {error}"),
                    }
                }
                ViewerMessage::Event(Ok(ServerEvent::InputPolicy(permissions))) => {
                    self.permissions = permissions;
                    self.status = format!(
                        "Permissões: teclado={}, mouse={}, controle={}",
                        permissions.keyboard, permissions.pointer, permissions.controller
                    );
                }
                ViewerMessage::Event(Ok(ServerEvent::Overlays(overlays))) => {
                    self.overlays = overlays
                }
                ViewerMessage::Event(Ok(ServerEvent::InputAccepted)) => {
                    self.status = "Evento de entrada aceito".into()
                }
                ViewerMessage::Event(Ok(ServerEvent::InputRejected(reason))) => {
                    self.status = format!("Entrada rejeitada: {reason}")
                }
                ViewerMessage::Event(Ok(ServerEvent::Stopped)) => {
                    self.status = "Host encerrou a sessão".into()
                }
                ViewerMessage::Event(Err(error)) => {
                    self.status = format!("Transporte: {error}");
                }
                ViewerMessage::Error(error) => {
                    self.status = format!("Transporte: {error}");
                }
            }
        }
    }

    fn poll_gamepad(&mut self) {
        if !self.control_enabled || !self.permissions.controller {
            return;
        }
        let Some(gilrs) = self.gilrs.as_mut() else {
            return;
        };
        let mut pending = Vec::new();
        while let Some(event) = gilrs.next_event() {
            match event.event {
                EventType::ButtonPressed(button, _) => {
                    if let Some(code) = linux_gamepad_button(button) {
                        pending.push(InputEventPacket::GamepadButton { code, value: 1 });
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    if let Some(code) = linux_gamepad_button(button) {
                        pending.push(InputEventPacket::GamepadButton { code, value: 0 });
                    }
                }
                EventType::AxisChanged(axis, value, _) => {
                    if let Some(axis) = gamepad_axis(axis) {
                        let scaled = (value.clamp(-1.0, 1.0) * 32767.0) as i32;
                        pending.push(InputEventPacket::GamepadAxis {
                            axis,
                            value: scaled,
                        });
                    }
                }
                _ => {}
            }
        }
        for event in pending {
            self.send(event);
        }
    }

    fn poll_keyboard_and_buttons(&self, ctx: &egui::Context, image_rect: egui::Rect) {
        if !self.control_enabled {
            return;
        }
        let events = ctx.input(|input| input.events.clone());
        for event in events {
            match event {
                egui::Event::Key { key, pressed, .. } if self.permissions.keyboard => {
                    if let Some(code) = linux_key_code(key) {
                        self.send(InputEventPacket::Key {
                            code,
                            value: i32::from(pressed),
                        });
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } if self.permissions.pointer && image_rect.contains(pos) => {
                    if let Some(code) = linux_pointer_button(button) {
                        self.send(InputEventPacket::PointerButton {
                            code,
                            value: i32::from(pressed),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn draw_overlays(&self, ctx: &egui::Context, image_rect: egui::Rect) {
        for overlay in &self.overlays {
            let pos = egui::pos2(
                image_rect.left() + overlay.x.clamp(0.0, 1.0) * image_rect.width(),
                image_rect.top() + overlay.y.clamp(0.0, 1.0) * image_rect.height(),
            );
            let label = if overlay.controller_active {
                format!("{}  [PAD]", overlay.nickname)
            } else {
                overlay.nickname.clone()
            };
            egui::Area::new(egui::Id::new(("pacord-cursor", overlay.client_id)))
                .fixed_pos(pos + egui::vec2(10.0, 8.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(egui::Color32::BLACK)
                        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::WHITE))
                        .inner_margin(egui::Margin::symmetric(6.0, 3.0))
                        .show(ui, |ui| {
                            ui.colored_label(egui::Color32::WHITE, label);
                        });
                });
        }
    }
}

impl eframe::App for ViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_network(ctx);
        self.poll_gamepad();

        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = egui::Color32::BLACK;
        visuals.panel_fill = egui::Color32::BLACK;
        visuals.override_text_color = Some(egui::Color32::WHITE);
        ctx.set_visuals(visuals);

        egui::TopBottomPanel::top("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("PACORD — janela remota");
                ui.separator();
                ui.label(&self.status);
                ui.separator();
                ui.checkbox(&mut self.control_enabled, "Enviar entrada");
                if !self.permissions.keyboard
                    && !self.permissions.pointer
                    && !self.permissions.controller
                {
                    ui.label("Host não concedeu entrada");
                }
            });
        });

        let mut image_rect = egui::Rect::NOTHING;
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(texture) = &self.texture {
                let available = ui.available_size();
                let size = texture.size_vec2();
                let scale = (available.x / size.x).min(available.y / size.y).min(1.0);
                let display_size = size * scale;
                let response = ui.add(
                    egui::Image::new((texture.id(), display_size))
                        .sense(egui::Sense::click_and_drag()),
                );
                image_rect = response.rect;
                if self.control_enabled && self.permissions.pointer && response.hovered() {
                    let pointer_delta = ctx.input(|input| input.pointer.delta());
                    if pointer_delta != egui::Vec2::ZERO {
                        self.send(InputEventPacket::PointerMotion {
                            dx: pointer_delta.x.round() as i32,
                            dy: pointer_delta.y.round() as i32,
                        });
                    }
                    if let Some(pos) = response.hover_pos() {
                        let x = (pos.x - response.rect.left()) / response.rect.width();
                        let y = (pos.y - response.rect.top()) / response.rect.height();
                        self.send(InputEventPacket::PointerPosition { x, y });
                    }
                    let scroll = ctx.input(|input| input.raw_scroll_delta.y);
                    if scroll.abs() > 0.1 {
                        self.send(InputEventPacket::PointerWheel {
                            value: scroll.round() as i32,
                        });
                    }
                }
                self.poll_keyboard_and_buttons(ctx, response.rect);
            } else {
                ui.centered_and_justified(|ui| ui.label("O host ainda não enviou um frame."));
            }
        });
        if image_rect != egui::Rect::NOTHING {
            self.draw_overlays(ctx, image_rect);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

fn linux_pointer_button(button: egui::PointerButton) -> Option<u16> {
    Some(match button {
        egui::PointerButton::Primary => 272,
        egui::PointerButton::Secondary => 273,
        egui::PointerButton::Middle => 274,
        egui::PointerButton::Extra1 => 275,
        egui::PointerButton::Extra2 => 276,
    })
}

fn linux_key_code(key: egui::Key) -> Option<u16> {
    Some(match key {
        egui::Key::A => 30,
        egui::Key::B => 48,
        egui::Key::C => 46,
        egui::Key::D => 32,
        egui::Key::E => 18,
        egui::Key::F => 33,
        egui::Key::G => 34,
        egui::Key::H => 35,
        egui::Key::I => 23,
        egui::Key::J => 36,
        egui::Key::K => 37,
        egui::Key::L => 38,
        egui::Key::M => 50,
        egui::Key::N => 49,
        egui::Key::O => 24,
        egui::Key::P => 25,
        egui::Key::Q => 16,
        egui::Key::R => 19,
        egui::Key::S => 31,
        egui::Key::T => 20,
        egui::Key::U => 22,
        egui::Key::V => 47,
        egui::Key::W => 17,
        egui::Key::X => 45,
        egui::Key::Y => 21,
        egui::Key::Z => 44,
        egui::Key::Num0 => 11,
        egui::Key::Num1 => 2,
        egui::Key::Num2 => 3,
        egui::Key::Num3 => 4,
        egui::Key::Num4 => 5,
        egui::Key::Num5 => 6,
        egui::Key::Num6 => 7,
        egui::Key::Num7 => 8,
        egui::Key::Num8 => 9,
        egui::Key::Num9 => 10,
        egui::Key::Escape => 1,
        egui::Key::Tab => 15,
        egui::Key::Backspace => 14,
        egui::Key::Enter => 28,
        egui::Key::Space => 57,
        egui::Key::ArrowUp => 103,
        egui::Key::ArrowDown => 108,
        egui::Key::ArrowLeft => 105,
        egui::Key::ArrowRight => 106,
        egui::Key::Insert => 110,
        egui::Key::Delete => 111,
        egui::Key::Home => 102,
        egui::Key::End => 107,
        egui::Key::PageUp => 104,
        egui::Key::PageDown => 109,
        egui::Key::F1 => 59,
        egui::Key::F2 => 60,
        egui::Key::F3 => 61,
        egui::Key::F4 => 62,
        egui::Key::F5 => 63,
        egui::Key::F6 => 64,
        egui::Key::F7 => 65,
        egui::Key::F8 => 66,
        egui::Key::F9 => 67,
        egui::Key::F10 => 68,
        egui::Key::F11 => 87,
        egui::Key::F12 => 88,
        _ => return None,
    })
}

fn linux_gamepad_button(button: Button) -> Option<u16> {
    Some(match button {
        Button::South => 0x130,
        Button::East => 0x131,
        Button::C => 0x132,
        Button::North => 0x133,
        Button::West => 0x134,
        Button::Z => 0x135,
        Button::LeftTrigger => 0x136,
        Button::RightTrigger => 0x137,
        Button::LeftTrigger2 => 0x138,
        Button::RightTrigger2 => 0x139,
        Button::Select => 0x13a,
        Button::Start => 0x13b,
        Button::Mode => 0x13c,
        Button::LeftThumb => 0x13d,
        Button::RightThumb => 0x13e,
        _ => return None,
    })
}

fn gamepad_axis(axis: Axis) -> Option<GamepadAxis> {
    match axis {
        Axis::LeftStickX => Some(GamepadAxis::X),
        Axis::LeftStickY => Some(GamepadAxis::Y),
        Axis::RightStickX => Some(GamepadAxis::Rx),
        Axis::RightStickY => Some(GamepadAxis::Ry),
        _ => None,
    }
}

fn usage() {
    eprintln!(
        "Uso: PACORD_SECRET='<segredo>' cargo run --bin pacord-viewer -- <host-zerotier-ip:porta> <apelido>\n\nExemplo:\n  PACORD_SECRET='troque-por-um-segredo-longo' cargo run --bin pacord-viewer -- 10.147.20.5:7777 Alice"
    );
}

fn main() -> Result<(), eframe::Error> {
    let mut args = env::args().skip(1);
    let Some(server) = args.next() else {
        usage();
        return Ok(());
    };
    let Some(nickname) = args.next() else {
        usage();
        return Ok(());
    };
    let secret = match env::var("PACORD_SECRET") {
        Ok(value) if value.len() >= 16 => value.into_bytes(),
        _ => {
            eprintln!("PACORD_SECRET deve existir e ter pelo menos 16 bytes.");
            return Ok(());
        }
    };
    let server: SocketAddr = match server.parse() {
        Ok(value) => value,
        Err(error) => {
            eprintln!("endereço inválido: {error}");
            return Ok(());
        }
    };

    let (message_sender, message_receiver) = mpsc::channel();
    thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("runtime Tokio");
        runtime.block_on(async move {
            match connect_input_client(server, nickname, secret).await {
                Ok((handle, mut events)) => {
                    let _ = message_sender.send(ViewerMessage::Connected(handle));
                    while let Some(event) = events.recv().await {
                        if message_sender.send(ViewerMessage::Event(event)).is_err() {
                            break;
                        }
                    }
                }
                Err(error) => {
                    let _ = message_sender.send(ViewerMessage::Error(error.to_string()));
                }
            }
        });
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "PACORD — janela remota",
        options,
        Box::new(|_cc| Ok(Box::new(ViewerApp::new(message_receiver)))),
    )
}
