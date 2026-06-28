//! Entry point for `keeplin-daemon` — the gRPC server that exposes the Keeplin
//! note-taking API over a network socket.
//!
//! This file wires together three sub-modules (`config`, `proto`, `server`),
//! selects the correct storage back-end based on the loaded [`Config`], optionally
//! wraps it with [`EncryptedBackend`] for at-rest encryption, attaches a Basic-Auth
//! interceptor to every incoming gRPC call, and then starts the tonic server.
//! Graceful shutdown is triggered by a CTRL-C (SIGINT) signal.

mod auth;
mod config;
mod event_backend;
mod proto;
mod rest;
mod server;

use std::sync::Arc;

use clap::Parser;
use keeplin_core::{
    encryption::EncryptedBackend,
    storage::{db::DbBackend, fs::FsBackend, StorageBackend},
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
    /// Path to the TOML configuration file. The file is read once on startup;
    /// changes to the file while the daemon is running have no effect. If the
    /// file does not exist at startup, the daemon falls back to [`Config::default`]
    /// and logs a warning.
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

    // Environment variable overrides are applied after the TOML file is parsed.
    // This allows operators to keep sensitive credentials out of the configuration
    // file entirely — the file can be committed to version control while secrets
    // are injected at deploy time through environment variables.
    if let Ok(pw) = std::env::var("KEEPLIN_ENCRYPTION_PASSWORD") {
        cfg.encryption_password = Some(pw);
    }
    if let Ok(salt) = std::env::var("KEEPLIN_KEY_SALT") {
        cfg.key_salt = Some(salt);
    }
    if let Ok(pw) = std::env::var("KEEPLIN_AUTH_PASSWORD") {
        cfg.auth_password = Some(pw);
    }
    if let Ok(user) = std::env::var("KEEPLIN_AUTH_USERNAME") {
        cfg.auth_username = Some(user);
    }

    let addr: std::net::SocketAddr = cfg.grpc_addr.parse()?;

    // Emit a warning when the gRPC port is bound to a non-loopback address but
    // no authentication credentials are configured. Without auth, any process on
    // the network can read, modify, or delete notes. The warning is loud by design.
    let auth_configured = cfg.auth_username.is_some() && cfg.auth_password.is_some();
    if !addr.ip().is_loopback() && !auth_configured {
        tracing::warn!(
            %addr,
            "gRPC is exposed to the network WITHOUT authentication. \
             Set auth_username + auth_password in keeplin.toml or KEEPLIN_AUTH_PASSWORD env var."
        );
    }
    // Same warning for the optional HTTP (REST/WebSocket) listener.
    if let Some(http) = &cfg.http_addr {
        if let Ok(http_addr) = http.parse::<std::net::SocketAddr>() {
            if !http_addr.ip().is_loopback() && !auth_configured {
                tracing::warn!(
                    %http_addr,
                    "HTTP (REST/WebSocket) is exposed to the network WITHOUT authentication."
                );
            }
        }
    }

    let encrypted = cfg.encryption_password.is_some();

    // When encryption is enabled without an explicit key_salt, the key is derived from
    // this device's ID. That is safe for a single device but means another device cannot
    // decrypt this device's data — encrypted multi-device sync would silently produce
    // unreadable records. Warn so operators who sync set a shared key_salt on every device.
    if encrypted && cfg.key_salt.is_none() {
        tracing::warn!(
            "encryption is enabled but key_salt is not set: encrypted data is bound to \
             this device and cannot be decrypted on other devices. Set the same key_salt \
             on all devices to enable encrypted multi-device sync."
        );
    }

    tracing::info!(mode = ?cfg.mode, %addr, encrypted, auth = auth_configured, "Starting keeplin-daemon");

    match (cfg.mode.clone(), cfg.encryption_password.clone()) {
        (Mode::Offline, None) => {
            let backend = FsBackend::new(&cfg.data_dir).await?;
            tracing::info!(data_dir = %cfg.data_dir.display(), "Offline mode");
            run_server(&cfg, addr, backend).await?;
        }
        (Mode::Offline, Some(pw)) => {
            let backend = FsBackend::new(&cfg.data_dir).await?;
            let salt = resolve_key_salt(&cfg, &backend).await?;
            let enc = EncryptedBackend::new(backend, &pw, &salt).await?;
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
            let salt = resolve_key_salt(&cfg, &backend).await?;
            let enc = EncryptedBackend::new(backend, &pw, &salt).await?;
            tracing::info!(db = %db_path.display(), server = %cfg.server_url, "Server mode (encrypted)");
            run_server(&cfg, addr, enc).await?;
        }
    }

    Ok(())
}

/// Resolves the Argon2id salt used to derive the at-rest encryption key.
///
/// Returns the configured `key_salt` bytes when set (the value that must be shared
/// across devices for portable encryption), otherwise falls back to this device's ID so
/// that single-device encrypted stores keep working without any configuration.
async fn resolve_key_salt<B: StorageBackend>(cfg: &Config, backend: &B) -> anyhow::Result<Vec<u8>> {
    match &cfg.key_salt {
        Some(salt) => Ok(salt.as_bytes().to_vec()),
        None => Ok(backend.get_device_id().await?.into_bytes()),
    }
}

/// Configure and start the tonic gRPC server with the given `backend`.
///
/// This function is generic over `B: StorageBackend` so the compiler generates a
/// separate, fully inlined version for each combination of storage mode and
/// encryption — avoiding runtime dispatch overhead. Steps performed:
///
/// 1. Build a `KeeplinServiceServer` from `KeeplinServer<B>` and apply message-size limits.
/// 2. Wrap the service with a `Basic-Auth` interceptor (a no-op when no credentials
///    are configured).
/// 3. Optionally load a TLS identity from PEM files and enable TLS on the server builder.
/// 4. Serve the service at `addr` and block until a CTRL-C signal arrives.
///
/// The `#[allow(clippy::result_large_err)]` attribute suppresses a Clippy warning that
/// arises because tonic's `tls_config` returns a large `Err` variant; the error is only
/// returned once during startup so heap allocation is not a concern here.
#[allow(clippy::result_large_err)]
async fn run_server<B: keeplin_core::storage::StorageBackend>(
    cfg: &Config,
    addr: std::net::SocketAddr,
    backend: B,
) -> anyhow::Result<()> {
    // Wrap the backend so that every successful mutation — from gRPC or REST — is published
    // to the live-change broadcast channel that WebSocket clients subscribe to. The
    // `EventBackend` sits outside any `EncryptedBackend`, so the changes it broadcasts carry
    // already-decrypted (plaintext) values for connected API clients.
    let (events, _rx) = tokio::sync::broadcast::channel::<keeplin_core::models::Change>(1024);
    let backend = Arc::new(event_backend::EventBackend::new(backend, events.clone()));

    // One shared backend instance behind every surface: the gRPC service and (optionally)
    // the REST/HTTP server both hold a clone of this `Arc`.
    let (auth_user, auth_pass) = (cfg.auth_username.clone(), cfg.auth_password.clone());

    let svc_inner = KeeplinServiceServer::new(KeeplinServer::from_shared(
        backend.clone(),
        cfg.journal_retention_days,
    ))
    .max_decoding_message_size(cfg.max_message_size)
    .max_encoding_message_size(cfg.max_message_size);

    // Wrap every RPC with the same Basic-Auth interceptor so authentication applies
    // uniformly to all methods regardless of the storage mode chosen. When neither
    // auth_username nor auth_password is set, the interceptor is a transparent no-op.
    let svc = InterceptedService::new(svc_inner, move |req: tonic::Request<()>| {
        validate_basic_auth(req, auth_user.as_deref(), auth_pass.as_deref())
    });

    let mut builder = Server::builder();
    if let (Some(cert_path), Some(key_path)) = (&cfg.tls_cert_path, &cfg.tls_key_path) {
        let cert = tokio::fs::read(cert_path).await?;
        let key = tokio::fs::read(key_path).await?;
        let identity = Identity::from_pem(cert, key);
        builder = builder.tls_config(ServerTlsConfig::new().identity(identity))?;
        tracing::info!("TLS enabled (gRPC)");
    }

    tracing::info!(%addr, "gRPC server listening");
    let grpc = builder
        .add_service(svc)
        .serve_with_shutdown(addr, shutdown_signal());

    // Optionally also serve the REST/JSON API (and, later, the WebSocket feed) on a
    // separate HTTP port, sharing the same backend and Basic-Auth credentials.
    if let Some(http_addr) = &cfg.http_addr {
        let http_addr: std::net::SocketAddr = http_addr.parse()?;
        let state = Arc::new(rest::AppState {
            backend: backend.clone(),
            events: events.clone(),
            auth_username: cfg.auth_username.clone(),
            auth_password: cfg.auth_password.clone(),
        });
        let app = rest::router(state);
        let listener = tokio::net::TcpListener::bind(http_addr).await?;
        tracing::info!(%http_addr, "HTTP (REST) server listening");
        let http = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal());

        // Run both servers; Ctrl-C drains both. If either exits with an error, abort.
        tokio::try_join!(
            async move { grpc.await.map_err(anyhow::Error::from) },
            async move { http.await.map_err(anyhow::Error::from) },
        )?;
    } else {
        grpc.await?;
    }

    Ok(())
}

/// Resolves when the process receives a Ctrl-C (SIGINT). Each server awaits its own copy;
/// on Unix every `ctrl_c()` future fires on the same signal, so both drain together.
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("Shutdown signal received, draining connections");
}

/// Validate an HTTP Basic Authentication header on an incoming gRPC request.
///
/// The expected wire format of the header is:
/// `Authorization: Basic <base64(username ":" password)>`
///
/// When `expected_user` and `expected_pass` are both `None` (authentication is not
/// configured), the function returns `Ok(req)` immediately and allows all callers
/// through without checking any header.
///
/// When both expected values are provided, the function:
/// 1. Extracts the `authorization` metadata entry from the request.
/// 2. Strips the `"Basic "` prefix to obtain the Base64-encoded credentials.
/// 3. Decodes the Base64 payload and splits on the **first** colon to separate the
///    username from the password. Passwords may themselves contain colons.
/// 4. Compares username and password using [`subtle::ConstantTimeEq`] to prevent
///    timing side-channels that could reveal the correct credential length.
///
/// Returns `Err(tonic::Status::unauthenticated(...))` for any malformed header or
/// wrong credentials. The specific rejection reason is intentionally terse to avoid
/// leaking information to an unauthenticated caller.
///
/// The `#[allow(clippy::result_large_err)]` attribute is required because
/// `tonic::Status` exceeds Clippy's default size threshold for `Err` variants.
#[allow(clippy::result_large_err)]
fn validate_basic_auth(
    req: tonic::Request<()>,
    expected_user: Option<&str>,
    expected_pass: Option<&str>,
) -> Result<tonic::Request<()>, tonic::Status> {
    let (Some(expected_user), Some(expected_pass)) = (expected_user, expected_pass) else {
        return Ok(req);
    };

    let header = req
        .metadata()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    if auth::verify_basic(header, expected_user, expected_pass) {
        Ok(req)
    } else {
        Err(tonic::Status::unauthenticated("invalid credentials"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};

    /// Build a bare tonic `Request<()>` and optionally attach an `authorization`
    /// metadata entry. The value string must already be in the correct wire format
    /// (e.g. `"Basic <base64>"`).
    fn make_req(auth_header: Option<&str>) -> tonic::Request<()> {
        let mut req = tonic::Request::new(());
        if let Some(v) = auth_header {
            req.metadata_mut()
                .insert("authorization", v.parse().unwrap());
        }
        req
    }

    /// Format a well-formed `Authorization: Basic` header value for the given
    /// username and password pair. The colon separator between the two values is
    /// included before Base64 encoding, matching RFC 7617.
    fn basic(user: &str, pass: &str) -> String {
        format!("Basic {}", STANDARD.encode(format!("{user}:{pass}")))
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
        // RFC 7617 requires splitting on the first colon only, so passwords that
        // themselves contain colons (a common practice) must be accepted without error.
        let pass = "p:a:s:s:word";
        let req = make_req(Some(&basic("alice", pass)));
        assert!(validate_basic_auth(req, Some("alice"), Some(pass)).is_ok());
    }
}
