use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Offline,
    Server,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Storage mode: "offline" or "server".
    #[serde(default)]
    pub mode: Mode,

    /// Root directory for offline filesystem storage.
    pub data_dir: PathBuf,

    /// WebSocket URL of the sync server (only used in server mode).
    #[serde(default)]
    pub server_url: String,

    /// Authentication token for the sync server (only used in server mode).
    #[serde(default)]
    pub auth_token: String,

    /// gRPC listen address. Defaults to 127.0.0.1:50051.
    #[serde(default = "default_grpc_addr")]
    pub grpc_addr: String,

    /// Optional password for at-rest AES-256-GCM encryption (Argon2id key derivation).
    /// When set, all user content is encrypted before being written to the backend.
    #[serde(default)]
    pub encryption_password: Option<String>,
}

fn default_grpc_addr() -> String {
    "127.0.0.1:50051".to_string()
}

impl Config {
    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&raw)?;
        Ok(cfg)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Offline,
            data_dir: PathBuf::from("./keeplin-data"),
            server_url: String::new(),
            auth_token: String::new(),
            grpc_addr: default_grpc_addr(),
            encryption_password: None,
        }
    }
}
