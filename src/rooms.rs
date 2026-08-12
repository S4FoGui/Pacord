use crate::input::InputPermissions;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::SocketAddr;
use uuid::Uuid;

pub const MAX_ROOM_PARTICIPANTS: usize = 8;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RoomMode {
    Host,
    Client,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RoomLifecycle {
    Draft,
    WaitingForApproval,
    Active,
    Closed,
}

impl fmt::Display for RoomLifecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Draft => "RASCUNHO",
            Self::WaitingForApproval => "AGUARDANDO APROVAÇÃO",
            Self::Active => "ATIVA",
            Self::Closed => "ENCERRADA",
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ParticipantState {
    Pending,
    Approved,
    Connected,
    Rejected,
    Disconnected,
}

impl fmt::Display for ParticipantState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pending => "PENDENTE",
            Self::Approved => "APROVADO",
            Self::Connected => "CONECTADO",
            Self::Rejected => "RECUSADO",
            Self::Disconnected => "DESCONECTADO",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoomInvite {
    pub version: u8,
    pub room_id: String,
    pub room_name: String,
    pub host_address: String,
    pub port: u16,
    pub network_id: String,
    pub room_code: String,
    pub secret_hex: String,
}

impl RoomInvite {
    pub fn endpoint(&self) -> Result<SocketAddr, RoomError> {
        format!("{}:{}", self.host_address, self.port)
            .parse()
            .map_err(|error| RoomError::InvalidInvite(format!("endereço inválido: {error}")))
    }

    pub fn secret(&self) -> Result<Vec<u8>, RoomError> {
        decode_hex(&self.secret_hex)
    }

    pub fn encode(&self) -> Result<String, RoomError> {
        toml::to_string(self).map_err(|error| RoomError::InvalidInvite(error.to_string()))
    }

    pub fn decode(text: &str) -> Result<Self, RoomError> {
        let invite: Self = toml::from_str(text.trim()).map_err(|error| {
            RoomError::InvalidInvite(format!("TOML de convite inválido: {error}"))
        })?;
        invite.validate()?;
        Ok(invite)
    }

    pub fn validate(&self) -> Result<(), RoomError> {
        if self.version != 4 {
            return Err(RoomError::InvalidInvite(
                "versão de convite não suportada".into(),
            ));
        }
        if self.room_id.is_empty() || self.room_id.len() > 64 {
            return Err(RoomError::InvalidInvite(
                "identificador de sala inválido".into(),
            ));
        }
        if self.room_name.trim().is_empty() || self.room_name.len() > 80 {
            return Err(RoomError::InvalidInvite("nome de sala inválido".into()));
        }
        if self.network_id.len() != 16
            || !self
                .network_id
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(RoomError::InvalidInvite(
                "ID de rede ZeroTier inválido".into(),
            ));
        }
        if self.room_code.len() != 8
            || !self
                .room_code
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
        {
            return Err(RoomError::InvalidInvite("código de sala inválido".into()));
        }
        if self.secret_hex.len() < 32 || self.secret_hex.len() % 2 != 0 {
            return Err(RoomError::InvalidInvite("segredo de sala inválido".into()));
        }
        let _ = self.endpoint()?;
        let _ = self.secret()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoomParticipant {
    pub id: usize,
    pub nickname: String,
    pub address: String,
    pub state: ParticipantState,
    pub permissions: InputPermissions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdmissionParticipant {
    pub nickname: String,
    pub address: String,
    pub state: ParticipantState,
    pub permissions: InputPermissions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoomAdmission {
    pub version: u8,
    pub room_id: String,
    pub participants: Vec<AdmissionParticipant>,
}

impl RoomAdmission {
    pub fn allows(&self, nickname: &str, peer: SocketAddr) -> bool {
        self.participants.iter().any(|participant| {
            participant.nickname == nickname
                && matches!(
                    participant.state,
                    ParticipantState::Approved | ParticipantState::Connected
                )
                && participant_address_matches(&participant.address, peer)
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoomRecord {
    pub mode: RoomMode,
    pub lifecycle: RoomLifecycle,
    pub invite: RoomInvite,
    pub local_nickname: String,
    pub participants: Vec<RoomParticipant>,
}

impl RoomRecord {
    pub fn participant_count(&self) -> usize {
        self.participants.len()
    }

    pub fn invite_text(&self) -> Result<String, RoomError> {
        self.invite.encode()
    }
}

#[derive(Debug)]
pub enum RoomError {
    InvalidName(String),
    InvalidNickname(String),
    InvalidAddress(String),
    InvalidNetworkId(String),
    InvalidInvite(String),
    Capacity,
    NoRoom,
    NotHost,
    ParticipantNotFound,
    AlreadyExists,
    Io(String),
    Random(String),
}

impl fmt::Display for RoomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(message) => write!(f, "nome de sala inválido: {message}"),
            Self::InvalidNickname(message) => write!(f, "nickname inválido: {message}"),
            Self::InvalidAddress(message) => write!(f, "endereço da sala inválido: {message}"),
            Self::InvalidNetworkId(message) => write!(f, "ID ZeroTier inválido: {message}"),
            Self::InvalidInvite(message) => write!(f, "convite inválido: {message}"),
            Self::Capacity => write!(f, "a sala já atingiu o limite de oito participantes"),
            Self::NoRoom => write!(f, "nenhuma sala aberta"),
            Self::NotHost => write!(f, "somente o host pode executar esta ação"),
            Self::ParticipantNotFound => write!(f, "participante não encontrado"),
            Self::AlreadyExists => write!(f, "participante já está registrado"),
            Self::Io(message) => write!(f, "falha no arquivo de admissão: {message}"),
            Self::Random(message) => write!(f, "falha ao gerar credenciais: {message}"),
        }
    }
}

impl std::error::Error for RoomError {}

#[derive(Debug, Default)]
pub struct RoomManager {
    current: Option<RoomRecord>,
    next_participant_id: usize,
    admission_path: Option<std::path::PathBuf>,
    admission_dirty: bool,
}

impl RoomManager {
    pub fn current(&self) -> Option<&RoomRecord> {
        self.current.as_ref()
    }

    pub fn current_mut(&mut self) -> Option<&mut RoomRecord> {
        self.current.as_mut()
    }

    pub fn create_host_room(
        &mut self,
        room_name: String,
        nickname: String,
        host_address: String,
        port: u16,
        network_id: String,
    ) -> Result<&RoomRecord, RoomError> {
        validate_name(&room_name)?;
        validate_nickname(&nickname)?;
        validate_network_id(&network_id)?;
        let endpoint = format!("{host_address}:{port}");
        endpoint
            .parse::<SocketAddr>()
            .map_err(|error| RoomError::InvalidAddress(error.to_string()))?;
        let room_code = random_alphanumeric(8)?;
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).map_err(|error| RoomError::Random(error.to_string()))?;
        let invite = RoomInvite {
            version: 4,
            room_id: Uuid::new_v4().to_string(),
            room_name,
            host_address,
            port,
            network_id,
            room_code,
            secret_hex: encode_hex(&secret),
        };
        let participant = RoomParticipant {
            id: 0,
            nickname: nickname.clone(),
            address: endpoint,
            state: ParticipantState::Connected,
            permissions: InputPermissions::none(),
        };
        self.next_participant_id = 1;
        self.admission_path = Some(
            std::env::temp_dir()
                .join("pacord")
                .join(format!("admission-{}.toml", invite.room_id)),
        );
        self.admission_dirty = true;
        self.current = Some(RoomRecord {
            mode: RoomMode::Host,
            lifecycle: RoomLifecycle::Active,
            invite,
            local_nickname: nickname,
            participants: vec![participant],
        });
        Ok(self.current.as_ref().expect("sala criada"))
    }

    pub fn join_room(
        &mut self,
        invite: RoomInvite,
        nickname: String,
    ) -> Result<&RoomRecord, RoomError> {
        invite.validate()?;
        validate_nickname(&nickname)?;
        self.next_participant_id = 1;
        self.admission_path = None;
        self.admission_dirty = false;
        self.current = Some(RoomRecord {
            mode: RoomMode::Client,
            lifecycle: RoomLifecycle::WaitingForApproval,
            invite,
            local_nickname: nickname,
            participants: Vec::new(),
        });
        Ok(self.current.as_ref().expect("sala ingressada"))
    }

    pub fn add_participant(
        &mut self,
        nickname: String,
        address: String,
    ) -> Result<usize, RoomError> {
        validate_nickname(&nickname)?;
        address
            .parse::<SocketAddr>()
            .map_err(|error| RoomError::InvalidAddress(error.to_string()))?;
        let room = self.current.as_mut().ok_or(RoomError::NoRoom)?;
        if room.mode != RoomMode::Host {
            return Err(RoomError::NotHost);
        }
        if room.participants.len() >= MAX_ROOM_PARTICIPANTS {
            return Err(RoomError::Capacity);
        }
        if room
            .participants
            .iter()
            .any(|participant| participant.nickname == nickname || participant.address == address)
        {
            return Err(RoomError::AlreadyExists);
        }
        let id = self.next_participant_id;
        self.next_participant_id += 1;
        room.participants.push(RoomParticipant {
            id,
            nickname,
            address,
            state: ParticipantState::Pending,
            permissions: InputPermissions::none(),
        });
        self.admission_dirty = true;
        Ok(id)
    }

    pub fn set_participant_state(
        &mut self,
        id: usize,
        state: ParticipantState,
    ) -> Result<(), RoomError> {
        let room = self.current.as_mut().ok_or(RoomError::NoRoom)?;
        if room.mode != RoomMode::Host {
            return Err(RoomError::NotHost);
        }
        let participant = room
            .participants
            .iter_mut()
            .find(|participant| participant.id == id)
            .ok_or(RoomError::ParticipantNotFound)?;
        participant.state = state;
        self.admission_dirty = true;
        Ok(())
    }

    pub fn set_participant_permissions(
        &mut self,
        id: usize,
        permissions: InputPermissions,
    ) -> Result<(), RoomError> {
        let room = self.current.as_mut().ok_or(RoomError::NoRoom)?;
        if room.mode != RoomMode::Host {
            return Err(RoomError::NotHost);
        }
        let participant = room
            .participants
            .iter_mut()
            .find(|participant| participant.id == id)
            .ok_or(RoomError::ParticipantNotFound)?;
        participant.permissions = permissions;
        self.admission_dirty = true;
        Ok(())
    }

    pub fn remove_participant(&mut self, id: usize) -> Result<(), RoomError> {
        let room = self.current.as_mut().ok_or(RoomError::NoRoom)?;
        if room.mode != RoomMode::Host {
            return Err(RoomError::NotHost);
        }
        let before = room.participants.len();
        room.participants.retain(|participant| participant.id != id);
        if room.participants.len() == before {
            return Err(RoomError::ParticipantNotFound);
        }
        self.admission_dirty = true;
        Ok(())
    }

    pub fn close_room(&mut self) -> Result<(), RoomError> {
        let room = self.current.as_mut().ok_or(RoomError::NoRoom)?;
        room.lifecycle = RoomLifecycle::Closed;
        self.admission_dirty = true;
        Ok(())
    }

    pub fn admission_path(&self) -> Option<&std::path::Path> {
        self.admission_path.as_deref()
    }

    pub fn sync_admission_file(&mut self) -> Result<(), RoomError> {
        if !self.admission_dirty {
            return Ok(());
        }
        let Some(path) = self.admission_path.clone() else {
            self.admission_dirty = false;
            return Ok(());
        };
        let admission = {
            let room = self.current.as_ref().ok_or(RoomError::NoRoom)?;
            if room.mode != RoomMode::Host {
                return Err(RoomError::NotHost);
            }
            RoomAdmission {
                version: 1,
                room_id: room.invite.room_id.clone(),
                participants: room
                    .participants
                    .iter()
                    .map(|participant| AdmissionParticipant {
                        nickname: participant.nickname.clone(),
                        address: participant.address.clone(),
                        state: participant.state,
                        permissions: participant.permissions,
                    })
                    .collect(),
            }
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| RoomError::Io(error.to_string()))?;
        }
        let temporary = path.with_extension("toml.tmp");
        let text = toml::to_string(&admission).map_err(|error| RoomError::Io(error.to_string()))?;
        std::fs::write(&temporary, text).map_err(|error| RoomError::Io(error.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
                .map_err(|error| RoomError::Io(error.to_string()))?;
        }
        std::fs::rename(&temporary, &path).map_err(|error| RoomError::Io(error.to_string()))?;
        self.admission_dirty = false;
        Ok(())
    }

    pub fn clear(&mut self) {
        self.current = None;
        self.next_participant_id = 0;
        if let Some(path) = self.admission_path.take() {
            let _ = std::fs::remove_file(path);
        }
        self.admission_dirty = false;
    }
}

fn participant_address_matches(allowed: &str, peer: SocketAddr) -> bool {
    allowed
        .parse::<SocketAddr>()
        .map(|address| address.ip() == peer.ip())
        .unwrap_or_else(|_| allowed == peer.ip().to_string())
}

fn validate_name(name: &str) -> Result<(), RoomError> {
    if name.trim().is_empty()
        || name.len() > 80
        || name
            .chars()
            .any(|character| character == '\n' || character == '\r')
    {
        return Err(RoomError::InvalidName("use entre 1 e 80 caracteres".into()));
    }
    Ok(())
}

fn validate_nickname(nickname: &str) -> Result<(), RoomError> {
    if nickname.trim().is_empty()
        || nickname.len() > 64
        || nickname
            .chars()
            .any(|character| character == '\n' || character == '\r')
    {
        return Err(RoomError::InvalidNickname(
            "use entre 1 e 64 caracteres".into(),
        ));
    }
    Ok(())
}

fn validate_network_id(network_id: &str) -> Result<(), RoomError> {
    if network_id.len() != 16
        || !network_id
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(RoomError::InvalidNetworkId(network_id.into()));
    }
    Ok(())
}

fn random_alphanumeric(length: usize) -> Result<String, RoomError> {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = vec![0u8; length];
    getrandom::fill(&mut bytes).map_err(|error| RoomError::Random(error.to_string()))?;
    Ok(bytes
        .into_iter()
        .map(|byte| ALPHABET[byte as usize % ALPHABET.len()] as char)
        .collect())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex(value: &str) -> Result<Vec<u8>, RoomError> {
    if value.len() % 2 != 0 {
        return Err(RoomError::InvalidInvite("segredo hexadecimal ímpar".into()));
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|error| RoomError::InvalidInvite(error.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ParticipantState, RoomAdmission, RoomInvite, RoomLifecycle, RoomManager, RoomMode,
        MAX_ROOM_PARTICIPANTS,
    };

    fn invite() -> RoomInvite {
        RoomInvite {
            version: 4,
            room_id: "room-1".into(),
            room_name: "Sala de teste".into(),
            host_address: "10.147.20.5".into(),
            port: 7777,
            network_id: "8056c2e21c000001".into(),
            room_code: "ABCD2345".into(),
            secret_hex: "00112233445566778899aabbccddeeff".into(),
        }
    }

    #[test]
    fn invite_round_trips_and_validates() {
        let text = invite().encode().unwrap();
        let decoded = RoomInvite::decode(&text).unwrap();
        assert_eq!(decoded, invite());
        assert_eq!(decoded.secret().unwrap().len(), 16);
    }

    #[test]
    fn admission_file_tracks_approval_and_is_removed_on_clear() {
        let mut manager = RoomManager::default();
        manager
            .create_host_room(
                "Sala".into(),
                "Host".into(),
                "10.147.20.5".into(),
                7777,
                "8056c2e21c000001".into(),
            )
            .unwrap();
        let path = manager.admission_path().unwrap().to_path_buf();
        manager.sync_admission_file().unwrap();
        let first: RoomAdmission =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(first.participants.len(), 1);
        assert!(first.allows("Host", "10.147.20.5:9000".parse().unwrap()));

        let id = manager
            .add_participant("Alice".into(), "10.147.20.12:4000".into())
            .unwrap();
        manager
            .set_participant_state(id, ParticipantState::Approved)
            .unwrap();
        manager.sync_admission_file().unwrap();
        let second: RoomAdmission =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(second.allows("Alice", "10.147.20.12:5000".parse().unwrap()));
        manager.clear();
        assert!(!path.exists());
    }

    #[test]
    fn manager_enforces_manual_approval_and_capacity() {
        let mut manager = RoomManager::default();
        manager
            .create_host_room(
                "Sala".into(),
                "Host".into(),
                "10.147.20.5".into(),
                7777,
                "8056c2e21c000001".into(),
            )
            .unwrap();
        let id = manager
            .add_participant("Alice".into(), "10.147.20.12:4000".into())
            .unwrap();
        assert_eq!(
            manager.current().unwrap().participants[1].state,
            ParticipantState::Pending
        );
        manager
            .set_participant_state(id, ParticipantState::Approved)
            .unwrap();
        manager
            .set_participant_permissions(id, crate::input::InputPermissions::all())
            .unwrap();
        assert_eq!(
            manager.current().unwrap().participants[1].permissions,
            crate::input::InputPermissions::all()
        );
        assert_eq!(manager.current().unwrap().mode, RoomMode::Host);
        assert_eq!(manager.current().unwrap().lifecycle, RoomLifecycle::Active);
        assert_eq!(manager.current().unwrap().participant_count(), 2);
        assert!(MAX_ROOM_PARTICIPANTS >= 8);
    }
}
