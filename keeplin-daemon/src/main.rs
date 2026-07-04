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
mod metrics;
mod proto;
mod rest;
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

#[derive(Parser, Debug)]
#[command(name = "keeplin-daemon", about = "Keeplin core daemon (gRPC)")]
struct Args {
    /// Path to the TOML configuration file. The file is read once on startup;
    /// changes to the file while the daemon is running have no effect. If the
    /// file does not exist at startup, the daemon falls back to [`Config::default`]
    /// and logs a warning.
    #[arg(short, long, default_value = "keeplin.toml")]
    config: std::path::PathBuf,

    /// When present, run a one-off subcommand instead of starting the server.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Copy all data from one backend into another, then exit (does not start the server).
    ///
    /// Each side is described by its own config file, so this works for any combination of
    /// filesystem/server mode and plaintext/encrypted storage. It is a one-shot copy of the
    /// current live state into a fresh destination — see the "Migrating between backends"
    /// section of the README.
    Migrate {
        /// Config file describing the source backend to read from.
        #[arg(long)]
        from: std::path::PathBuf,
        /// Config file describing the destination backend to write to.
        #[arg(long)]
        to: std::path::PathBuf,
    },
}

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

/// Load a [`Config`] from `path` (falling back to defaults if the file is absent) and apply
/// the environment-variable overrides.
///
/// Overrides are applied after the TOML file is parsed so operators can keep sensitive
/// credentials out of the configuration file entirely — the file can be committed to version
/// control while secrets are injected at deploy time through environment variables.
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

/// Take the per-store daemon lock: an OS-level exclusive advisory lock on
/// `{data_dir}/.keeplin/daemon.lock`.
///
/// The decorator stack keeps in-process state (write serialisation, the alias index, the
/// live-change feed), so exactly **one daemon may serve a store at a time** — a second
/// process would silently violate the single-writer assumptions. The lock is advisory and
/// kernel-held: the returned handle must stay alive for the daemon's lifetime, and it is
/// released automatically on process exit (crashes included), so no stale lock can ever
/// block a restart. The lock file lives inside `.keeplin/` (per-device, excluded from
/// replication), and file *contents* are irrelevant — Syncthing copying an empty file
/// carries no lock.
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

/// Build the storage backend and run the gRPC (+ optional REST/WebSocket) server until
/// shutdown.
async fn serve(cfg: Config) -> anyhow::Result<()> {
    let addr: std::net::SocketAddr = cfg.grpc_addr.parse()?;

    // Refuse to start on a credential configuration that would silently disable auth (only one
    // of username/password set, or an empty string) instead of leaving the daemon unexpectedly
    // wide open (issue #73). A valid, fully-configured pair enables auth; both unset leaves it
    // intentionally off.
    if let Err(reason) = cfg.validate_auth() {
        anyhow::bail!("invalid authentication configuration: {reason}");
    }
    let auth_configured = cfg.auth_enabled();

    // One daemon per store, enforced before anything touches the data directory. Held
    // for the whole lifetime of `serve` (released by the OS on exit or crash).
    let _store_lock = acquire_store_lock(&cfg.data_dir)?;

    // Refuse to start in a configuration that would expose data or credentials on an untrusted
    // network (unauthenticated network listener, or a plaintext ws:// sync URL that leaks the
    // token). `insecure = true` downgrades these to warnings for deployments where another
    // layer provides the protection. Missing daemon-terminated TLS is intentionally not
    // flagged — fronting TLS at a reverse proxy is a supported deployment.
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

    // When encryption is enabled without an explicit key_salt, the key is derived from a
    // per-store salt persisted at `{data_dir}/.keeplin/key_salt` (device-id fallback on
    // first use — see `resolve_key_salt`). That is safe for a single device but means
    // another device cannot decrypt this device's data — encrypted multi-device sync
    // would silently produce unreadable records. Warn so operators who sync set a shared
    // key_salt on every device, and so everyone backs up the salt file.
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
                    run_server_with(&cfg, addr, collab, Some(starter)).await?;
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
                // Collab wraps the *decrypted* view: the line protocol needs
                // plaintext bodies (the collaborative server merges by line).
                Some(collab_cfg) => {
                    let collab = CollabBackend::new(enc, collab_cfg)?;
                    let starter = collab_starter(&collab);
                    run_server_with(&cfg, addr, collab, Some(starter)).await?;
                }
                None => run_server(&cfg, addr, enc).await?,
            }
        }
    }

    Ok(())
}

/// Resolves the Argon2id salt used to derive the at-rest encryption key.
///
/// Returns the configured `key_salt` bytes when set (the value that must be shared
/// across devices for portable encryption). When unset, the salt is read from — or on
/// first use derived from this device's ID and **persisted to** — the store's
/// `{data_dir}/.keeplin/key_salt` file.
///
/// Persisting the fallback matters for recovery: the salt is required (together with the
/// password) to derive the key, and before this file existed it lived only implicitly in
/// `.keeplin/device_id`. Losing that one file made encrypted data unrecoverable even with
/// the correct password. Now there is a single, explicitly named, plaintext-safe file the
/// user can back up — and restoring it into a fresh store is all a recovery needs. The
/// file also takes precedence over the device ID, so a store whose device-id file was
/// regenerated (or adopted by another machine) still decrypts.
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
            // An empty file cannot have been the salt of any real key; fall through and
            // re-persist the device-id fallback.
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

/// Where the effective encryption salt is persisted when `key_salt` is not configured:
/// `{data_dir}/.keeplin/key_salt`. The salt is not secret (see SECURITY.md), so a plain
/// file is fine; what matters is that it is explicit, stable, and easy to back up.
fn key_salt_path(cfg: &Config) -> std::path::PathBuf {
    cfg.data_dir.join(".keeplin").join("key_salt")
}

/// Build the **base** storage stack described by `cfg`, type-erased behind
/// `Arc<dyn StorageBackend>`.
///
/// This is the storage layer only — `FsBackend`/`DbBackend` plus an optional
/// [`EncryptedBackend`] — **without** the `LinkingBackend`/`EventBackend` decorators the
/// server adds. It is used by [`run_migrate`], which needs to hold two heterogeneous backends
/// at once and does not need link derivation or the live-change feed. The server path keeps
/// its own generic (monomorphised) construction in [`serve`].
async fn build_storage(cfg: &Config) -> anyhow::Result<Arc<dyn StorageBackend>> {
    // Ensure the data directory exists. `FsBackend::new` already does this, but
    // `DbBackend::new` opens the `.db` file directly and fails (SQLITE_CANTOPEN) if the
    // parent is missing — which is the common case for a fresh migration destination.
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

/// Copy all data from the backend described by `from` into the backend described by `to`.
///
/// Each side is built from its own config (so encryption keys, modes, and paths are
/// independent) and the copy runs through [`keeplin_core::migrate::migrate`], which uses the
/// typed `create_*` methods so the destination stores and re-indexes every entity natively.
/// This is a one-shot copy of the current live state into a fresh destination.
async fn run_migrate(from: &std::path::Path, to: &std::path::Path) -> anyhow::Result<()> {
    let from_cfg = load_config(from)?;
    let to_cfg = load_config(to)?;
    tracing::info!(
        from = %from.display(), from_mode = ?from_cfg.mode,
        to = %to.display(), to_mode = ?to_cfg.mode,
        "Starting migration",
    );

    // Migration must not race a live daemon on either side: take both store locks for
    // the duration of the copy (a running daemon on either store makes this fail fast).
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
/// Callback invoked with the fully-built decorator stack, used to hand the
/// collaborative client its top-of-stack handle (remote writes must flow
/// through linking + eventing exactly like local ones).
type StackHook = Box<dyn FnOnce(Arc<dyn keeplin_core::storage::StorageBackend>) + Send>;

/// Derive the collaborative channel settings from the daemon config.
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

/// Build the hook that starts the collab connection task once the stack Arc
/// exists.
fn collab_starter<B: keeplin_core::storage::StorageBackend>(
    collab: &CollabBackend<B>,
) -> StackHook {
    let collab = collab.clone();
    Box::new(move |top| {
        tokio::spawn(async move { collab.start(top).await });
    })
}

#[allow(clippy::result_large_err)]
async fn run_server<B: keeplin_core::storage::StorageBackend>(
    cfg: &Config,
    addr: std::net::SocketAddr,
    backend: B,
) -> anyhow::Result<()> {
    run_server_with(cfg, addr, backend, None).await
}

#[allow(clippy::result_large_err)]
async fn run_server_with<B: keeplin_core::storage::StorageBackend>(
    cfg: &Config,
    addr: std::net::SocketAddr,
    backend: B,
    stack_hook: Option<StackHook>,
) -> anyhow::Result<()> {
    // Decorator stack (innermost → outermost): the storage backend, then (optionally)
    // `EncryptedBackend` already applied by the caller, then `LinkingBackend` which derives
    // bookmarks/links from each plaintext note body and resolves references, then
    // `EventBackend` which publishes every mutation to the live-change broadcast channel, then
    // `MetricsBackend` which records each operation for `/api/metrics`.
    // `LinkingBackend` sits outside encryption so it parses plaintext bodies; `EventBackend`
    // sits outside it so the feed carries the refreshed metadata; `MetricsBackend` is outermost
    // so it counts logical operations as a client issues them.
    let backend = keeplin_core::linking::LinkingBackend::new(backend);
    let (events, _rx) = tokio::sync::broadcast::channel::<keeplin_core::models::Change>(1024);
    let backend = event_backend::EventBackend::new(backend, events.clone());
    let metrics = Arc::new(metrics::Metrics::new());
    let backend = Arc::new(metrics::MetricsBackend::new(backend, metrics.clone()));

    // The Inbox system notebook ("Pizarra", nil UUID) must exist before any request: new
    // notes without a notebook land in it. Idempotent on every startup.
    keeplin_core::ordering::ensure_inbox(backend.as_ref()).await?;

    // Hand the collaborative client the finished stack: remote writes flow
    // through linking/eventing/metrics exactly like local ones.
    if let Some(hook) = stack_hook {
        hook(backend.clone());
    }

    // One shared backend instance behind every surface: the gRPC service and (optionally)
    // the REST/HTTP server both hold a clone of this `Arc`.
    let (auth_user, auth_pass) = (cfg.auth_username.clone(), cfg.auth_password.clone());

    let svc_inner = KeeplinServiceServer::new(KeeplinServer::from_shared(
        backend.clone(),
        cfg.journal_retention_days,
        cfg.resource_purge_days,
        cfg.max_upload_bytes,
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
/// When both expected values are provided, the function extracts the `authorization`
/// metadata entry and hands it to [`auth::verify_basic`], which:
/// 1. Rejects empty expected credentials outright (an empty pair would accept `Basic Og==`).
/// 2. Parses the scheme per RFC 7617 / RFC 7235 — case-insensitive, any separating
///    whitespace — rather than requiring a literal `"Basic "` prefix, then Base64-decodes
///    the credential token.
/// 3. Splits the decoded value on the **first** colon to separate the username from the
///    password (passwords may themselves contain colons).
/// 4. Compares username and password using [`subtle::ConstantTimeEq`] to prevent
///    timing side-channels that could reveal the correct credential length.
///
/// A half-configured or empty credential pair never reaches this point: the daemon refuses
/// to start on it (see `Config::validate_auth`).
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
    use keeplin_core::storage::SyncBackend as _;

    #[test]
    fn store_lock_is_exclusive_per_store_and_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire_store_lock(dir.path()).unwrap();

        let err = acquire_store_lock(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("already running"),
            "second daemon on the same store must be refused: {err}"
        );

        // A different store is unaffected.
        let other = tempfile::tempdir().unwrap();
        let _other_lock = acquire_store_lock(other.path()).unwrap();

        // Releasing the lock (daemon exit) lets the next daemon in.
        drop(first);
        let _second = acquire_store_lock(dir.path()).unwrap();
    }

    /// A default (offline, unencrypted-config) `Config` rooted at `dir`.
    fn cfg_at(dir: &std::path::Path) -> Config {
        Config {
            data_dir: dir.to_path_buf(),
            ..Config::default()
        }
    }

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

    #[tokio::test]
    async fn key_salt_fallback_is_persisted_and_stable() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FsBackend::new(dir.path()).await.unwrap();
        let cfg = cfg_at(dir.path());

        // First resolution: falls back to the device id and writes the salt file.
        let first = resolve_key_salt(&cfg, &backend).await.unwrap();
        let device_id = backend.get_device_id().await.unwrap();
        assert_eq!(first, device_id.as_bytes());
        let on_disk = std::fs::read_to_string(key_salt_path(&cfg)).unwrap();
        assert_eq!(on_disk.trim().as_bytes(), first.as_slice());

        // Second resolution reads the file — same salt, so the key stays derivable.
        let second = resolve_key_salt(&cfg, &backend).await.unwrap();
        assert_eq!(second, first);
    }

    #[tokio::test]
    async fn key_salt_file_survives_a_regenerated_device_id() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FsBackend::new(dir.path()).await.unwrap();
        let cfg = cfg_at(dir.path());
        let original = resolve_key_salt(&cfg, &backend).await.unwrap();

        // Simulate the recovery scenario: the device-id file is lost and regenerated with
        // a fresh uuid. The persisted salt file must still win, keeping data decryptable.
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
