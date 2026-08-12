use serde::Deserialize;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZeroTierNodeStatus {
    Online,
    Offline,
    Tunneled,
    Unknown,
}

impl fmt::Display for ZeroTierNodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Online => "ONLINE",
            Self::Offline => "OFFLINE",
            Self::Tunneled => "TUNNELED",
            Self::Unknown => "UNKNOWN",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZeroTierNode {
    pub address: String,
    pub version: String,
    pub status: ZeroTierNodeStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZeroTierNetwork {
    pub id: String,
    pub name: String,
    pub status: String,
    pub device: String,
    pub assigned_addresses: Vec<String>,
    pub authorized: bool,
}

impl ZeroTierNetwork {
    pub fn first_ipv4(&self) -> Option<String> {
        self.assigned_addresses.iter().find_map(|address| {
            let ip = address.split('/').next()?;
            ip.parse::<std::net::Ipv4Addr>()
                .ok()
                .map(|_| ip.to_string())
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZeroTierSnapshot {
    pub node: ZeroTierNode,
    pub networks: Vec<ZeroTierNetwork>,
}

impl ZeroTierSnapshot {
    pub fn first_ipv4(&self) -> Option<String> {
        self.networks.iter().find_map(ZeroTierNetwork::first_ipv4)
    }
}

#[derive(Debug)]
pub enum ZeroTierError {
    NotInstalled(String),
    PermissionDenied(String),
    CommandFailed(String),
    InvalidJson(String),
    InvalidNetworkId(String),
}

impl fmt::Display for ZeroTierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled(message) => write!(f, "ZeroTier não encontrado: {message}"),
            Self::PermissionDenied(message) => {
                write!(f, "permissão do ZeroTier insuficiente: {message}")
            }
            Self::CommandFailed(message) => write!(f, "comando ZeroTier falhou: {message}"),
            Self::InvalidJson(message) => {
                write!(f, "resposta JSON inválida do ZeroTier: {message}")
            }
            Self::InvalidNetworkId(message) => write!(f, "ID de rede ZeroTier inválido: {message}"),
        }
    }
}

impl Error for ZeroTierError {}

#[derive(Debug, Deserialize)]
struct RawStatus {
    #[serde(default)]
    address: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    online: bool,
    #[serde(default, rename = "tcpFallbackActive")]
    tcp_fallback_active: bool,
}

#[derive(Debug, Deserialize)]
struct RawNetwork {
    #[serde(default)]
    id: String,
    #[serde(default)]
    nwid: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    status: String,
    #[serde(default, rename = "dev")]
    device: String,
    #[serde(default, rename = "assignedAddresses")]
    assigned_addresses: Vec<String>,
    #[serde(default)]
    authorized: bool,
}

#[derive(Debug, Clone)]
pub struct ZeroTierClient {
    binary: PathBuf,
}

impl Default for ZeroTierClient {
    fn default() -> Self {
        Self::new("zerotier-cli")
    }
}

impl ZeroTierClient {
    pub fn new<P: AsRef<Path>>(binary: P) -> Self {
        Self {
            binary: binary.as_ref().to_path_buf(),
        }
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    pub fn inspect(&self) -> Result<ZeroTierSnapshot, ZeroTierError> {
        let status: RawStatus = self.json_command("status")?;
        let raw_networks: Vec<RawNetwork> = self.json_command("listnetworks")?;
        let node_status = if status.tcp_fallback_active {
            ZeroTierNodeStatus::Tunneled
        } else if status.online {
            ZeroTierNodeStatus::Online
        } else {
            ZeroTierNodeStatus::Offline
        };
        let networks = raw_networks
            .into_iter()
            .map(|network| ZeroTierNetwork {
                id: if network.id.is_empty() {
                    network.nwid
                } else {
                    network.id
                },
                name: network.name,
                status: network.status,
                device: network.device,
                assigned_addresses: network.assigned_addresses,
                authorized: network.authorized,
            })
            .collect();
        Ok(ZeroTierSnapshot {
            node: ZeroTierNode {
                address: status.address,
                version: status.version,
                status: node_status,
            },
            networks,
        })
    }

    pub fn join(&self, network_id: &str) -> Result<String, ZeroTierError> {
        validate_network_id(network_id)?;
        self.text_command(["join", network_id])
    }

    pub fn leave(&self, network_id: &str) -> Result<String, ZeroTierError> {
        validate_network_id(network_id)?;
        self.text_command(["leave", network_id])
    }

    fn json_command<T>(&self, command: &str) -> Result<T, ZeroTierError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let output = self.run(["-j", command])?;
        serde_json::from_slice(&output)
            .map_err(|error| ZeroTierError::InvalidJson(error.to_string()))
    }

    fn text_command<const N: usize>(&self, args: [&str; N]) -> Result<String, ZeroTierError> {
        let output = self.run(args)?;
        Ok(String::from_utf8_lossy(&output).trim().to_string())
    }

    fn run<const N: usize>(&self, args: [&str; N]) -> Result<Vec<u8>, ZeroTierError> {
        let output = Command::new(&self.binary)
            .args(args)
            .output()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    ZeroTierError::NotInstalled(error.to_string())
                } else {
                    ZeroTierError::CommandFailed(error.to_string())
                }
            })?;
        if output.status.success() {
            return Ok(output.stdout);
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        } else {
            stderr
        };
        if message.to_ascii_lowercase().contains("authentication")
            || message.to_ascii_lowercase().contains("permission")
            || message.to_ascii_lowercase().contains("privilege")
        {
            Err(ZeroTierError::PermissionDenied(message))
        } else {
            Err(ZeroTierError::CommandFailed(message))
        }
    }
}

fn validate_network_id(network_id: &str) -> Result<(), ZeroTierError> {
    if network_id.len() != 16
        || !network_id
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(ZeroTierError::InvalidNetworkId(network_id.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_network_id, ZeroTierNetwork, ZeroTierNodeStatus};

    #[test]
    fn validates_zero_tier_network_ids() {
        assert!(validate_network_id("8056c2e21c000001").is_ok());
        assert!(validate_network_id("short").is_err());
        assert!(validate_network_id("8056c2e21c00000z").is_err());
    }

    #[test]
    fn finds_first_ipv4_on_network() {
        let network = ZeroTierNetwork {
            id: "8056c2e21c000001".into(),
            name: "PACORD".into(),
            status: "OK".into(),
            device: "ztabc".into(),
            assigned_addresses: vec!["fd00::1/64".into(), "10.147.20.5/24".into()],
            authorized: true,
        };
        assert_eq!(network.first_ipv4().as_deref(), Some("10.147.20.5"));
        assert_eq!(ZeroTierNodeStatus::Online.to_string(), "ONLINE");
    }
}
