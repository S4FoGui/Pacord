use eframe::egui;
use pacord::profile::UserProfile;
use pacord::rooms::{ParticipantState, RoomManager, RoomMode};
use pacord::session::SessionController;
use pacord::zerotier::{ZeroTierClient, ZeroTierSnapshot};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    Overview,
    Host,
    Join,
    Customize,
}

struct PacordApp {
    profile: UserProfile,
    panel: Panel,
    room_manager: RoomManager,
    session_controller: SessionController,
    zerotier_client: ZeroTierClient,
    zerotier_snapshot: Option<ZeroTierSnapshot>,
    zerotier_message: String,
    room_name: String,
    nickname: String,
    host_address: String,
    port: u16,
    network_id: String,
    backend: String,
    participant_nickname: String,
    participant_address: String,
    join_invite: String,
    status: String,
    last_zerotier_probe: Option<Instant>,
}

impl Default for PacordApp {
    fn default() -> Self {
        let profile = UserProfile::load_from_file("pacord_profile.toml").unwrap_or_default();
        let backend = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            "wayland"
        } else {
            "x11"
        };
        Self {
            nickname: profile.nickname.clone(),
            profile,
            panel: Panel::Overview,
            room_manager: RoomManager::default(),
            session_controller: SessionController::default(),
            zerotier_client: ZeroTierClient::default(),
            zerotier_snapshot: None,
            zerotier_message: "ZeroTier ainda não foi consultado".into(),
            room_name: "Sala PACORD".into(),
            host_address: String::new(),
            port: 7777,
            network_id: String::new(),
            backend: backend.into(),
            participant_nickname: String::new(),
            participant_address: String::new(),
            join_invite: String::new(),
            status: "Pronto. Nenhuma sala foi aberta.".into(),
            last_zerotier_probe: None,
        }
    }
}

impl PacordApp {
    fn refresh_zerotier(&mut self) {
        match self.zerotier_client.inspect() {
            Ok(snapshot) => {
                if self.network_id.is_empty() {
                    if let Some(network) = snapshot.networks.first() {
                        self.network_id = network.id.clone();
                    }
                }
                if self.host_address.is_empty() {
                    if let Some(address) = snapshot.first_ipv4() {
                        self.host_address = address;
                    }
                }
                self.zerotier_message = format!(
                    "Nó {} — {} — {} rede(s)",
                    snapshot.node.address,
                    snapshot.node.status,
                    snapshot.networks.len()
                );
                self.zerotier_snapshot = Some(snapshot);
                self.status = "Diagnóstico ZeroTier atualizado".into();
            }
            Err(error) => {
                self.zerotier_message = error.to_string();
                self.status = "Não foi possível consultar o ZeroTier".into();
            }
        }
        self.last_zerotier_probe = Some(Instant::now());
    }

    fn join_zerotier(&mut self) {
        match self.zerotier_client.join(self.network_id.trim()) {
            Ok(message) => {
                self.status = format!("ZeroTier: {message}");
                self.refresh_zerotier();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn leave_zerotier(&mut self) {
        match self.zerotier_client.leave(self.network_id.trim()) {
            Ok(message) => {
                self.status = format!("ZeroTier: {message}");
                self.refresh_zerotier();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn create_room(&mut self) {
        let result = self.room_manager.create_host_room(
            self.room_name.trim().to_string(),
            self.nickname.trim().to_string(),
            self.host_address.trim().to_string(),
            self.port,
            self.network_id.trim().to_string(),
        );
        match result {
            Ok(room) => {
                self.status = format!(
                    "Sala '{}' criada — código {}",
                    room.invite.room_name, room.invite.room_code
                );
                self.panel = Panel::Host;
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn join_room(&mut self) {
        match pacord::rooms::RoomInvite::decode(&self.join_invite) {
            Ok(invite) => match self
                .room_manager
                .join_room(invite, self.nickname.trim().to_string())
            {
                Ok(room) => {
                    self.status = format!(
                        "Convite aceito para '{}' — aguardando aprovação do host",
                        room.invite.room_name
                    );
                    self.panel = Panel::Join;
                }
                Err(error) => self.status = error.to_string(),
            },
            Err(error) => self.status = error.to_string(),
        }
    }

    fn start_room_session(&mut self) {
        let Some(room) = self.room_manager.current().cloned() else {
            self.status = "Abra ou entre em uma sala antes de iniciar uma sessão".into();
            return;
        };
        if room.mode == RoomMode::Host {
            if let Err(error) = self.room_manager.sync_admission_file() {
                self.status = error.to_string();
                return;
            }
        }
        let result = match room.mode {
            RoomMode::Host => self.session_controller.start_host(&room, &self.backend),
            RoomMode::Client => self.session_controller.start_viewer(&room),
        };
        match result {
            Ok(()) => self.status = "Processo de sessão iniciado".into(),
            Err(error) => self.status = error.to_string(),
        }
    }

    fn stop_room_session(&mut self) {
        match self.session_controller.stop() {
            Ok(()) => self.status = "Sessão encerrada".into(),
            Err(error) => self.status = error.to_string(),
        }
    }

    fn close_room(&mut self) {
        if let Err(error) = self.session_controller.stop() {
            self.status = error.to_string();
            return;
        }
        match self.room_manager.close_room() {
            Ok(()) => self.status = "Sala encerrada; novas conexões foram interrompidas".into(),
            Err(error) => self.status = error.to_string(),
        }
    }

    fn draw_header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("pacord_header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("PACORD");
                ui.label("salas colaborativas");
                ui.separator();
                ui.label(format!("Sessão: {}", self.session_controller.state()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Sair da sessão").clicked() && self.session_controller.is_running()
                    {
                        self.stop_room_session();
                    }
                });
            });
        });
    }

    fn draw_navigation(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("pacord_navigation")
            .resizable(false)
            .default_width(170.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.label("NAVEGAÇÃO");
                ui.separator();
                nav_button(ui, &mut self.panel, Panel::Overview, "Visão geral");
                nav_button(ui, &mut self.panel, Panel::Host, "Criar sala");
                nav_button(ui, &mut self.panel, Panel::Join, "Entrar em sala");
                nav_button(ui, &mut self.panel, Panel::Customize, "Personalização");
                ui.add_space(18.0);
                ui.label("ZERO TIER");
                ui.separator();
                if ui.button("Atualizar diagnóstico").clicked() {
                    self.refresh_zerotier();
                }
                ui.label(&self.zerotier_message);
                ui.add_space(12.0);
                ui.label("PACORD 4.0");
                ui.label("KDE Plasma / preto e branco");
            });
    }

    fn draw_content(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            match self.panel {
                Panel::Overview => self.draw_overview(ui),
                Panel::Host => self.draw_host(ui),
                Panel::Join => self.draw_join(ui),
                Panel::Customize => self.draw_customize(ui),
            }
            ui.add_space(12.0);
            ui.separator();
            ui.label(format!("STATUS: {}", self.status));
        });
    }

    fn draw_overview(&mut self, ui: &mut egui::Ui) {
        ui.heading("Visão geral");
        ui.label("Gerencie salas PACORD, conectividade ZeroTier e participantes autorizados.");
        ui.add_space(8.0);
        ui.columns(2, |columns| {
            columns[0].group(|ui| {
                ui.heading("Sala atual");
                if let Some(room) = self.room_manager.current().cloned() {
                    ui.label(format!("Nome: {}", room.invite.room_name));
                    ui.label(format!("Modo: {:?}", room.mode));
                    ui.label(format!("Estado: {}", room.lifecycle));
                    ui.label(format!("Participantes: {} / 8", room.participant_count()));
                    ui.label(format!("Rede: {}", room.invite.network_id));
                    ui.label(format!(
                        "Endpoint: {}:{}",
                        room.invite.host_address, room.invite.port
                    ));
                    if ui.button("Abrir painel da sala").clicked() {
                        self.panel = if room.mode == RoomMode::Host {
                            Panel::Host
                        } else {
                            Panel::Join
                        };
                    }
                } else {
                    ui.label("Nenhuma sala aberta.");
                    if ui.button("Criar uma sala").clicked() {
                        self.panel = Panel::Host;
                    }
                    if ui.button("Entrar com convite").clicked() {
                        self.panel = Panel::Join;
                    }
                }
            });
            columns[1].group(|ui| {
                ui.heading("ZeroTier");
                if let Some(snapshot) = &self.zerotier_snapshot {
                    ui.label(format!("Nó: {}", snapshot.node.address));
                    ui.label(format!("Estado: {}", snapshot.node.status));
                    ui.label(format!("Versão: {}", snapshot.node.version));
                    ui.label(format!("Redes conectadas: {}", snapshot.networks.len()));
                    for network in &snapshot.networks {
                        ui.label(format!(
                            "{} — {} — {}",
                            network.id, network.status, network.device
                        ));
                    }
                } else {
                    ui.label("Clique em Atualizar diagnóstico para consultar o serviço local.");
                }
            });
        });
    }

    fn draw_host(&mut self, ui: &mut egui::Ui) {
        ui.heading("Criar e administrar sala");
        ui.label("A criação gera um código e um segredo exclusivos. Compartilhe o convite apenas com pessoas autorizadas.");
        ui.add_space(8.0);
        ui.columns(2, |columns| {
            columns[0].group(|ui| {
                ui.heading("Nova sala");
                ui.label("Nome da sala");
                ui.text_edit_singleline(&mut self.room_name);
                ui.label("Seu nickname");
                ui.text_edit_singleline(&mut self.nickname);
                ui.label("IP ZeroTier do host");
                ui.text_edit_singleline(&mut self.host_address);
                ui.horizontal(|ui| {
                    ui.label("Porta");
                    ui.add(egui::DragValue::new(&mut self.port).range(1..=65535));
                });
                ui.label("ID da rede ZeroTier");
                ui.text_edit_singleline(&mut self.network_id);
                ui.label("Backend de captura");
                egui::ComboBox::from_id_source("backend")
                    .selected_text(&self.backend)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.backend,
                            "wayland".into(),
                            "Wayland / PipeWire",
                        );
                        ui.selectable_value(&mut self.backend, "x11".into(), "X11 / XShm");
                    });
                if ui.button("Criar sala host").clicked() {
                    self.create_room();
                }
            });
            columns[1].group(|ui| {
                ui.heading("Rede ZeroTier");
                ui.label("As ações abaixo são administrativas e só executam após clique.");
                if ui.button("Entrar na rede informada").clicked() {
                    self.join_zerotier();
                }
                if ui.button("Sair da rede informada").clicked() {
                    self.leave_zerotier();
                }
                ui.separator();
                if let Some(room) = self.room_manager.current().cloned() {
                    ui.label(format!("Sala atual: {}", room.invite.room_name));
                    ui.label(format!("Código: {}", room.invite.room_code));
                    ui.label("Convite completo (copie por um canal confiável):");
                    let mut invite = room
                        .invite_text()
                        .unwrap_or_else(|_| "convite indisponível".into());
                    ui.add(
                        egui::TextEdit::multiline(&mut invite)
                            .desired_rows(5)
                            .interactive(false),
                    );
                    if ui.button("Copiar convite").clicked() {
                        ui.output_mut(|output| output.copied_text = invite.clone());
                        self.status = "Convite copiado para a área de transferência".into();
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Iniciar host").clicked() {
                            self.start_room_session();
                        }
                        if ui.button("Encerrar sala").clicked() {
                            self.close_room();
                        }
                    });
                }
            });
        });
        self.draw_participants(ui);
    }

    fn draw_participants(&mut self, ui: &mut egui::Ui) {
        let Some(room) = self.room_manager.current().cloned() else {
            return;
        };
        if room.mode != RoomMode::Host {
            return;
        }
        ui.add_space(10.0);
        ui.heading(format!("Participantes — {} / 8", room.participant_count()));
        ui.horizontal(|ui| {
            ui.label("Nickname");
            ui.text_edit_singleline(&mut self.participant_nickname);
            ui.label("IP:porta");
            ui.text_edit_singleline(&mut self.participant_address);
            if ui.button("Adicionar pendente").clicked() {
                let nickname = std::mem::take(&mut self.participant_nickname);
                let address = std::mem::take(&mut self.participant_address);
                match self.room_manager.add_participant(nickname, address) {
                    Ok(_) => self.status = "Participante adicionado como pendente".into(),
                    Err(error) => self.status = error.to_string(),
                }
            }
        });
        let participants = self
            .room_manager
            .current()
            .map(|room| room.participants.clone())
            .unwrap_or_default();
        for participant in participants {
            let mut permissions = participant.permissions;
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "#{} {} — {} — {}",
                        participant.id,
                        participant.nickname,
                        participant.address,
                        participant.state
                    ));
                    if participant.state == ParticipantState::Pending
                        && ui.button("Aprovar").clicked()
                    {
                        let _ = self
                            .room_manager
                            .set_participant_state(participant.id, ParticipantState::Approved);
                    }
                    if participant.state != ParticipantState::Rejected
                        && ui.button("Recusar").clicked()
                    {
                        let _ = self
                            .room_manager
                            .set_participant_state(participant.id, ParticipantState::Rejected);
                    }
                    if ui.button("Remover").clicked() {
                        let _ = self.room_manager.remove_participant(participant.id);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Permissões:");
                    ui.checkbox(&mut permissions.keyboard, "teclado");
                    ui.checkbox(&mut permissions.pointer, "mouse");
                    ui.checkbox(&mut permissions.controller, "controle [PAD]");
                });
            });
            if permissions != participant.permissions {
                let _ = self
                    .room_manager
                    .set_participant_permissions(participant.id, permissions);
            }
        }
    }

    fn draw_join(&mut self, ui: &mut egui::Ui) {
        ui.heading("Entrar em uma sala");
        ui.label("Cole o convite TOML recebido do host. O PACORD não descobre salas fora da rede ZeroTier.");
        ui.label("Seu nickname");
        ui.text_edit_singleline(&mut self.nickname);
        ui.label("Convite");
        ui.add(egui::TextEdit::multiline(&mut self.join_invite).desired_rows(8));
        ui.horizontal(|ui| {
            if ui.button("Validar e entrar").clicked() {
                self.join_room();
            }
            if self
                .room_manager
                .current()
                .map(|room| room.mode == RoomMode::Client)
                .unwrap_or(false)
                && ui.button("Iniciar viewer").clicked()
            {
                self.start_room_session();
            }
            if ui.button("Limpar").clicked() {
                self.join_invite.clear();
                self.room_manager.clear();
            }
        });
        if let Some(room) = self.room_manager.current().cloned() {
            if room.mode == RoomMode::Client {
                ui.separator();
                ui.label(format!("Sala: {}", room.invite.room_name));
                ui.label(format!(
                    "Endpoint: {}:{}",
                    room.invite.host_address, room.invite.port
                ));
                ui.label(format!("Estado: {}", room.lifecycle));
                ui.label("A aprovação e as permissões são controladas pelo host.");
                if self.session_controller.is_running() && ui.button("Encerrar viewer").clicked() {
                    self.stop_room_session();
                }
            }
        }
    }

    fn draw_customize(&mut self, ui: &mut egui::Ui) {
        ui.heading("Personalização");
        ui.label("As cores da interface são fixadas em preto e branco para manter a identidade do PACORD.");
        ui.label("Nickname padrão");
        ui.text_edit_singleline(&mut self.profile.nickname);
        ui.add(
            egui::Slider::new(&mut self.profile.mouse.sensitivity, 0.1..=5.0)
                .text("Sensibilidade do mouse"),
        );
        ui.add(
            egui::Slider::new(&mut self.profile.controller.deadzone, 0.01..=0.5)
                .text("Zona morta do controle"),
        );
        ui.checkbox(
            &mut self.profile.controller.invert_y,
            "Inverter eixo Y do controle",
        );
        if ui.button("Salvar perfil").clicked() {
            match self.profile.save_to_file("pacord_profile.toml") {
                Ok(()) => self.status = "Perfil salvo".into(),
                Err(error) => self.status = format!("Falha ao salvar perfil: {error}"),
            }
        }
    }
}

fn nav_button(ui: &mut egui::Ui, current: &mut Panel, target: Panel, label: &str) {
    let selected = *current == target;
    if ui.selectable_label(selected, label).clicked() {
        *current = target;
    }
}

impl eframe::App for PacordApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.session_controller.poll();
        if self
            .last_zerotier_probe
            .map(|time| time.elapsed() > Duration::from_secs(60))
            .unwrap_or(false)
        {
            self.refresh_zerotier();
        }
        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = egui::Color32::BLACK;
        visuals.panel_fill = egui::Color32::BLACK;
        visuals.extreme_bg_color = egui::Color32::BLACK;
        visuals.override_text_color = Some(egui::Color32::WHITE);
        visuals.widgets.noninteractive.bg_fill = egui::Color32::BLACK;
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);
        visuals.widgets.inactive.bg_fill = egui::Color32::BLACK;
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);
        visuals.widgets.hovered.bg_fill = egui::Color32::from_gray(40);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);
        visuals.widgets.active.bg_fill = egui::Color32::from_gray(80);
        visuals.widgets.active.fg_stroke = egui::Stroke::new(2.0_f32, egui::Color32::WHITE);
        ctx.set_visuals(visuals);
        self.draw_header(ctx);
        self.draw_navigation(ctx);
        self.draw_content(ctx);
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

fn main() -> Result<(), eframe::Error> {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "PACORD — salas colaborativas",
        options,
        Box::new(|_cc| Ok(Box::new(PacordApp::default()))),
    )
}
