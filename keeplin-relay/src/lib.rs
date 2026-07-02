//! `keeplin-relay` — a minimal production WebSocket sync hub for Keeplin **server mode**.
//!
//! In server mode each device's `DbBackend` connects to a relay over WebSocket, pushes its
//! local changes, and receives the changes other devices pushed. The relay is a **broadcast
//! hub**: it authenticates each connection, then forwards every `changes` frame a device sends
//! to **all other** connected devices — never echoing it back to the sender. It keeps **no
//! persistent state**, so a device that is offline misses whatever was broadcast while it was
//! gone and catches up once both peers are online again; because every change is idempotent and
//! version-vector resolved, replaying them converges. (Durable per-device buffering for
//! long-offline devices is a deliberate non-goal here — see the crate README.)
//!
//! This is the shippable counterpart to the in-process relay in
//! `keeplin-core/tests/ws_sync.rs`, which the client already speaks to: same wire protocol,
//! plus a real auth check, configuration, TLS-at-a-proxy posture, and graceful shutdown.
//!
//! # Wire protocol
//!
//! Matching `DbBackend::connect_ws` / `send_changes` / `receive_changes`:
//! 1. The client's **first** text frame is the auth handshake: `{"type":"auth","token":"…"}`.
//!    The relay validates the token (constant-time) against its configured token and closes the
//!    connection on mismatch.
//! 2. Every **subsequent** text frame (a `{"type":"changes",…}` batch) is forwarded verbatim to
//!    all other authenticated clients.
//!
//! The relay does not parse or trust the `changes` payloads — it only moves bytes between
//! devices; conflict resolution and encryption remain entirely in the clients.
//!
//! # TLS
//!
//! The relay speaks plain `ws://`. Terminate TLS at a reverse proxy (nginx, Caddy, …) and point
//! devices at `wss://your-proxy`, exactly as the daemon's REST/token guidance recommends — the
//! daemon refuses a non-loopback `ws://` `server_url` for that reason.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use subtle::ConstantTimeEq;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

/// How many pending broadcast frames a slow client may fall behind before it is dropped from
/// the channel (a `Lagged` error). Sized generously; a client that lags is disconnected and its
/// device re-syncs from scratch on reconnect (all changes are idempotent).
const BROADCAST_CAPACITY: usize = 1024;

/// Relay configuration.
pub struct RelayConfig {
    /// Shared secret every client must present in its first frame. An **empty** token disables
    /// authentication (development only) — the caller should warn loudly in that case.
    pub auth_token: String,
}

/// One frame to fan out, tagged with the id of the connection that sent it so the forwarder can
/// skip echoing it back to its origin.
type Broadcast = (u64, String);

/// Run the relay on an already-bound `listener` until `shutdown` resolves.
///
/// Taking a bound `TcpListener` (rather than an address) lets tests bind an ephemeral port and
/// hand it straight in. Each accepted connection is served on its own task; `shutdown` (e.g. a
/// Ctrl-C future) ends the accept loop and the function returns, dropping the broadcast sender
/// so in-flight connections wind down.
pub async fn serve(
    listener: TcpListener,
    config: RelayConfig,
    shutdown: impl std::future::Future<Output = ()>,
) -> anyhow::Result<()> {
    let (tx, _rx) = broadcast::channel::<Broadcast>(BROADCAST_CAPACITY);
    let config = Arc::new(config);

    tokio::select! {
        _ = accept_loop(listener, config, tx) => {}
        _ = shutdown => {
            tracing::info!("Shutdown signal received, stopping relay");
        }
    }
    Ok(())
}

/// Accept connections forever, spawning [`handle_connection`] for each.
async fn accept_loop(
    listener: TcpListener,
    config: Arc<RelayConfig>,
    tx: broadcast::Sender<Broadcast>,
) {
    let next_id = Arc::new(AtomicU64::new(0));
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let id = next_id.fetch_add(1, Ordering::Relaxed);
                let config = Arc::clone(&config);
                let tx = tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, id, config, tx).await {
                        tracing::debug!(%peer, conn = id, "connection ended: {e}");
                    }
                });
            }
            Err(e) => {
                tracing::warn!("accept failed: {e}");
            }
        }
    }
}

/// Validate `token` against the configured one in constant time. An empty configured token
/// accepts any client (development mode).
fn token_ok(configured: &str, presented: &str) -> bool {
    if configured.is_empty() {
        return true;
    }
    configured
        .as_bytes()
        .ct_eq(presented.as_bytes())
        .unwrap_u8()
        == 1
}

/// Extract the token from an auth frame `{"type":"auth","token":"…"}`. Returns `None` when the
/// frame is not a well-formed auth message.
fn parse_auth_token(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("auth") {
        return None;
    }
    Some(v.get("token")?.as_str()?.to_string())
}

/// Serve one client: perform the auth handshake, then bridge it onto the broadcast bus —
/// forwarding others' frames to it and its frames to everyone else.
async fn handle_connection(
    stream: TcpStream,
    id: u64,
    config: Arc<RelayConfig>,
    tx: broadcast::Sender<Broadcast>,
) -> anyhow::Result<()> {
    let ws = tokio_tungstenite::accept_async(stream).await?;
    let (mut write, mut read) = ws.split();

    // Step 1: the first frame must be a valid auth handshake.
    let first = match read.next().await {
        Some(Ok(Message::Text(t))) => t,
        _ => {
            let _ = write.send(Message::Close(None)).await;
            anyhow::bail!("no auth frame");
        }
    };
    match parse_auth_token(&first) {
        Some(token) if token_ok(&config.auth_token, &token) => {}
        _ => {
            tracing::warn!(conn = id, "rejected: invalid or missing auth token");
            let _ = write.send(Message::Close(None)).await;
            anyhow::bail!("auth failed");
        }
    }
    tracing::info!(conn = id, "client authenticated");

    // Step 2a: forward every *other* client's frames to this client.
    let mut rx = tx.subscribe();
    let forwarder = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok((sender, text)) => {
                    if sender != id && write.send(Message::Text(text)).await.is_err() {
                        break; // client went away
                    }
                }
                // Fell behind the channel: drop the client so its device re-syncs on reconnect.
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(conn = id, "client lagged {n} frames; disconnecting");
                    let _ = write.send(Message::Close(None)).await;
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Step 2b: broadcast every subsequent frame this client sends to all others.
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                // A send error means there are no receivers right now (no other clients); that
                // is fine — the frame is simply dropped, matching best-effort broadcast.
                let _ = tx.send((id, text));
            }
            Ok(Message::Close(_)) | Err(_) => break,
            // Ping/Pong/Binary are ignored; the protocol is text-only.
            Ok(_) => {}
        }
    }

    forwarder.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_validation() {
        assert!(
            token_ok("", "anything"),
            "empty configured token accepts any"
        );
        assert!(token_ok("s3cr3t", "s3cr3t"));
        assert!(!token_ok("s3cr3t", "wrong"));
        assert!(!token_ok("s3cr3t", ""));
    }

    #[test]
    fn auth_frame_parsing() {
        assert_eq!(
            parse_auth_token(r#"{"type":"auth","token":"abc"}"#),
            Some("abc".to_string())
        );
        assert_eq!(
            parse_auth_token(r#"{"type":"auth","token":""}"#),
            Some(String::new())
        );
        // Not an auth frame / malformed → None.
        assert_eq!(parse_auth_token(r#"{"type":"changes","changes":[]}"#), None);
        assert_eq!(parse_auth_token(r#"{"type":"auth"}"#), None);
        assert_eq!(parse_auth_token("not json"), None);
    }
}
