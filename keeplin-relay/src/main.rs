//! Entry point for `keeplin-relay` — the Keeplin server-mode WebSocket sync hub.
//!
//! Binds a TCP listener, logs configuration, and runs [`keeplin_relay::serve`] until Ctrl-C.
//! See the crate docs (`lib.rs`) and `README.md` for the wire protocol and deployment posture
//! (terminate TLS at a reverse proxy; point devices at `wss://`).

use clap::Parser;
use keeplin_relay::{serve, RelayConfig};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "keeplin-relay",
    about = "Keeplin server-mode WebSocket sync relay"
)]
struct Args {
    /// Address to listen on for device WebSocket connections.
    #[arg(long, default_value = "127.0.0.1:9000")]
    listen: String,

    /// Shared secret every device must present in its auth frame. Prefer the environment
    /// variable over a command line (which is visible in the process list). An empty value
    /// disables authentication — development only.
    #[arg(long, env = "KEEPLIN_RELAY_TOKEN", default_value = "")]
    auth_token: String,

    /// Directory for the durable frame buffer (the journal that lets an offline device
    /// catch up on reconnect). Created if missing.
    #[arg(
        long,
        env = "KEEPLIN_RELAY_DATA_DIR",
        default_value = "./keeplin-relay-data"
    )]
    data_dir: std::path::PathBuf,

    /// Days of buffered frames to keep before compaction drops them (0 = keep forever).
    /// Size this above the longest a device is expected to stay offline.
    #[arg(long, env = "KEEPLIN_RELAY_RETENTION_DAYS", default_value_t = 30)]
    retention_days: u64,

    /// Run without the durable buffer (pure in-memory broadcast): offline devices miss
    /// whatever is sent while they are away. Only for throwaway setups.
    #[arg(long)]
    ephemeral: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("keeplin_relay=info".parse()?))
        .init();

    let args = Args::parse();
    let addr: std::net::SocketAddr = args.listen.parse()?;

    if args.auth_token.is_empty() {
        tracing::warn!(
            "no auth token configured: the relay will accept ANY client. \
             Set --auth-token or KEEPLIN_RELAY_TOKEN before exposing it."
        );
    }
    // The relay speaks plain ws://; terminate TLS at a reverse proxy so the auth token and
    // change payloads are not sent in the clear across the network.
    if !addr.ip().is_loopback() {
        tracing::warn!(
            %addr,
            "listening on a non-loopback address in plaintext — front the relay with a \
             TLS-terminating reverse proxy and point devices at wss://"
        );
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "keeplin-relay listening");

    if args.ephemeral {
        tracing::warn!(
            "running --ephemeral: no durable buffer, offline devices miss frames sent \
             while they are away"
        );
    }

    serve(
        listener,
        RelayConfig {
            auth_token: args.auth_token,
            data_dir: (!args.ephemeral).then_some(args.data_dir),
            retention_days: args.retention_days,
        },
        shutdown_signal(),
    )
    .await
}

/// Resolves when the process receives a Ctrl-C (SIGINT).
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("Shutdown signal received, draining connections");
}
