// md:Overview
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

// md:Mode
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Offline,
    Server,
}

// md:Config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub mode: Mode,

    pub data_dir: PathBuf,

    #[serde(default)]
    pub server_url: String,

    #[serde(default)]
    pub auth_token: String,

    #[serde(default)]
    pub collab_api_url: Option<String>,

    #[serde(default = "default_grpc_addr")]
    pub grpc_addr: String,

    #[serde(default)]
    pub http_addr: Option<String>,

    #[serde(default)]
    pub tls_cert_path: Option<String>,

    #[serde(default)]
    pub tls_key_path: Option<String>,

    #[serde(default = "default_max_message_size")]
    pub max_message_size: usize,

    #[serde(default = "default_max_upload_bytes")]
    pub max_upload_bytes: usize,

    #[serde(default = "default_journal_retention_days")]
    pub journal_retention_days: u64,

    #[serde(default)]
    pub resource_purge_days: u64,

    #[serde(default)]
    pub sync_interval_secs: u64,

    #[serde(default)]
    pub encryption_password: Option<String>,

    #[serde(default)]
    pub key_salt: Option<String>,

    #[serde(default)]
    pub auth_username: Option<String>,

    #[serde(default)]
    pub auth_password: Option<String>,

    #[serde(default)]
    pub insecure: bool,
}

// md:fn default_grpc_addr
fn default_grpc_addr() -> String {
    "127.0.0.1:50051".to_string()
}

// md:fn default_max_message_size
fn default_max_message_size() -> usize {
    32 * 1024 * 1024
}

// md:fn default_max_upload_bytes
fn default_max_upload_bytes() -> usize {
    1024 * 1024 * 1024
}

// md:fn default_journal_retention_days
fn default_journal_retention_days() -> u64 {
    30
}

// md:impl Config (loading)
impl Config {
    // md:impl Config (loading) > fn from_file
    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&raw)?;
        Ok(cfg)
    }
}

// md:impl Default for Config
impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Offline,
            data_dir: PathBuf::from("./keeplin-data"),
            server_url: String::new(),
            auth_token: String::new(),
            collab_api_url: None,
            grpc_addr: default_grpc_addr(),
            http_addr: None,
            tls_cert_path: None,
            tls_key_path: None,
            max_message_size: default_max_message_size(),
            max_upload_bytes: default_max_upload_bytes(),
            journal_retention_days: default_journal_retention_days(),
            resource_purge_days: 0,
            sync_interval_secs: 0,
            encryption_password: None,
            key_salt: None,
            auth_username: None,
            auth_password: None,
            insecure: false,
        }
    }
}

// md:impl Config (security)
impl Config {
    // md:impl Config (security) > fn security_issues
    pub fn security_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        let auth = self.auth_enabled();

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

    // md:impl Config (security) > fn auth_enabled
    pub fn auth_enabled(&self) -> bool {
        matches!(
            (self.auth_username.as_deref(), self.auth_password.as_deref()),
            (Some(u), Some(p)) if !u.is_empty() && !p.is_empty()
        )
    }

    // md:impl Config (security) > fn validate_auth
    pub fn validate_auth(&self) -> Result<(), String> {
        match (self.auth_username.as_deref(), self.auth_password.as_deref()) {
            (None, None) => Ok(()),
            (Some(_), None) => Err(
                "auth_username is set but auth_password is not — set both (or neither); \
                 prefer KEEPLIN_AUTH_USERNAME + KEEPLIN_AUTH_PASSWORD"
                    .into(),
            ),
            (None, Some(_)) => Err(
                "auth_password is set but auth_username is not — set both (or neither); \
                 prefer KEEPLIN_AUTH_USERNAME + KEEPLIN_AUTH_PASSWORD"
                    .into(),
            ),
            (Some(u), Some(p)) => {
                if u.is_empty() || p.is_empty() {
                    Err(
                        "auth_username and auth_password must both be non-empty when \
                         authentication is configured"
                            .into(),
                    )
                } else {
                    Ok(())
                }
            }
        }
    }
}

// md:fn plaintext_ws_remote_host
fn plaintext_ws_remote_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("ws://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = match authority.strip_prefix('[') {
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

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;

    // md:mod tests > fn base
    fn base() -> Config {
        Config::default()
    }

    // md:mod tests > fn with_auth
    fn with_auth(mut c: Config) -> Config {
        c.auth_username = Some("alice".into());
        c.auth_password = Some("s3cr3t".into());
        c
    }

    // md:mod tests > fn loopback_defaults_are_safe
    #[test]
    fn loopback_defaults_are_safe() {
        assert!(base().security_issues().is_empty());
    }

    // md:mod tests > fn network_grpc_without_auth_is_flagged
    #[test]
    fn network_grpc_without_auth_is_flagged() {
        let mut c = base();
        c.grpc_addr = "0.0.0.0:50051".into();
        let issues = c.security_issues();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("grpc_addr"));

        assert!(with_auth(c).security_issues().is_empty());
    }

    // md:mod tests > fn network_http_without_auth_is_flagged
    #[test]
    fn network_http_without_auth_is_flagged() {
        let mut c = base();
        c.http_addr = Some("0.0.0.0:50052".into());
        let issues = c.security_issues();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("http_addr"));
        assert!(with_auth(c).security_issues().is_empty());
    }

    // md:mod tests > fn validate_auth_rejects_partial_and_empty_credentials
    #[test]
    fn validate_auth_rejects_partial_and_empty_credentials() {
        let mut c = base();
        assert!(c.validate_auth().is_ok());
        assert!(!c.auth_enabled());

        c.auth_username = Some("alice".into());
        assert!(c.validate_auth().is_err());
        assert!(!c.auth_enabled());

        c.auth_username = None;
        c.auth_password = Some("s3cr3t".into());
        assert!(c.validate_auth().is_err());
        assert!(!c.auth_enabled());

        c.auth_username = Some(String::new());
        c.auth_password = Some(String::new());
        assert!(c.validate_auth().is_err());
        assert!(!c.auth_enabled());

        c.auth_username = Some("alice".into());
        c.auth_password = Some(String::new());
        assert!(c.validate_auth().is_err());
        assert!(!c.auth_enabled());

        c.auth_password = Some("s3cr3t".into());
        assert!(c.validate_auth().is_ok());
        assert!(c.auth_enabled());
    }

    // md:mod tests > fn partial_auth_still_flags_network_exposure
    #[test]
    fn partial_auth_still_flags_network_exposure() {
        let mut c = base();
        c.grpc_addr = "0.0.0.0:50051".into();
        c.auth_password = Some("s3cr3t".into());
        let issues = c.security_issues();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("grpc_addr"));
    }

    // md:mod tests > fn plaintext_ws_to_remote_is_flagged_in_server_mode
    #[test]
    fn plaintext_ws_to_remote_is_flagged_in_server_mode() {
        let mut c = with_auth(base());
        c.mode = Mode::Server;
        c.server_url = "ws://sync.example.com:9000/ws".into();
        let issues = c.security_issues();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("server_url"));

        c.server_url = "wss://sync.example.com:9000/ws".into();
        assert!(c.security_issues().is_empty());

        c.server_url = "ws://127.0.0.1:9000/ws".into();
        assert!(c.security_issues().is_empty());

        c.mode = Mode::Offline;
        c.server_url = "ws://sync.example.com:9000/ws".into();
        assert!(c.security_issues().is_empty());
    }

    // md:mod tests > fn plaintext_ws_remote_host_parsing
    #[test]
    fn plaintext_ws_remote_host_parsing() {
        assert_eq!(
            plaintext_ws_remote_host("ws://example.com:9000/ws"),
            Some("example.com")
        );
        assert_eq!(
            plaintext_ws_remote_host("ws://example.com"),
            Some("example.com")
        );
        assert_eq!(
            plaintext_ws_remote_host("ws://[2001:db8::1]:80/x"),
            Some("2001:db8::1")
        );
        assert_eq!(plaintext_ws_remote_host("wss://example.com/ws"), None);
        assert_eq!(plaintext_ws_remote_host(""), None);
        assert_eq!(plaintext_ws_remote_host("ws://localhost:9000"), None);
        assert_eq!(plaintext_ws_remote_host("ws://127.0.0.1:9000"), None);
        assert_eq!(plaintext_ws_remote_host("ws://[::1]:9000"), None);
    }

    // md:mod tests > fn multiple_issues_accumulate
    #[test]
    fn multiple_issues_accumulate() {
        let mut c = base();
        c.grpc_addr = "0.0.0.0:50051".into();
        c.http_addr = Some("0.0.0.0:50052".into());
        c.mode = Mode::Server;
        c.server_url = "ws://sync.example.com/ws".into();
        assert_eq!(c.security_issues().len(), 3, "{:?}", c.security_issues());
    }
}
