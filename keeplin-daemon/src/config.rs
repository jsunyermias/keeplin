//! Runtime configuration for the `keeplin-daemon` binary.
//!
//! This module defines the [`Config`] struct, which is deserialized from a TOML
//! file on startup, and the [`Mode`] enum, which selects between local-only filesystem
//! storage (`Offline`) and server-backed LibSQL storage (`Server`). Sensitive fields
//! such as passwords can be overridden at runtime by environment variables so they
//! never need to appear in the TOML file on disk.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The storage back-end mode that the daemon should use.
///
/// The serialised form (in TOML / JSON) uses lowercase strings (`"offline"` or
/// `"server"`) because the `#[serde(rename_all = "lowercase")]` attribute is applied
/// to this enum.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Store data locally in the filesystem (`FsBackend`). An external
    /// file-synchronisation tool such as Syncthing is responsible for replicating
    /// data between devices. No network connection to a central server is required.
    #[default]
    Offline,
    /// Store data in a local LibSQL database and synchronise with a central server
    /// over a WebSocket connection (`DbBackend`). The server URL and authentication
    /// token must be provided in the configuration file.
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

    /// Path to the TLS certificate file (PEM format).
    /// Both tls_cert_path and tls_key_path must be set to enable TLS.
    #[serde(default)]
    pub tls_cert_path: Option<String>,

    /// Path to the TLS private key file (PEM format).
    #[serde(default)]
    pub tls_key_path: Option<String>,

    /// Maximum gRPC message size in bytes (default: 32 MiB).
    /// Covers PDFs and images up to ~32 MiB without manual tuning.
    #[serde(default = "default_max_message_size")]
    pub max_message_size: usize,

    /// How many days of change-journal history to retain (default: 30).
    ///
    /// After each successful sync the daemon prunes `entity_changes` rows older than
    /// this many days (no-op for the filesystem backend, whose logs are replicated by
    /// Syncthing). Keep this comfortably larger than the longest a peer device is
    /// expected to stay offline. Set to `0` to disable pruning entirely.
    #[serde(default = "default_journal_retention_days")]
    pub journal_retention_days: u64,

    /// Optional password for at-rest AES-256-GCM encryption (Argon2id key derivation).
    /// Prefer the KEEPLIN_ENCRYPTION_PASSWORD environment variable over storing the
    /// password in this file to avoid accidentally committing it to version control.
    #[serde(default)]
    pub encryption_password: Option<String>,

    /// Optional Argon2id salt for the encryption key (at least 8 bytes).
    ///
    /// The salt is not secret, but it must be identical on every device that needs to
    /// decrypt the same data. **Set the same value on all synced devices** to make
    /// encrypted notes portable between them. When left unset, the daemon falls back to
    /// this device's ID, which keeps encrypted data readable only on the device that
    /// wrote it — safe for single-device use but not for sync. May also be supplied via
    /// the KEEPLIN_KEY_SALT environment variable.
    #[serde(default)]
    pub key_salt: Option<String>,

    /// Username for gRPC client authentication (HTTP Basic Auth).
    /// When both auth_username and auth_password are set, every gRPC call must
    /// include an `authorization: Basic <base64(user:pass)>` metadata header.
    /// This applies equally in offline and server mode.
    /// Prefer the KEEPLIN_AUTH_USERNAME environment variable over storing the
    /// username here.
    #[serde(default)]
    pub auth_username: Option<String>,

    /// Password for gRPC client authentication.
    /// Prefer the KEEPLIN_AUTH_PASSWORD environment variable over storing the
    /// password here to avoid committing credentials to version control.
    #[serde(default)]
    pub auth_password: Option<String>,
}

/// Returns the default gRPC listen address: `127.0.0.1:50051`.
///
/// Binding to the loopback interface by default prevents accidental network exposure
/// without authentication when the daemon is first started with no configuration file.
fn default_grpc_addr() -> String {
    "127.0.0.1:50051".to_string()
}

/// Returns the default maximum gRPC message size in bytes (32 MiB = 33,554,432 bytes).
///
/// This limit applies to both incoming (decoding) and outgoing (encoding) messages.
/// 32 MiB covers typical PDFs and images without requiring manual tuning for most
/// use cases.
fn default_max_message_size() -> usize {
    32 * 1024 * 1024
}

/// Returns the default change-journal retention window in days (30).
///
/// Thirty days comfortably exceeds the time a peer device is normally offline, so
/// pruning entries older than this does not strand a device that has not yet synced.
fn default_journal_retention_days() -> u64 {
    30
}

impl Config {
    /// Load a [`Config`] from a TOML file at `path`.
    ///
    /// Reads the entire file into memory, parses it with the `toml` crate, and
    /// returns the resulting `Config`. Missing optional fields fall back to their
    /// `#[serde(default)]` values so a minimal TOML file with only `data_dir` is
    /// sufficient to start the daemon in offline mode.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or if the TOML is malformed.
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
            tls_cert_path: None,
            tls_key_path: None,
            max_message_size: default_max_message_size(),
            journal_retention_days: default_journal_retention_days(),
            encryption_password: None,
            key_salt: None,
            auth_username: None,
            auth_password: None,
        }
    }
}
