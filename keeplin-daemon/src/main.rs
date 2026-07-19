// md:Overview

mod auth;
mod config;
mod event_backend;
mod metrics;
mod proto;
mod rest;
mod search;
mod server;

use std::sync::Arc;

use clap::Parser;
use keeplin_core::collab::CollabBackend;
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

// md:Args
#[derive(Parser, Debug)]
#[command(name = "keeplin-daemon", about = "Keeplin core daemon (gRPC)")]
struct Args {
    #[arg(short, long, default_value = "keeplin.toml")]
    config: std::path::PathBuf,

    #[command(subcommand)]
    command: Option<Command>,
}

// md:Command
#[derive(clap::Subcommand, Debug)]
enum Command {
    Migrate {
        #[arg(long)]
        from: std::path::PathBuf,
        #[arg(long)]
        to: std::path::PathBuf,
    },
}

// md:fn main
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("keeplin=info".parse()?))
        .init();

    let args = Args::parse();

    match args.command {
        Some(Command::Migrate { from, to }) => run_migrate(&from, &to).await,
        None => serve(load_config(&args.config)?).await,
    }
}

// md:fn load_config
fn load_config(path: &std::path::Path) -> anyhow::Result<Config> {
    let mut cfg = if path.exists() {
        Config::from_file(path)?
    } else {
        tracing::warn!(path = %path.display(), "Config file not found; using defaults");
        Config::default()
    };

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
    Ok(cfg)
}

// md:fn acquire_store_lock
fn acquire_store_lock(data_dir: &std::path::Path) -> anyhow::Result<std::fs::File> {
    let dir = data_dir.join(".keeplin");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("daemon.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => anyhow::bail!(
            "another keeplin-daemon is already running against {} (held lock: {}). \
             Run exactly one daemon per store; a second instance would corrupt \
             in-process write serialisation.",
            data_dir.display(),
            path.display()
        ),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

// md:fn serve
async fn serve(cfg: Config) -> anyhow::Result<()> {
    let addr: std::net::SocketAddr = cfg.grpc_addr.parse()?;

    if let Err(reason) = cfg.validate_auth() {
        anyhow::bail!("invalid authentication configuration: {reason}");
    }
    let auth_configured = cfg.auth_enabled();

    let _store_lock = acquire_store_lock(&cfg.data_dir)?;

    let issues = cfg.security_issues();
    if !issues.is_empty() {
        if cfg.insecure {
            for issue in &issues {
                tracing::warn!("insecure = true, starting despite: {issue}");
            }
        } else {
            let list = issues
                .iter()
                .map(|i| format!("  - {i}"))
                .collect::<Vec<_>>()
                .join("\n");
            anyhow::bail!(
                "refusing to start — insecure configuration:\n{list}\n\
                 Fix the above, or set `insecure = true` if another layer (isolated network, \
                 mTLS, a proxy that also enforces auth) provides this protection."
            );
        }
    }

    let encrypted = cfg.encryption_password.is_some();

    if encrypted && cfg.key_salt.is_none() {
        tracing::warn!(
            path = %key_salt_path(&cfg).display(),
            "encryption is enabled but key_salt is not set: encrypted data is bound to \
             this store's persisted salt file and cannot be decrypted on other devices. \
             Back up that file (it is required, with your password, to recover the data), \
             or set the same key_salt on all devices for encrypted multi-device sync."
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
            match collab_config(&cfg) {
                Some(collab_cfg) => {
                    let collab = CollabBackend::new(backend, collab_cfg)?;
                    let starter = collab_starter(&collab);
                    let handle = collab.handle();
                    run_server_with(&cfg, addr, collab, Some(starter), Some(handle)).await?;
                }
                None => run_server(&cfg, addr, backend).await?,
            }
        }
        (Mode::Server, Some(pw)) => {
            let db_path = cfg.data_dir.join("keeplin.db");
            let backend = DbBackend::new(&db_path, &cfg.server_url, &cfg.auth_token).await?;
            let salt = resolve_key_salt(&cfg, &backend).await?;
            let enc = EncryptedBackend::new(backend, &pw, &salt).await?;
            tracing::info!(db = %db_path.display(), server = %cfg.server_url, "Server mode (encrypted)");
            match collab_config(&cfg) {
                Some(collab_cfg) => {
                    let collab = CollabBackend::new(enc, collab_cfg)?;
                    let starter = collab_starter(&collab);
                    let handle = collab.handle();
                    run_server_with(&cfg, addr, collab, Some(starter), Some(handle)).await?;
                }
                None => run_server(&cfg, addr, enc).await?,
            }
        }
    }

    Ok(())
}

// md:fn resolve_key_salt
async fn resolve_key_salt<B: StorageBackend>(cfg: &Config, backend: &B) -> anyhow::Result<Vec<u8>> {
    if let Some(salt) = &cfg.key_salt {
        return Ok(salt.as_bytes().to_vec());
    }
    let path = key_salt_path(cfg);
    match tokio::fs::read_to_string(&path).await {
        Ok(persisted) => {
            let persisted = persisted.trim();
            if !persisted.is_empty() {
                return Ok(persisted.as_bytes().to_vec());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let salt = backend.get_device_id().await?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, &salt).await?;
    tracing::warn!(
        path = %path.display(),
        "encryption key salt persisted (device-id fallback); BACK UP this file — without \
         it (or its value in `key_salt`) and your password, the encrypted data cannot be \
         recovered"
    );
    Ok(salt.into_bytes())
}

// md:fn key_salt_path
fn key_salt_path(cfg: &Config) -> std::path::PathBuf {
    cfg.data_dir.join(".keeplin").join("key_salt")
}

// md:fn build_storage
async fn build_storage(cfg: &Config) -> anyhow::Result<Arc<dyn StorageBackend>> {
    tokio::fs::create_dir_all(&cfg.data_dir).await?;
    Ok(match (cfg.mode.clone(), cfg.encryption_password.clone()) {
        (Mode::Offline, None) => Arc::new(FsBackend::new(&cfg.data_dir).await?) as _,
        (Mode::Offline, Some(pw)) => {
            let backend = FsBackend::new(&cfg.data_dir).await?;
            let salt = resolve_key_salt(cfg, &backend).await?;
            Arc::new(EncryptedBackend::new(backend, &pw, &salt).await?) as _
        }
        (Mode::Server, None) => {
            let db_path = cfg.data_dir.join("keeplin.db");
            Arc::new(DbBackend::new(&db_path, &cfg.server_url, &cfg.auth_token).await?) as _
        }
        (Mode::Server, Some(pw)) => {
            let db_path = cfg.data_dir.join("keeplin.db");
            let backend = DbBackend::new(&db_path, &cfg.server_url, &cfg.auth_token).await?;
            let salt = resolve_key_salt(cfg, &backend).await?;
            Arc::new(EncryptedBackend::new(backend, &pw, &salt).await?) as _
        }
    })
}

// md:fn run_migrate
async fn run_migrate(from: &std::path::Path, to: &std::path::Path) -> anyhow::Result<()> {
    let from_cfg = load_config(from)?;
    let to_cfg = load_config(to)?;
    tracing::info!(
        from = %from.display(), from_mode = ?from_cfg.mode,
        to = %to.display(), to_mode = ?to_cfg.mode,
        "Starting migration",
    );

    let _from_lock = acquire_store_lock(&from_cfg.data_dir)?;
    let _to_lock = acquire_store_lock(&to_cfg.data_dir)?;

    let src = build_storage(&from_cfg).await?;
    let dst = build_storage(&to_cfg).await?;

    let report = keeplin_core::migrate::migrate(src.as_ref(), dst.as_ref()).await?;
    tracing::info!(
        notebooks = report.notebooks,
        tags = report.tags,
        notes = report.notes,
        note_tags = report.note_tags,
        resources = report.resources,
        "Migration complete",
    );
    println!(
        "Migration complete: {} notebooks, {} tags, {} notes, {} note-tags, {} resources",
        report.notebooks, report.tags, report.notes, report.note_tags, report.resources,
    );
    Ok(())
}

// md:StackHook
type StackHook = Box<dyn FnOnce(Arc<dyn keeplin_core::storage::StorageBackend>) + Send>;

// md:fn collab_config
fn collab_config(cfg: &Config) -> Option<keeplin_core::collab::CollabConfig> {
    let api_url = cfg.collab_api_url.clone()?;
    let ws_url = format!(
        "{}/api/ws",
        api_url
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1)
    );
    Some(keeplin_core::collab::CollabConfig {
        api_url,
        ws_url,
        token: cfg.auth_token.clone(),
    })
}

// md:fn collab_starter
fn collab_starter<B: keeplin_core::storage::StorageBackend>(
    collab: &CollabBackend<B>,
) -> StackHook {
    let collab = collab.clone();
    Box::new(move |top| {
        tokio::spawn(async move {
            if let Err(e) = collab.start(top).await {
                tracing::error!(error = %e, "collaborative channel disabled");
            }
        });
    })
}

// md:fn run_server
#[allow(clippy::result_large_err)]
async fn run_server<B: keeplin_core::storage::StorageBackend>(
    cfg: &Config,
    addr: std::net::SocketAddr,
    backend: B,
) -> anyhow::Result<()> {
    run_server_with(cfg, addr, backend, None, None).await
}

// md:fn run_server_with
#[allow(clippy::result_large_err)]
async fn run_server_with<B: keeplin_core::storage::StorageBackend>(
    cfg: &Config,
    addr: std::net::SocketAddr,
    backend: B,
    stack_hook: Option<StackHook>,
    collab_handle: Option<keeplin_core::collab::CollabHandle>,
) -> anyhow::Result<()> {
    let backend = keeplin_core::linking::LinkingBackend::new(backend);
    let (events, _rx) = tokio::sync::broadcast::channel::<keeplin_core::models::Change>(1024);
    let backend = event_backend::EventBackend::new(backend, events.clone());
    let metrics = Arc::new(metrics::Metrics::new());
    let backend = Arc::new(metrics::MetricsBackend::new(backend, metrics.clone()));

    keeplin_core::ordering::ensure_inbox(backend.as_ref()).await?;

    if let Some(hook) = stack_hook {
        hook(backend.clone());
    }

    if cfg.sync_interval_secs > 0 {
        let backend = backend.clone();
        let period = std::time::Duration::from_secs(cfg.sync_interval_secs);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(period);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if let Err(e) = keeplin_core::sync::run_sync(backend.as_ref(), |_, _| {}).await {
                    tracing::warn!(error = %e, "background sync cycle failed");
                }
            }
        });
    }

    let (auth_user, auth_pass) = (cfg.auth_username.clone(), cfg.auth_password.clone());

    let svc_inner = KeeplinServiceServer::new(KeeplinServer::from_shared(
        backend.clone(),
        cfg.journal_retention_days,
        cfg.resource_purge_days,
        cfg.max_upload_bytes,
    ))
    .max_decoding_message_size(cfg.max_message_size)
    .max_encoding_message_size(cfg.max_message_size);

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

    if let Some(http_addr) = &cfg.http_addr {
        let http_addr: std::net::SocketAddr = http_addr.parse()?;
        let search = search::start(backend.clone(), events.clone()).await;
        let state = Arc::new(rest::AppState {
            backend: backend.clone(),
            collab: collab_handle.clone(),
            search,
            events: events.clone(),
            metrics: metrics.clone(),
            max_body_bytes: cfg.max_message_size,
            max_upload_bytes: cfg.max_upload_bytes,
            journal_retention_days: cfg.journal_retention_days,
            resource_purge_days: cfg.resource_purge_days,
            auth_username: cfg.auth_username.clone(),
            auth_password: cfg.auth_password.clone(),
        });
        let app = rest::router(state);
        let listener = tokio::net::TcpListener::bind(http_addr).await?;
        tracing::info!(%http_addr, "HTTP (REST) server listening");
        let http = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal());

        tokio::try_join!(
            async move { grpc.await.map_err(anyhow::Error::from) },
            async move { http.await.map_err(anyhow::Error::from) },
        )?;
    } else {
        grpc.await?;
    }

    Ok(())
}

// md:fn shutdown_signal
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("Shutdown signal received, draining connections");
}

// md:fn validate_basic_auth
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

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use keeplin_core::storage::SyncBackend as _;

    // md:mod tests > fn store_lock_is_exclusive_per_store_and_released_on_drop
    #[test]
    fn store_lock_is_exclusive_per_store_and_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire_store_lock(dir.path()).unwrap();

        let err = acquire_store_lock(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("already running"),
            "second daemon on the same store must be refused: {err}"
        );

        let other = tempfile::tempdir().unwrap();
        let _other_lock = acquire_store_lock(other.path()).unwrap();

        drop(first);
        let _second = acquire_store_lock(dir.path()).unwrap();
    }

    // md:mod tests > fn cfg_at
    fn cfg_at(dir: &std::path::Path) -> Config {
        Config {
            data_dir: dir.to_path_buf(),
            ..Config::default()
        }
    }

    // md:mod tests > fn key_salt_config_value_wins_and_persists_nothing
    #[tokio::test]
    async fn key_salt_config_value_wins_and_persists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FsBackend::new(dir.path()).await.unwrap();
        let mut cfg = cfg_at(dir.path());
        cfg.key_salt = Some("shared-salt".into());

        let salt = resolve_key_salt(&cfg, &backend).await.unwrap();
        assert_eq!(salt, b"shared-salt");
        assert!(
            !key_salt_path(&cfg).exists(),
            "a configured salt must not be shadowed by a persisted file"
        );
    }

    // md:mod tests > fn key_salt_fallback_is_persisted_and_stable
    #[tokio::test]
    async fn key_salt_fallback_is_persisted_and_stable() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FsBackend::new(dir.path()).await.unwrap();
        let cfg = cfg_at(dir.path());

        let first = resolve_key_salt(&cfg, &backend).await.unwrap();
        let device_id = backend.get_device_id().await.unwrap();
        assert_eq!(first, device_id.as_bytes());
        let on_disk = std::fs::read_to_string(key_salt_path(&cfg)).unwrap();
        assert_eq!(on_disk.trim().as_bytes(), first.as_slice());

        let second = resolve_key_salt(&cfg, &backend).await.unwrap();
        assert_eq!(second, first);
    }

    // md:mod tests > fn key_salt_file_survives_a_regenerated_device_id
    #[tokio::test]
    async fn key_salt_file_survives_a_regenerated_device_id() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FsBackend::new(dir.path()).await.unwrap();
        let cfg = cfg_at(dir.path());
        let original = resolve_key_salt(&cfg, &backend).await.unwrap();

        std::fs::remove_file(dir.path().join(".keeplin").join("device_id")).unwrap();
        let reopened = FsBackend::new(dir.path()).await.unwrap();
        assert_ne!(
            reopened.get_device_id().await.unwrap().as_bytes(),
            original.as_slice(),
            "precondition: the regenerated device id differs"
        );
        let resolved = resolve_key_salt(&cfg, &reopened).await.unwrap();
        assert_eq!(resolved, original, "salt must come from the persisted file");
    }

    // md:mod tests > fn make_req
    fn make_req(auth_header: Option<&str>) -> tonic::Request<()> {
        let mut req = tonic::Request::new(());
        if let Some(v) = auth_header {
            req.metadata_mut()
                .insert("authorization", v.parse().unwrap());
        }
        req
    }

    // md:mod tests > fn basic
    fn basic(user: &str, pass: &str) -> String {
        format!("Basic {}", STANDARD.encode(format!("{user}:{pass}")))
    }

    // md:mod tests > fn auth_not_configured_allows_all
    #[test]
    fn auth_not_configured_allows_all() {
        let req = make_req(None);
        assert!(validate_basic_auth(req, None, None).is_ok());
    }

    // md:mod tests > fn auth_valid_credentials_pass
    #[test]
    fn auth_valid_credentials_pass() {
        let req = make_req(Some(&basic("alice", "s3cr3t")));
        assert!(validate_basic_auth(req, Some("alice"), Some("s3cr3t")).is_ok());
    }

    // md:mod tests > fn auth_wrong_password_rejected
    #[test]
    fn auth_wrong_password_rejected() {
        let req = make_req(Some(&basic("alice", "wrong")));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    // md:mod tests > fn auth_wrong_user_rejected
    #[test]
    fn auth_wrong_user_rejected() {
        let req = make_req(Some(&basic("mallory", "s3cr3t")));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    // md:mod tests > fn auth_missing_header_rejected
    #[test]
    fn auth_missing_header_rejected() {
        let req = make_req(None);
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    // md:mod tests > fn auth_bearer_scheme_rejected
    #[test]
    fn auth_bearer_scheme_rejected() {
        let req = make_req(Some("Bearer some-opaque-token"));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    // md:mod tests > fn auth_malformed_base64_rejected
    #[test]
    fn auth_malformed_base64_rejected() {
        let req = make_req(Some("Basic !!!notbase64!!!"));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    // md:mod tests > fn auth_no_colon_in_credentials_rejected
    #[test]
    fn auth_no_colon_in_credentials_rejected() {
        let req = make_req(Some(&format!("Basic {}", STANDARD.encode("nocolon"))));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    // md:mod tests > fn auth_password_containing_colon_works
    #[test]
    fn auth_password_containing_colon_works() {
        let pass = "p:a:s:s:word";
        let req = make_req(Some(&basic("alice", pass)));
        assert!(validate_basic_auth(req, Some("alice"), Some(pass)).is_ok());
    }
}
