mod config;
mod proto;
mod server;

use clap::Parser;
use keeplin_core::storage::{db::DbBackend, fs::FsBackend};
use tonic::transport::Server;
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

    let cfg = if args.config.exists() {
        Config::from_file(&args.config)?
    } else {
        tracing::warn!(
            path = %args.config.display(),
            "Config file not found; using defaults"
        );
        Config::default()
    };

    let addr: std::net::SocketAddr = cfg.grpc_addr.parse()?;
    tracing::info!(mode = ?cfg.mode, addr = %addr, "Starting keeplin-daemon");

    match cfg.mode {
        Mode::Offline => {
            let backend = FsBackend::new(&cfg.data_dir).await?;
            tracing::info!(data_dir = %cfg.data_dir.display(), "Offline mode");
            run_server(addr, backend).await?;
        }
        Mode::Server => {
            let db_path = cfg.data_dir.join("keeplin.db");
            let backend =
                DbBackend::new(&db_path, &cfg.server_url, &cfg.auth_token).await?;
            tracing::info!(
                db = %db_path.display(),
                server = %cfg.server_url,
                "Server mode"
            );
            run_server(addr, backend).await?;
        }
    }

    Ok(())
}

async fn run_server<B: keeplin_core::storage::StorageBackend>(
    addr: std::net::SocketAddr,
    backend: B,
) -> anyhow::Result<()> {
    let svc = KeeplinServiceServer::new(KeeplinServer::new(backend));
    tracing::info!(%addr, "gRPC server listening");
    Server::builder().add_service(svc).serve(addr).await?;
    Ok(())
}
