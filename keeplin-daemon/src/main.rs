mod config;
mod proto;
mod server;

use clap::Parser;
use keeplin_core::{
    encryption::EncryptedBackend,
    storage::{db::DbBackend, fs::FsBackend},
};
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

    // Environment variable takes precedence over the config file so the password
    // is never stored on disk in plaintext.
    if let Ok(pw) = std::env::var("KEEPLIN_ENCRYPTION_PASSWORD") {
        cfg.encryption_password = Some(pw);
    }

    let addr: std::net::SocketAddr = cfg.grpc_addr.parse()?;
    let encrypted = cfg.encryption_password.is_some();
    tracing::info!(mode = ?cfg.mode, %addr, encrypted, "Starting keeplin-daemon");

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
    let svc = KeeplinServiceServer::new(KeeplinServer::new(backend))
        .max_decoding_message_size(cfg.max_message_size)
        .max_encoding_message_size(cfg.max_message_size);

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
