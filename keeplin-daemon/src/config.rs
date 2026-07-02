//! Runtime configuration for the `keeplin-daemon` binary.
//!
//! This module defines the [`Config`] struct, which is deserialized from a TOML
//! file on startup, and the [`Mode`] enum, which selects between local-only filesystem
//! storage (`Offline`) and server-backed LibSQL storage (`Server`). Sensitive fields
//! such as passwords can be overridden at runtime by environment variables so they
//! never need to appear in the TOML file on disk.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
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

    /// Optional HTTP listen address for the REST/JSON API and the WebSocket change feed
    /// (e.g. `127.0.0.1:50052`). When unset, only the gRPC server runs. The HTTP listener
    /// is plain HTTP — terminate TLS at a reverse proxy in production. The same
    /// `auth_username`/`auth_password` Basic-Auth credentials apply.
    #[serde(default)]
    pub http_addr: Option<String>,

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
    /// After each successful sync — driven by the gRPC `Sync` RPC or REST
    /// `POST /api/sync` alike — the daemon prunes `entity_changes` rows older than
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

    /// Escape hatch that downgrades the startup security checks from **errors** to warnings.
    ///
    /// By default the daemon **refuses to start** in a configuration that would expose data
    /// or credentials without protection — a network-reachable API with no auth, or a
    /// plaintext `ws://` sync URL to a remote host that would leak the bearer token (see
    /// [`Config::security_issues`]). Set `insecure = true` only for deployments where another
    /// layer provides that protection (an isolated network, an mTLS mesh, a fronting proxy that
    /// also enforces auth); the daemon then logs each issue as a warning and starts anyway.
    #[serde(default)]
    pub insecure: bool,
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
            http_addr: None,
            tls_cert_path: None,
            tls_key_path: None,
            max_message_size: default_max_message_size(),
            journal_retention_days: default_journal_retention_days(),
            encryption_password: None,
            key_salt: None,
            auth_username: None,
            auth_password: None,
            insecure: false,
        }
    }
}

impl Config {
    /// Enumerate the security problems in this configuration that would expose data or
    /// credentials on an untrusted network. Empty means the config is safe to start.
    ///
    /// Pure and side-effect-free so it is easy to unit-test; the daemon calls it once at
    /// startup and — unless [`insecure`](Self::insecure) is set — refuses to start when it
    /// returns anything (see `main::serve`). Each string is a complete, human-readable line.
    ///
    /// It flags only unambiguous exposures that no fronting TLS proxy can fix, so the
    /// documented "terminate TLS at a reverse proxy" deployment is never blocked:
    /// - a **network-reachable** (non-loopback) gRPC or HTTP listener with **no auth**
    ///   configured — a proxy cannot invent application credentials; and
    /// - a **plaintext `ws://` sync URL to a non-loopback host** (server mode), which sends the
    ///   `auth_token` in the clear on the daemon's *outbound* connection, where a proxy in
    ///   front of the daemon does not help.
    ///
    /// Missing daemon-terminated TLS on the listeners is deliberately **not** flagged: fronting
    /// TLS at a reverse proxy is a supported, documented deployment.
    pub fn security_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        let auth = self.auth_username.is_some() && self.auth_password.is_some();

        if !auth {
            if let Ok(addr) = self.grpc_addr.parse::<SocketAddr>() {
                if !addr.ip().is_loopback() {
                    issues.push(format!(
                        "grpc_addr ({addr}) is reachable from the network but no auth is \
                         configured — set auth_username + auth_password (or KEEPLIN_AUTH_*)"
                    ));
                }
            }
            if let Some(http) = &self.http_addr {
                if let Ok(addr) = http.parse::<SocketAddr>() {
                    if !addr.ip().is_loopback() {
                        issues.push(format!(
                            "http_addr ({addr}) is reachable from the network but no auth is \
                             configured — set auth_username + auth_password (or KEEPLIN_AUTH_*)"
                        ));
                    }
                }
            }
        }

        if matches!(self.mode, Mode::Server) {
            if let Some(host) = plaintext_ws_remote_host(&self.server_url) {
                issues.push(format!(
                    "server_url uses plaintext ws:// to a non-loopback host ({host}), leaking \
                     the auth_token in transit — use wss:// (TLS)"
                ));
            }
        }

        issues
    }
}

/// If `url` is a **plaintext** `ws://` URL pointing at a **non-loopback** host, return that
/// host; otherwise `None`. `wss://` (TLS), an empty URL, and loopback targets are all safe and
/// yield `None`. A host that cannot be confidently identified as loopback is treated as remote
/// (fail safe: better a spurious warning than a silent token leak).
fn plaintext_ws_remote_host(url: &str) -> Option<&str> {
    // wss:// is TLS-protected; only bare ws:// leaks the token.
    let rest = url.strip_prefix("ws://")?;
    // Strip any path/query: `ws://host:port/path` → `host:port`.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    // Drop an optional `port`, tolerating an IPv6 literal in `[…]`.
    let host = match authority.strip_prefix('[') {
        // `[::1]:9000` → `::1`
        Some(after) => after.split(']').next().unwrap_or(after),
        None => authority.rsplit_once(':').map_or(authority, |(h, _)| h),
    };
    let is_loopback = matches!(host, "localhost" | "127.0.0.1" | "::1")
        || host.starts_with("127.")
        || host.eq_ignore_ascii_case("ip6-localhost");
    if is_loopback {
        None
    } else {
        Some(host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal loopback config with the given tweaks applied by the caller.
    fn base() -> Config {
        Config::default()
    }

    fn with_auth(mut c: Config) -> Config {
        c.auth_username = Some("alice".into());
        c.auth_password = Some("s3cr3t".into());
        c
    }

    #[test]
    fn loopback_defaults_are_safe() {
        assert!(base().security_issues().is_empty());
    }

    #[test]
    fn network_grpc_without_auth_is_flagged() {
        let mut c = base();
        c.grpc_addr = "0.0.0.0:50051".into();
        let issues = c.security_issues();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("grpc_addr"));

        // Adding auth clears it.
        assert!(with_auth(c).security_issues().is_empty());
    }

    #[test]
    fn network_http_without_auth_is_flagged() {
        let mut c = base();
        c.http_addr = Some("0.0.0.0:50052".into());
        let issues = c.security_issues();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("http_addr"));
        assert!(with_auth(c).security_issues().is_empty());
    }

    #[test]
    fn plaintext_ws_to_remote_is_flagged_in_server_mode() {
        let mut c = with_auth(base()); // auth on, so only the ws:// issue can surface
        c.mode = Mode::Server;
        c.server_url = "ws://sync.example.com:9000/ws".into();
        let issues = c.security_issues();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("server_url"));

        // wss:// is safe.
        c.server_url = "wss://sync.example.com:9000/ws".into();
        assert!(c.security_issues().is_empty());

        // A loopback ws:// relay (local testing) is safe.
        c.server_url = "ws://127.0.0.1:9000/ws".into();
        assert!(c.security_issues().is_empty());

        // The same ws:// URL in offline mode is ignored (server_url is unused there).
        c.mode = Mode::Offline;
        c.server_url = "ws://sync.example.com:9000/ws".into();
        assert!(c.security_issues().is_empty());
    }

    #[test]
    fn plaintext_ws_remote_host_parsing() {
        // Remote ws:// → Some(host).
        assert_eq!(
            plaintext_ws_remote_host("ws://example.com:9000/ws"),
            Some("example.com")
        );
        assert_eq!(
            plaintext_ws_remote_host("ws://example.com"),
            Some("example.com")
        );
        // IPv6 literal, port stripped.
        assert_eq!(
            plaintext_ws_remote_host("ws://[2001:db8::1]:80/x"),
            Some("2001:db8::1")
        );
        // Safe cases → None.
        assert_eq!(plaintext_ws_remote_host("wss://example.com/ws"), None);
        assert_eq!(plaintext_ws_remote_host(""), None);
        assert_eq!(plaintext_ws_remote_host("ws://localhost:9000"), None);
        assert_eq!(plaintext_ws_remote_host("ws://127.0.0.1:9000"), None);
        assert_eq!(plaintext_ws_remote_host("ws://[::1]:9000"), None);
    }

    #[test]
    fn multiple_issues_accumulate() {
        let mut c = base();
        c.grpc_addr = "0.0.0.0:50051".into();
        c.http_addr = Some("0.0.0.0:50052".into());
        c.mode = Mode::Server;
        c.server_url = "ws://sync.example.com/ws".into();
        // No auth → grpc + http + ws all flagged.
        assert_eq!(c.security_issues().len(), 3, "{:?}", c.security_issues());
    }
}
