mod config;
mod proto;
mod server;

use base64::{engine::general_purpose::STANDARD, Engine};
use clap::Parser;
use keeplin_core::{
    encryption::EncryptedBackend,
    storage::{db::DbBackend, fs::FsBackend},
};
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tracing_subscriber::EnvFilter;

use crate::{
    config::{Config, Mode},
    proto::keeplin::keeplin_service_server::KeeplinServiceServer,
    server::KeeplinServer,
};

#[derive(Parser, Debug)]
#[command(name = "keeplin-daemon", about = "Keeplin core daemon (gRPC)")]
struct Args {
    /// Path to the TOML configuration file.
    #[arg(short, long, default_value = "keeplin.toml")]
    config: std::path::PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("keeplin=info".parse()?))
        .init();

    let args = Args::parse();

    let mut cfg = if args.config.exists() {
        Config::from_file(&args.config)?
    } else {
        tracing::warn!(
            path = %args.config.display(),
            "Config file not found; using defaults"
        );
        Config::default()
    };

    // Environment variables take precedence over the config file so passwords
    // are never stored on disk in plaintext.
    if let Ok(pw) = std::env::var("KEEPLIN_ENCRYPTION_PASSWORD") {
        cfg.encryption_password = Some(pw);
    }
    if let Ok(pw) = std::env::var("KEEPLIN_AUTH_PASSWORD") {
        cfg.auth_password = Some(pw);
    }

    let addr: std::net::SocketAddr = cfg.grpc_addr.parse()?;

    // Warn loudly when the gRPC port is reachable from the network without auth.
    let auth_configured = cfg.auth_username.is_some() && cfg.auth_password.is_some();
    if !addr.ip().is_loopback() && !auth_configured {
        tracing::warn!(
            %addr,
            "gRPC is exposed to the network WITHOUT authentication. \
             Set auth_username + auth_password in keeplin.toml or KEEPLIN_AUTH_PASSWORD env var."
        );
    }

    let encrypted = cfg.encryption_password.is_some();
    tracing::info!(mode = ?cfg.mode, %addr, encrypted, auth = auth_configured, "Starting keeplin-daemon");

    match (cfg.mode.clone(), cfg.encryption_password.clone()) {
        (Mode::Offline, None) => {
            let backend = FsBackend::new(&cfg.data_dir).await?;
            tracing::info!(data_dir = %cfg.data_dir.display(), "Offline mode");
            run_server(&cfg, addr, backend).await?;
        }
        (Mode::Offline, Some(pw)) => {
            let backend = FsBackend::new(&cfg.data_dir).await?;
            let enc = EncryptedBackend::new(backend, &pw).await?;
            tracing::info!(data_dir = %cfg.data_dir.display(), "Offline mode (encrypted)");
            run_server(&cfg, addr, enc).await?;
        }
        (Mode::Server, None) => {
            let db_path = cfg.data_dir.join("keeplin.db");
            let backend = DbBackend::new(&db_path, &cfg.server_url, &cfg.auth_token).await?;
            tracing::info!(db = %db_path.display(), server = %cfg.server_url, "Server mode");
            run_server(&cfg, addr, backend).await?;
        }
        (Mode::Server, Some(pw)) => {
            let db_path = cfg.data_dir.join("keeplin.db");
            let backend = DbBackend::new(&db_path, &cfg.server_url, &cfg.auth_token).await?;
            let enc = EncryptedBackend::new(backend, &pw).await?;
            tracing::info!(db = %db_path.display(), server = %cfg.server_url, "Server mode (encrypted)");
            run_server(&cfg, addr, enc).await?;
        }
    }

    Ok(())
}

async fn run_server<B: keeplin_core::storage::StorageBackend>(
    cfg: &Config,
    addr: std::net::SocketAddr,
    backend: B,
) -> anyhow::Result<()> {
    let (auth_user, auth_pass) = (cfg.auth_username.clone(), cfg.auth_password.clone());

    let svc_inner = KeeplinServiceServer::new(KeeplinServer::new(backend))
        .max_decoding_message_size(cfg.max_message_size)
        .max_encoding_message_size(cfg.max_message_size);

    // Wrap every RPC with the same Basic-Auth interceptor regardless of mode.
    let svc = InterceptedService::new(svc_inner, move |req: tonic::Request<()>| {
        validate_basic_auth(req, auth_user.as_deref(), auth_pass.as_deref())
    });

    let mut builder = Server::builder();

    if let (Some(cert_path), Some(key_path)) = (&cfg.tls_cert_path, &cfg.tls_key_path) {
        let cert = tokio::fs::read(cert_path).await?;
        let key = tokio::fs::read(key_path).await?;
        let identity = Identity::from_pem(cert, key);
        builder = builder.tls_config(ServerTlsConfig::new().identity(identity))?;
        tracing::info!("TLS enabled");
    }

    tracing::info!(%addr, "gRPC server listening");
    builder
        .add_service(svc)
        .serve_with_shutdown(addr, async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Shutdown signal received, draining connections");
        })
        .await?;

    Ok(())
}

/// Validate an `Authorization: Basic <base64(user:pass)>` header on every RPC.
/// If credentials are not configured in the server, all calls are allowed through.
fn validate_basic_auth(
    req: tonic::Request<()>,
    expected_user: Option<&str>,
    expected_pass: Option<&str>,
) -> Result<tonic::Request<()>, tonic::Status> {
    let (Some(expected_user), Some(expected_pass)) = (expected_user, expected_pass) else {
        return Ok(req);
    };

    let auth = req
        .metadata()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let encoded = auth
        .strip_prefix("Basic ")
        .ok_or_else(|| tonic::Status::unauthenticated("authorization header missing or not Basic"))?;

    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| tonic::Status::unauthenticated("malformed authorization"))?;

    let creds = std::str::from_utf8(&decoded)
        .map_err(|_| tonic::Status::unauthenticated("malformed authorization"))?;

    let colon = creds
        .find(':')
        .ok_or_else(|| tonic::Status::unauthenticated("malformed authorization"))?;

    let (user, pass) = (&creds[..colon], &creds[colon + 1..]);

    if !ct_eq(user.as_bytes(), expected_user.as_bytes())
        || !ct_eq(pass.as_bytes(), expected_pass.as_bytes())
    {
        return Err(tonic::Status::unauthenticated("invalid credentials"));
    }

    Ok(req)
}

/// Constant-time byte-slice comparison to avoid timing side-channels.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_req(auth_header: Option<&str>) -> tonic::Request<()> {
        let mut req = tonic::Request::new(());
        if let Some(v) = auth_header {
            req.metadata_mut()
                .insert("authorization", v.parse().unwrap());
        }
        req
    }

    fn basic(user: &str, pass: &str) -> String {
        format!("Basic {}", STANDARD.encode(format!("{user}:{pass}")))
    }

    #[test]
    fn ct_eq_equal() {
        assert!(ct_eq(b"hello", b"hello"));
        assert!(ct_eq(b"", b""));
    }

    #[test]
    fn ct_eq_unequal() {
        assert!(!ct_eq(b"hello", b"world"));
        assert!(!ct_eq(b"abc", b"abcd"));
    }

    #[test]
    fn auth_not_configured_allows_all() {
        let req = make_req(None);
        assert!(validate_basic_auth(req, None, None).is_ok());
    }

    #[test]
    fn auth_valid_credentials_pass() {
        let req = make_req(Some(&basic("alice", "s3cr3t")));
        assert!(validate_basic_auth(req, Some("alice"), Some("s3cr3t")).is_ok());
    }

    #[test]
    fn auth_wrong_password_rejected() {
        let req = make_req(Some(&basic("alice", "wrong")));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn auth_wrong_user_rejected() {
        let req = make_req(Some(&basic("mallory", "s3cr3t")));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn auth_missing_header_rejected() {
        let req = make_req(None);
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn auth_bearer_scheme_rejected() {
        let req = make_req(Some("Bearer some-opaque-token"));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn auth_malformed_base64_rejected() {
        let req = make_req(Some("Basic !!!notbase64!!!"));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn auth_no_colon_in_credentials_rejected() {
        let req = make_req(Some(&format!("Basic {}", STANDARD.encode("nocolon"))));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn auth_password_containing_colon_works() {
        // Only the FIRST colon splits user from password.
        let pass = "p:a:s:s:word";
        let req = make_req(Some(&basic("alice", pass)));
        assert!(validate_basic_auth(req, Some("alice"), Some(pass)).is_ok());
    }
}
