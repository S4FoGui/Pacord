use crate::input::{InputManager, InputOverlay, InputPermissions};
use eframe::egui;
use std::sync::Arc;
use std::thread;

pub fn spawn_host_windows(input_manager: Arc<InputManager>) {
    let overlay_manager = input_manager.clone();
    thread::spawn(move || {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_decorations(false)
                .with_transparent(true)
                .with_always_on_top()
                .with_fullscreen(true)
                .with_mouse_passthrough(true),
            ..Default::default()
        };
        let _ = eframe::run_native(
            "PACORD — indicadores",
            options,
            Box::new(|_cc| Ok(Box::new(HostOverlayApp::new(overlay_manager)))),
        );
    });

    thread::spawn(move || {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([380.0, 300.0]),
            ..Default::default()
        };
        let _ = eframe::run_native(
            "PACORD — controle do host",
            options,
            Box::new(|_cc| Ok(Box::new(HostControlApp::new(input_manager)))),
        );
    });
}

struct HostOverlayApp {
    input_manager: Arc<InputManager>,
}

impl HostOverlayApp {
    fn new(input_manager: Arc<InputManager>) -> Self {
        Self { input_manager }
    }
}

impl eframe::App for HostOverlayApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|input| input.key_pressed(egui::Key::F12)) {
            self.input_manager.revoke_all();
        }
        let screen = ctx.screen_rect();
        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = egui::Color32::TRANSPARENT;
        visuals.panel_fill = egui::Color32::TRANSPARENT;
        visuals.override_text_color = Some(egui::Color32::WHITE);
        ctx.set_visuals(visuals);

        for overlay in self.input_manager.overlays() {
            draw_overlay(ctx, screen, &overlay);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

fn draw_overlay(ctx: &egui::Context, screen: egui::Rect, overlay: &InputOverlay) {
    let cursor = egui::pos2(
        screen.left() + overlay.x.clamp(0.0, 1.0) * screen.width(),
        screen.top() + overlay.y.clamp(0.0, 1.0) * screen.height(),
    );
    let label = if overlay.controller_active {
        format!("{}  [PAD]", overlay.nickname)
    } else {
        overlay.nickname.clone()
    };
    egui::Area::new(egui::Id::new(("pacord-host-cursor", overlay.client_id)))
        .fixed_pos(cursor + egui::vec2(10.0, 10.0))
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
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new(("pacord-host-pointer", overlay.client_id)),
    ));
    painter.rect_filled(
        egui::Rect::from_min_size(cursor, egui::vec2(10.0, 10.0)),
        0.0,
        egui::Color32::WHITE,
    );
}

struct HostControlApp {
    input_manager: Arc<InputManager>,
    status: String,
}

impl HostControlApp {
    fn new(input_manager: Arc<InputManager>) -> Self {
        Self {
            input_manager,
            status: "Entrada remota pronta".into(),
        }
    }
}

impl eframe::App for HostControlApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = egui::Color32::BLACK;
        visuals.panel_fill = egui::Color32::BLACK;
        visuals.override_text_color = Some(egui::Color32::WHITE);
        ctx.set_visuals(visuals);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("PACORD — controle do host");
            ui.label("F12 também revoga todas as entradas remotas.");
            ui.separator();
            let mut permissions = self.input_manager.permissions();
            let before = permissions;
            ui.checkbox(&mut permissions.keyboard, "Permitir teclado");
            ui.checkbox(&mut permissions.pointer, "Permitir mouse");
            ui.checkbox(&mut permissions.controller, "Permitir controle [PAD]");
            if permissions != before {
                self.input_manager.set_permissions(permissions);
            }
            ui.separator();
            if ui.button("REVOGAR TODAS AS ENTRADAS").clicked() {
                let removed = self.input_manager.revoke_all();
                self.status = format!("{removed} sessão(ões) revogada(s)");
            }
            if ui.button("Reativar novas sessões").clicked() {
                self.input_manager.reset_emergency_stop();
                self.status = "Novas sessões podem ser registradas".into();
            }
            ui.label(&self.status);
            ui.separator();
            ui.label(format!(
                "Participantes ativos: {} / 8",
                self.input_manager.overlays().len()
            ));
            for overlay in self.input_manager.overlays() {
                ui.label(format!(
                    "{} — cursor {:.0}%/{:.0}%{}",
                    overlay.nickname,
                    overlay.x * 100.0,
                    overlay.y * 100.0,
                    if overlay.controller_active {
                        " [PAD]"
                    } else {
                        ""
                    }
                ));
            }
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

#[allow(dead_code)]
fn _permissions_label(permissions: InputPermissions) -> String {
    format!(
        "K:{} M:{} P:{}",
        permissions.keyboard, permissions.pointer, permissions.controller
    )
}
