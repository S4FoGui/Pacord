use crate::input::{InputEventPacket, InputManager, InputOverlay, InputPermissions};
use crate::rooms::RoomAdmission;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::error::Error;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

pub const MAX_CLIENTS: usize = 8;
pub const PROTOCOL_VERSION: u16 = 4;
const MAX_MESSAGE_BYTES: u32 = 8 * 1024 * 1024;
const CHALLENGE_BYTES: usize = 32;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FramePacket {
    pub sequence: u64,
    pub captured_at_micros: u64,
    pub width: u32,
    pub height: u32,
    pub jpeg: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputOverlayPacket {
    pub client_id: usize,
    pub nickname: String,
    pub x: f32,
    pub y: f32,
    pub controller_active: bool,
}

impl From<InputOverlay> for InputOverlayPacket {
    fn from(value: InputOverlay) -> Self {
        Self {
            client_id: value.client_id,
            nickname: value.nickname,
            x: value.x,
            y: value.y,
            controller_active: value.controller_active,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
enum ClientMessage {
    Hello {
        protocol_version: u16,
        nickname: String,
        proof: [u8; 32],
    },
    Input(InputEventPacket),
    Stop,
}

#[derive(Debug, Serialize, Deserialize)]
enum ServerMessage {
    Challenge {
        nonce: [u8; 32],
    },
    Accepted {
        session_id: String,
        client_id: usize,
    },
    Rejected {
        reason: String,
    },
    InputPolicy {
        permissions: InputPermissions,
    },
    Overlays {
        overlays: Vec<InputOverlayPacket>,
    },
    InputAccepted,
    InputRejected {
        reason: String,
    },
    Frame(FramePacket),
    Stopped,
}

#[derive(Debug)]
pub enum TransportError {
    Io(std::io::Error),
    Codec(Box<bincode::ErrorKind>),
    Authentication,
    Rejected(String),
    Protocol(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "erro de rede: {err}"),
            Self::Codec(err) => write!(f, "erro de protocolo: {err}"),
            Self::Authentication => write!(f, "autenticação PACORD recusada"),
            Self::Rejected(reason) => write!(f, "sessão rejeitada: {reason}"),
            Self::Protocol(reason) => write!(f, "protocolo inválido: {reason}"),
        }
    }
}

impl Error for TransportError {}

impl From<std::io::Error> for TransportError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<Box<bincode::ErrorKind>> for TransportError {
    fn from(value: Box<bincode::ErrorKind>) -> Self {
        Self::Codec(value)
    }
}

fn proof(secret: &[u8], nonce: &[u8; CHALLENGE_BYTES], nickname: &str) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC aceita qualquer chave não vazia");
    mac.update(nonce);
    mac.update(nickname.as_bytes());
    mac.update(&PROTOCOL_VERSION.to_be_bytes());
    mac.finalize().into_bytes().into()
}

async fn write_message<W, T>(writer: &mut W, message: &T) -> Result<(), TransportError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = bincode::serialize(message)?;
    if body.len() > MAX_MESSAGE_BYTES as usize {
        return Err(TransportError::Protocol("mensagem excede o limite".into()));
    }
    writer.write_u32(body.len() as u32).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_message<R, T>(reader: &mut R) -> Result<T, TransportError>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let len = reader.read_u32().await?;
    if len == 0 || len > MAX_MESSAGE_BYTES {
        return Err(TransportError::Protocol(
            "tamanho de mensagem inválido".into(),
        ));
    }
    let mut body = vec![0u8; len as usize];
    reader.read_exact(&mut body).await?;
    Ok(bincode::deserialize(&body)?)
}

#[derive(Clone)]
pub struct FrameServer {
    bind_addr: SocketAddr,
    secret: Arc<Vec<u8>>,
    clients: Arc<AtomicUsize>,
    input_manager: Arc<InputManager>,
    overlays: broadcast::Sender<Vec<InputOverlayPacket>>,
    admission_file: Option<PathBuf>,
}

impl FrameServer {
    pub fn new(bind_addr: SocketAddr, secret: Vec<u8>) -> Result<Self, TransportError> {
        if secret.len() < 16 {
            return Err(TransportError::Protocol(
                "o segredo do PACORD deve ter pelo menos 16 bytes".into(),
            ));
        }
        let (overlays, _) = broadcast::channel(8);
        Ok(Self {
            bind_addr,
            secret: Arc::new(secret),
            clients: Arc::new(AtomicUsize::new(0)),
            input_manager: Arc::new(InputManager::new(InputPermissions::none())),
            overlays,
            admission_file: std::env::var_os("PACORD_ADMISSION_FILE").map(PathBuf::from),
        })
    }

    pub fn with_input_manager(mut self, input_manager: Arc<InputManager>) -> Self {
        self.input_manager = input_manager;
        self
    }

    pub fn input_manager(&self) -> Arc<InputManager> {
        self.input_manager.clone()
    }

    pub fn with_admission_file(mut self, path: Option<PathBuf>) -> Self {
        self.admission_file = path;
        self
    }

    pub async fn run(&self, frames: broadcast::Sender<FramePacket>) -> Result<(), TransportError> {
        let listener = TcpListener::bind(self.bind_addr).await?;
        self.run_on_listener(listener, frames).await
    }

    async fn run_on_listener(
        &self,
        listener: TcpListener,
        frames: broadcast::Sender<FramePacket>,
    ) -> Result<(), TransportError> {
        loop {
            let (stream, peer) = listener.accept().await?;
            if self
                .clients
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    (count < MAX_CLIENTS).then_some(count + 1)
                })
                .is_err()
            {
                let _ = reject_connection(stream, "limite de oito clientes atingido").await;
                continue;
            }

            let secret = self.secret.clone();
            let clients = self.clients.clone();
            let frames = frames.clone();
            let input_manager = self.input_manager.clone();
            let overlays = self.overlays.clone();
            let admission_file = self.admission_file.clone();
            tokio::spawn(async move {
                let result = handle_client(
                    stream,
                    peer,
                    secret,
                    frames,
                    input_manager,
                    overlays,
                    admission_file,
                )
                .await;
                clients.fetch_sub(1, Ordering::AcqRel);
                if let Err(error) = result {
                    log::warn!("cliente PACORD {peer} desconectado: {error}");
                }
            });
        }
    }
}

async fn reject_connection(mut stream: TcpStream, reason: &str) -> Result<(), TransportError> {
    write_message(
        &mut stream,
        &ServerMessage::Rejected {
            reason: reason.into(),
        },
    )
    .await
}

async fn handle_client(
    mut stream: TcpStream,
    peer: SocketAddr,
    secret: Arc<Vec<u8>>,
    frames: broadcast::Sender<FramePacket>,
    input_manager: Arc<InputManager>,
    overlays: broadcast::Sender<Vec<InputOverlayPacket>>,
    admission_file: Option<PathBuf>,
) -> Result<(), TransportError> {
    let mut nonce = [0u8; CHALLENGE_BYTES];
    getrandom::fill(&mut nonce).map_err(|e| TransportError::Protocol(e.to_string()))?;
    write_message(&mut stream, &ServerMessage::Challenge { nonce }).await?;

    let hello: ClientMessage = read_message(&mut stream).await?;
    let ClientMessage::Hello {
        protocol_version,
        nickname,
        proof: received_proof,
    } = hello
    else {
        return Err(TransportError::Protocol(
            "primeira mensagem não é Hello".into(),
        ));
    };

    if protocol_version != PROTOCOL_VERSION || nickname.trim().is_empty() || nickname.len() > 64 {
        let _ = reject_connection(stream, "identificação de cliente inválida").await;
        return Err(TransportError::Rejected("identificação inválida".into()));
    }
    let expected = proof(&secret, &nonce, &nickname);
    if received_proof != expected {
        let _ = reject_connection(stream, "prova de posse do segredo inválida").await;
        return Err(TransportError::Authentication);
    }

    if let Some(path) = admission_file {
        let admission = load_admission(&path)?;
        if !admission.allows(&nickname, peer) {
            let _ =
                reject_connection(stream, "participante ainda não foi aprovado pelo host").await;
            return Err(TransportError::Rejected("participante não aprovado".into()));
        }
    }

    let client_id = input_manager.register(nickname);
    write_message(
        &mut stream,
        &ServerMessage::Accepted {
            session_id: Uuid::new_v4().to_string(),
            client_id,
        },
    )
    .await?;
    write_message(
        &mut stream,
        &ServerMessage::InputPolicy {
            permissions: input_manager.permissions(),
        },
    )
    .await?;
    write_message(
        &mut stream,
        &ServerMessage::Overlays {
            overlays: input_manager
                .overlays()
                .into_iter()
                .map(Into::into)
                .collect(),
        },
    )
    .await?;

    let mut frame_receiver = frames.subscribe();
    let mut overlay_receiver = overlays.subscribe();
    loop {
        tokio::select! {
            frame = frame_receiver.recv() => {
                match frame {
                    Ok(frame) => write_message(&mut stream, &ServerMessage::Frame(frame)).await?,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => {
                        write_message(&mut stream, &ServerMessage::Stopped).await?;
                        input_manager.disconnect(client_id);
                        return Ok(());
                    }
                }
            }
            overlay = overlay_receiver.recv() => {
                match overlay {
                    Ok(overlays) => write_message(&mut stream, &ServerMessage::Overlays { overlays }).await?,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => continue,
                }
            }
            message = read_message::<_, ClientMessage>(&mut stream) => {
                match message? {
                    ClientMessage::Input(event) => {
                        match input_manager.handle_event(client_id, event) {
                            Ok(()) => {
                                let snapshot = input_manager.overlays().into_iter().map(Into::into).collect();
                                let _ = overlays.send(snapshot);
                                write_message(&mut stream, &ServerMessage::InputAccepted).await?;
                            }
                            Err(error) => write_message(&mut stream, &ServerMessage::InputRejected { reason: error.to_string() }).await?,
                        }
                    }
                    ClientMessage::Stop => {
                        input_manager.disconnect(client_id);
                        let snapshot = input_manager.overlays().into_iter().map(Into::into).collect();
                        let _ = overlays.send(snapshot);
                        write_message(&mut stream, &ServerMessage::Stopped).await?;
                        return Ok(());
                    }
                    ClientMessage::Hello { .. } => {
                        return Err(TransportError::Protocol("Hello repetido".into()));
                    }
                }
            }
        }
    }
}

fn load_admission(path: &std::path::Path) -> Result<RoomAdmission, TransportError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        TransportError::Protocol(format!("arquivo de admissão indisponível: {error}"))
    })?;
    toml::from_str(&text)
        .map_err(|error| TransportError::Protocol(format!("arquivo de admissão inválido: {error}")))
}

#[derive(Debug)]
pub enum ServerEvent {
    Frame(FramePacket),
    InputPolicy(InputPermissions),
    Overlays(Vec<InputOverlayPacket>),
    InputAccepted,
    InputRejected(String),
    Stopped,
}

#[derive(Debug, Clone)]
pub struct InputClientHandle {
    outgoing: mpsc::Sender<ClientMessage>,
}

impl InputClientHandle {
    pub fn try_send_input(&self, event: InputEventPacket) -> Result<(), TransportError> {
        self.outgoing
            .try_send(ClientMessage::Input(event))
            .map_err(|error| {
                TransportError::Protocol(format!("fila de entrada indisponível: {error}"))
            })
    }

    pub async fn send_input(&self, event: InputEventPacket) -> Result<(), TransportError> {
        self.outgoing
            .send(ClientMessage::Input(event))
            .await
            .map_err(|_| TransportError::Protocol("canal de entrada encerrado".into()))
    }

    pub async fn stop(&self) -> Result<(), TransportError> {
        self.outgoing
            .send(ClientMessage::Stop)
            .await
            .map_err(|_| TransportError::Protocol("canal de entrada encerrado".into()))
    }
}

async fn handshake(
    server: SocketAddr,
    nickname: String,
    secret: Vec<u8>,
) -> Result<TcpStream, TransportError> {
    if secret.len() < 16 {
        return Err(TransportError::Protocol(
            "o segredo do PACORD deve ter pelo menos 16 bytes".into(),
        ));
    }
    if nickname.trim().is_empty() || nickname.len() > 64 {
        return Err(TransportError::Protocol("apelido inválido".into()));
    }
    let mut stream = TcpStream::connect(server).await?;
    let challenge: ServerMessage = read_message(&mut stream).await?;
    let ServerMessage::Challenge { nonce } = challenge else {
        return Err(TransportError::Protocol(
            "servidor não enviou desafio".into(),
        ));
    };
    write_message(
        &mut stream,
        &ClientMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            proof: proof(&secret, &nonce, &nickname),
            nickname,
        },
    )
    .await?;
    match read_message::<_, ServerMessage>(&mut stream).await? {
        ServerMessage::Accepted { .. } => Ok(stream),
        ServerMessage::Rejected { reason } => Err(TransportError::Rejected(reason)),
        _ => Err(TransportError::Protocol(
            "resposta de aceitação inválida".into(),
        )),
    }
}

pub async fn connect_input_client(
    server: SocketAddr,
    nickname: String,
    secret: Vec<u8>,
) -> Result<
    (
        InputClientHandle,
        mpsc::Receiver<Result<ServerEvent, TransportError>>,
    ),
    TransportError,
> {
    let stream = handshake(server, nickname, secret).await?;
    let (mut reader, mut writer) = stream.into_split();
    let (outgoing, mut outgoing_receiver) = mpsc::channel::<ClientMessage>(64);
    let (events_sender, events_receiver) = mpsc::channel::<Result<ServerEvent, TransportError>>(16);
    let events_for_reader = events_sender.clone();
    tokio::spawn(async move {
        while let Some(message) = outgoing_receiver.recv().await {
            if let Err(error) = write_message(&mut writer, &message).await {
                let _ = events_sender.send(Err(error)).await;
                break;
            }
        }
    });
    tokio::spawn(async move {
        loop {
            match read_message::<_, ServerMessage>(&mut reader).await {
                Ok(ServerMessage::Frame(frame)) => {
                    if events_for_reader
                        .send(Ok(ServerEvent::Frame(frame)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(ServerMessage::InputPolicy { permissions }) => {
                    if events_for_reader
                        .send(Ok(ServerEvent::InputPolicy(permissions)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(ServerMessage::Overlays { overlays }) => {
                    if events_for_reader
                        .send(Ok(ServerEvent::Overlays(overlays)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(ServerMessage::InputAccepted) => {
                    if events_for_reader
                        .send(Ok(ServerEvent::InputAccepted))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(ServerMessage::InputRejected { reason }) => {
                    if events_for_reader
                        .send(Ok(ServerEvent::InputRejected(reason)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(ServerMessage::Stopped) => {
                    let _ = events_for_reader.send(Ok(ServerEvent::Stopped)).await;
                    break;
                }
                Ok(ServerMessage::Challenge { .. })
                | Ok(ServerMessage::Accepted { .. })
                | Ok(ServerMessage::Rejected { .. }) => {
                    let _ = events_for_reader
                        .send(Err(TransportError::Protocol(
                            "mensagem de sessão inesperada".into(),
                        )))
                        .await;
                    break;
                }
                Err(error) => {
                    let _ = events_for_reader.send(Err(error)).await;
                    break;
                }
            }
        }
    });
    Ok((InputClientHandle { outgoing }, events_receiver))
}

pub async fn connect_client(
    server: SocketAddr,
    nickname: String,
    secret: Vec<u8>,
) -> Result<mpsc::Receiver<Result<FramePacket, TransportError>>, TransportError> {
    let (_input_handle, mut events) = connect_input_client(server, nickname, secret).await?;
    let (sender, receiver) = mpsc::channel(2);
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                Ok(ServerEvent::Frame(frame)) => {
                    if sender.send(Ok(frame)).await.is_err() {
                        break;
                    }
                }
                Ok(ServerEvent::Stopped) => break,
                Ok(ServerEvent::InputRejected(reason)) => {
                    let _ = sender.send(Err(TransportError::Rejected(reason))).await;
                }
                Ok(
                    ServerEvent::InputPolicy(_)
                    | ServerEvent::Overlays(_)
                    | ServerEvent::InputAccepted,
                ) => {}
                Err(error) => {
                    let _ = sender.send(Err(error)).await;
                    break;
                }
            }
        }
    });
    Ok(receiver)
}

#[cfg(test)]
mod tests {
    use super::{
        connect_input_client, proof, FramePacket, FrameServer, ServerEvent, PROTOCOL_VERSION,
    };
    use crate::input::{InputEventPacket, InputManager, InputPermissions};
    use std::sync::Arc;

    #[test]
    fn proof_depends_on_nonce_and_nickname() {
        let secret = b"pacord-test-secret-1234";
        let nonce = [7u8; 32];
        let first = proof(secret, &nonce, "alice");
        assert_eq!(first, proof(secret, &nonce, "alice"));
        assert_ne!(first, proof(secret, &nonce, "bob"));
        assert_ne!(first, proof(secret, &[8u8; 32], "alice"));
        assert_eq!(PROTOCOL_VERSION, 4);
    }

    #[tokio::test]
    async fn authenticated_client_receives_frame_and_policy() {
        let secret = b"pacord-test-secret-1234".to_vec();
        let input = Arc::new(InputManager::new(InputPermissions::none()));
        let server = FrameServer::new("127.0.0.1:0".parse().unwrap(), secret.clone())
            .unwrap()
            .with_input_manager(input);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (frames, _) = tokio::sync::broadcast::channel(2);
        let server_task = tokio::spawn({
            let server = server.clone();
            let frames = frames.clone();
            async move { server.run_on_listener(listener, frames).await }
        });

        let (_handle, mut events) = connect_input_client(address, "alice".into(), secret)
            .await
            .expect("cliente deveria autenticar");
        let mut saw_policy = false;
        for _ in 0..3 {
            if let Some(Ok(ServerEvent::InputPolicy(policy))) =
                tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
                    .await
                    .expect("policy não chegou")
            {
                assert_eq!(policy, InputPermissions::none());
                saw_policy = true;
                break;
            }
        }
        assert!(saw_policy);
        frames
            .send(FramePacket {
                sequence: 42,
                captured_at_micros: 100,
                width: 2,
                height: 1,
                jpeg: vec![1, 2, 3],
            })
            .unwrap();
        let mut saw_frame = false;
        for _ in 0..4 {
            if let Some(Ok(ServerEvent::Frame(frame))) =
                tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
                    .await
                    .expect("frame não chegou")
            {
                assert_eq!(frame.sequence, 42);
                saw_frame = true;
                break;
            }
        }
        assert!(saw_frame);
        server_task.abort();
    }

    #[tokio::test]
    async fn input_is_rejected_when_host_permission_is_off() {
        let secret = b"pacord-test-secret-1234".to_vec();
        let input = Arc::new(InputManager::new(InputPermissions::none()));
        let server = FrameServer::new("127.0.0.1:0".parse().unwrap(), secret.clone())
            .unwrap()
            .with_input_manager(input);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (frames, _) = tokio::sync::broadcast::channel(2);
        let server_task = tokio::spawn({
            let server = server.clone();
            async move { server.run_on_listener(listener, frames).await }
        });
        let (handle, mut events) = connect_input_client(address, "alice".into(), secret)
            .await
            .unwrap();
        handle
            .send_input(InputEventPacket::PointerMotion { dx: 1, dy: 1 })
            .await
            .unwrap();
        let mut saw_rejection = false;
        for _ in 0..5 {
            if let Some(Ok(ServerEvent::InputRejected(reason))) =
                tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
                    .await
                    .unwrap()
            {
                assert!(reason.contains("mouse"));
                saw_rejection = true;
                break;
            }
        }
        assert!(saw_rejection);
        server_task.abort();
    }
}
