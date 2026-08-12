use crate::rooms::{RoomError, RoomMode, RoomRecord};
use std::env;
use std::io;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Host,
    Viewer,
}

impl std::fmt::Display for SessionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Host => "HOST",
            Self::Viewer => "VIEWER",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Stopped,
    Starting(SessionKind),
    Running(SessionKind),
    Failed(String),
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped => f.write_str("PARADO"),
            Self::Starting(kind) => write!(f, "INICIANDO {kind}"),
            Self::Running(kind) => write!(f, "EXECUTANDO {kind}"),
            Self::Failed(error) => write!(f, "FALHA: {error}"),
        }
    }
}

#[derive(Debug)]
pub enum SessionError {
    Room(RoomError),
    Io(String),
    NotHost,
    AlreadyRunning,
    InvalidSecret,
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Room(error) => write!(f, "sala: {error}"),
            Self::Io(error) => write!(f, "processo: {error}"),
            Self::NotHost => f.write_str("a sala atual não é host"),
            Self::AlreadyRunning => f.write_str("já existe uma sessão em execução"),
            Self::InvalidSecret => f.write_str("segredo da sala inválido"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<RoomError> for SessionError {
    fn from(value: RoomError) -> Self {
        Self::Room(value)
    }
}

pub struct SessionController {
    child: Option<Child>,
    kind: Option<SessionKind>,
    state: SessionState,
}

impl Default for SessionController {
    fn default() -> Self {
        Self {
            child: None,
            kind: None,
            state: SessionState::Stopped,
        }
    }
}

impl SessionController {
    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn start_host(&mut self, room: &RoomRecord, backend: &str) -> Result<(), SessionError> {
        if room.mode != RoomMode::Host {
            return Err(SessionError::NotHost);
        }
        self.start_process(SessionKind::Host, room, backend)
    }

    pub fn start_viewer(&mut self, room: &RoomRecord) -> Result<(), SessionError> {
        if room.mode != RoomMode::Client {
            return Err(SessionError::Room(RoomError::InvalidInvite(
                "viewer exige uma sala cliente".into(),
            )));
        }
        self.start_process(SessionKind::Viewer, room, "")
    }

    fn start_process(
        &mut self,
        kind: SessionKind,
        room: &RoomRecord,
        backend: &str,
    ) -> Result<(), SessionError> {
        self.poll();
        if self.child.is_some() {
            return Err(SessionError::AlreadyRunning);
        }
        let endpoint = room.invite.endpoint()?;
        let secret = room.invite.secret()?;
        if secret.len() < 16 {
            return Err(SessionError::InvalidSecret);
        }
        self.state = SessionState::Starting(kind);
        let binary = resolve_binary(kind);
        let mut command = Command::new(binary);
        command
            .env("PACORD_SECRET", &room.invite.secret_hex)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        match kind {
            SessionKind::Host => {
                command.arg(backend).arg(endpoint.to_string());
                command.env("PACORD_ALLOW_INPUT", "");
                command.env(
                    "PACORD_ADMISSION_FILE",
                    std::env::temp_dir()
                        .join("pacord")
                        .join(format!("admission-{}.toml", room.invite.room_id)),
                );
            }
            SessionKind::Viewer => {
                command.arg(endpoint.to_string()).arg(&room.local_nickname);
            }
        }
        let child = command.spawn().map_err(|error| {
            self.state = SessionState::Failed(error.to_string());
            SessionError::Io(error.to_string())
        })?;
        self.child = Some(child);
        self.kind = Some(kind);
        self.state = SessionState::Running(kind);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), SessionError> {
        if let Some(mut child) = self.child.take() {
            child
                .kill()
                .map_err(|error| SessionError::Io(error.to_string()))?;
            let _ = child.wait();
        }
        self.kind = None;
        self.state = SessionState::Stopped;
        Ok(())
    }

    pub fn poll(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                let kind = self.kind.take();
                self.child = None;
                self.state = if status.success() {
                    SessionState::Stopped
                } else {
                    SessionState::Failed(format!(
                        "processo encerrou com código {}",
                        status.code().unwrap_or(-1)
                    ))
                };
                let _ = kind;
            }
            Ok(None) => {}
            Err(error) => self.state = SessionState::Failed(error.to_string()),
        }
    }
}

fn resolve_binary(kind: SessionKind) -> PathBuf {
    let (variable, name) = match kind {
        SessionKind::Host => ("PACORD_HOST_BIN", "pacord-host"),
        SessionKind::Viewer => ("PACORD_VIEWER_BIN", "pacord-viewer"),
    };
    if let Ok(value) = env::var(variable) {
        return PathBuf::from(value);
    }
    if let Ok(current_exe) = env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let sibling = parent.join(name);
            if sibling.exists() {
                return sibling;
            }
        }
    }
    PathBuf::from(name)
}

#[allow(dead_code)]
fn _io_error(error: io::Error) -> SessionError {
    SessionError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{SessionController, SessionState};

    #[test]
    fn controller_starts_stopped() {
        let controller = SessionController::default();
        assert_eq!(controller.state(), &SessionState::Stopped);
        assert!(!controller.is_running());
    }
}
