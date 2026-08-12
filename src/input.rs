use crate::uinput_manager::ClientVirtualDevices;
use evdev::AbsoluteAxisType;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputPermissions {
    pub keyboard: bool,
    pub pointer: bool,
    pub controller: bool,
}

impl InputPermissions {
    pub const fn none() -> Self {
        Self {
            keyboard: false,
            pointer: false,
            controller: false,
        }
    }

    pub const fn all() -> Self {
        Self {
            keyboard: true,
            pointer: true,
            controller: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum GamepadAxis {
    X,
    Y,
    Rx,
    Ry,
}

impl GamepadAxis {
    fn evdev(self) -> AbsoluteAxisType {
        match self {
            Self::X => AbsoluteAxisType::ABS_X,
            Self::Y => AbsoluteAxisType::ABS_Y,
            Self::Rx => AbsoluteAxisType::ABS_RX,
            Self::Ry => AbsoluteAxisType::ABS_RY,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InputEventPacket {
    Key { code: u16, value: i32 },
    PointerMotion { dx: i32, dy: i32 },
    PointerPosition { x: f32, y: f32 },
    PointerButton { code: u16, value: i32 },
    PointerWheel { value: i32 },
    GamepadButton { code: u16, value: i32 },
    GamepadAxis { axis: GamepadAxis, value: i32 },
    ControllerPresence { active: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputOverlay {
    pub client_id: usize,
    pub nickname: String,
    pub x: f32,
    pub y: f32,
    pub controller_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputError {
    PermissionDenied(&'static str),
    InvalidEvent(&'static str),
    UnknownClient,
    Device(String),
    EmergencyStop,
}

impl std::fmt::Display for InputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionDenied(kind) => write!(f, "permissão de {kind} desabilitada pelo host"),
            Self::InvalidEvent(reason) => write!(f, "evento inválido: {reason}"),
            Self::UnknownClient => write!(f, "cliente de entrada desconhecido"),
            Self::Device(reason) => write!(f, "dispositivo virtual indisponível: {reason}"),
            Self::EmergencyStop => write!(f, "entrada desabilitada pelo bloqueio de emergência"),
        }
    }
}

impl std::error::Error for InputError {}

struct InputSession {
    client_id: usize,
    nickname: String,
    devices: Option<ClientVirtualDevices>,
    x: f32,
    y: f32,
    controller_active: bool,
}

pub struct InputManager {
    permissions: Mutex<InputPermissions>,
    sessions: Mutex<HashMap<usize, InputSession>>,
    next_client_id: AtomicUsize,
    emergency_stop: std::sync::atomic::AtomicBool,
}

impl InputManager {
    pub fn new(permissions: InputPermissions) -> Self {
        Self {
            permissions: Mutex::new(permissions),
            sessions: Mutex::new(HashMap::new()),
            next_client_id: AtomicUsize::new(1),
            emergency_stop: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn permissions(&self) -> InputPermissions {
        *self.permissions.lock().expect("permissões PACORD")
    }

    pub fn set_permissions(&self, permissions: InputPermissions) {
        *self.permissions.lock().expect("permissões PACORD") = permissions;
        if !permissions.keyboard || !permissions.pointer || !permissions.controller {
            if let Ok(mut sessions) = self.sessions.lock() {
                for session in sessions.values_mut() {
                    if !permissions.controller {
                        session.controller_active = false;
                    }
                }
            }
        }
    }

    pub fn register(&self, nickname: String) -> usize {
        let client_id = self.next_client_id.fetch_add(1, Ordering::Relaxed);
        let mut sessions = self.sessions.lock().expect("sessões PACORD");
        sessions.insert(
            client_id,
            InputSession {
                client_id,
                nickname,
                devices: None,
                x: 0.5,
                y: 0.5,
                controller_active: false,
            },
        );
        client_id
    }

    pub fn disconnect(&self, client_id: usize) {
        self.sessions
            .lock()
            .expect("sessões PACORD")
            .remove(&client_id);
    }

    pub fn revoke_all(&self) -> usize {
        self.emergency_stop.store(true, Ordering::Release);
        let mut sessions = self.sessions.lock().expect("sessões PACORD");
        let count = sessions.len();
        sessions.clear();
        count
    }

    pub fn reset_emergency_stop(&self) {
        self.emergency_stop.store(false, Ordering::Release);
    }

    pub fn handle_event(
        &self,
        client_id: usize,
        event: InputEventPacket,
    ) -> Result<(), InputError> {
        if self.emergency_stop.load(Ordering::Acquire) {
            return Err(InputError::EmergencyStop);
        }
        let permissions = self.permissions();
        let mut sessions = self.sessions.lock().expect("sessões PACORD");
        let session = sessions
            .get_mut(&client_id)
            .ok_or(InputError::UnknownClient)?;

        match &event {
            InputEventPacket::Key { code, value } => {
                if !permissions.keyboard {
                    return Err(InputError::PermissionDenied("teclado"));
                }
                if *code > 767 || !matches!(value, 0 | 1 | 2) {
                    return Err(InputError::InvalidEvent("código/valor de tecla"));
                }
            }
            InputEventPacket::PointerMotion { dx, dy } => {
                if !permissions.pointer {
                    return Err(InputError::PermissionDenied("mouse"));
                }
                if dx.abs() > 4096 || dy.abs() > 4096 {
                    return Err(InputError::InvalidEvent("movimento excessivo"));
                }
                session.x = (session.x + *dx as f32 / 1920.0).clamp(0.0, 1.0);
                session.y = (session.y + *dy as f32 / 1080.0).clamp(0.0, 1.0);
            }
            InputEventPacket::PointerPosition { x, y } => {
                if !permissions.pointer {
                    return Err(InputError::PermissionDenied("mouse"));
                }
                if !(0.0..=1.0).contains(x) || !(0.0..=1.0).contains(y) {
                    return Err(InputError::InvalidEvent("posição fora da área normalizada"));
                }
                session.x = *x;
                session.y = *y;
            }
            InputEventPacket::PointerButton { code, value } => {
                if !permissions.pointer {
                    return Err(InputError::PermissionDenied("mouse"));
                }
                if *code > 0x14a || !matches!(value, 0 | 1 | 2) {
                    return Err(InputError::InvalidEvent("botão/valor de mouse"));
                }
            }
            InputEventPacket::PointerWheel { value } => {
                if !permissions.pointer {
                    return Err(InputError::PermissionDenied("mouse"));
                }
                if value.abs() > 120 {
                    return Err(InputError::InvalidEvent("roda excessiva"));
                }
            }
            InputEventPacket::GamepadButton { code, value } => {
                if !permissions.controller {
                    return Err(InputError::PermissionDenied("controle"));
                }
                if *code > 0x2ff || !matches!(value, 0 | 1 | 2) {
                    return Err(InputError::InvalidEvent("botão/valor de controle"));
                }
                session.controller_active = true;
            }
            InputEventPacket::GamepadAxis { value, .. } => {
                if !permissions.controller {
                    return Err(InputError::PermissionDenied("controle"));
                }
                if !(-32768..=32767).contains(value) {
                    return Err(InputError::InvalidEvent("eixo fora do intervalo"));
                }
                session.controller_active = true;
            }
            InputEventPacket::ControllerPresence { active } => {
                if !permissions.controller {
                    return Err(InputError::PermissionDenied("controle"));
                }
                session.controller_active = *active;
            }
        }

        if session.devices.is_none() {
            session.devices = Some(
                ClientVirtualDevices::new(session.client_id, &session.nickname)
                    .map_err(|error| InputError::Device(error.to_string()))?,
            );
        }
        let devices = session
            .devices
            .as_mut()
            .expect("dispositivos recém-criados");
        match event {
            InputEventPacket::Key { code, value } => devices.send_key_event(code, value),
            InputEventPacket::PointerMotion { dx, dy } => devices.send_mouse_motion(dx, dy),
            InputEventPacket::PointerPosition { .. } => Ok(()),
            InputEventPacket::PointerButton { code, value } => {
                devices.send_mouse_button(code, value)
            }
            InputEventPacket::PointerWheel { value } => devices.send_mouse_wheel(value),
            InputEventPacket::GamepadButton { code, value } => {
                devices.send_gamepad_button(code, value)
            }
            InputEventPacket::GamepadAxis { axis, value } => {
                devices.send_gamepad_axis(axis.evdev(), value)
            }
            InputEventPacket::ControllerPresence { .. } => Ok(()),
        }
        .map_err(|error| InputError::Device(error.to_string()))
    }

    pub fn overlays(&self) -> Vec<InputOverlay> {
        self.sessions
            .lock()
            .expect("sessões PACORD")
            .values()
            .map(|session| InputOverlay {
                client_id: session.client_id,
                nickname: session.nickname.clone(),
                x: session.x,
                y: session.y,
                controller_active: session.controller_active,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{InputEventPacket, InputManager, InputPermissions};

    #[test]
    fn permissions_are_checked_before_device_creation() {
        let manager = InputManager::new(InputPermissions::none());
        let client = manager.register("alice".into());
        let error = manager
            .handle_event(client, InputEventPacket::PointerMotion { dx: 1, dy: 1 })
            .expect_err("mouse deveria ser bloqueado");
        assert!(matches!(
            error,
            super::InputError::PermissionDenied("mouse")
        ));
        assert!(manager.overlays()[0].nickname == "alice");
    }

    #[test]
    fn emergency_stop_removes_all_sessions() {
        let manager = InputManager::new(InputPermissions::all());
        manager.register("alice".into());
        manager.register("bob".into());
        assert_eq!(manager.revoke_all(), 2);
        assert!(manager.overlays().is_empty());
    }
}
