//! `keeplin-relay` — a minimal production WebSocket sync hub for Keeplin **server mode**.
//!
//! In server mode each device's `DbBackend` connects to a relay over WebSocket, pushes its
//! local changes, and receives the changes other devices pushed. The relay is a **broadcast
//! hub with a durable buffer**: it authenticates each connection, forwards every `changes`
//! frame a device sends to all other connected devices — never echoing it back to the sender
//! — and (unless running ephemeral) **persists every frame** so a device that was offline
//! receives, on reconnect, everything it missed. Because every change is idempotent and
//! version-vector resolved, replaying frames converges; over-delivery is always safe.
//!
//! The relay does not parse or trust the `changes` payloads — it only moves bytes between
//! devices; conflict resolution and encryption remain entirely in the clients.
//!
//! # Wire protocol
//!
//! Matching `DbBackend::connect_ws` / `send_changes` / `receive_changes`:
//! 1. The client's **first** text frame is the auth handshake:
//!    `{"type":"auth","token":"…","device_id":"…"}`. The relay validates the token
//!    (constant-time) against its configured token and closes the connection on mismatch.
//!    `device_id` is optional: a client that presents one gets **catch-up delivery** (every
//!    buffered frame from other devices past its per-device cursor is replayed before live
//!    forwarding starts); a client without one gets plain live broadcast, exactly as before
//!    this field existed.
//! 2. Every **subsequent** text frame (a `{"type":"changes",…}` batch) is forwarded verbatim
//!    to all other authenticated clients and appended to the durable buffer.
//!
//! # Durability model
//!
//! With a data directory configured (the default for the binary), the relay keeps:
//!
//! - `relay.log` — an append-only NDJSON journal of every broadcast frame, each entry
//!   carrying a monotonic `seq`, the sender's device id (when known), a timestamp, and the
//!   raw frame. Entries older than the retention window are dropped by periodic compaction
//!   (`seq` stays monotonic across compactions and restarts).
//! - `cursors/{device_id}` — the last `seq` delivered to each known device, advanced as
//!   frames are delivered (live or replayed) and consulted on reconnect, so nothing is
//!   re-sent and nothing is skipped. Cursors also advance past a device's *own* frames so
//!   they are never replayed back to it.
//!
//! A device offline longer than the retention window may miss dropped frames; size the
//! window (default 30 days, mirroring the daemon's journal retention) above the longest
//! expected offline period.
//!
//! # TLS
//!
//! The relay speaks plain `ws://`. Terminate TLS at a reverse proxy (nginx, Caddy, …) and point
//! devices at `wss://your-proxy`, exactly as the daemon's REST/token guidance recommends — the
//! daemon refuses a non-loopback `ws://` `server_url` for that reason.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, Mutex};
use tokio_tungstenite::tungstenite::Message;

/// How many pending broadcast frames a slow client may fall behind before it is dropped from
/// the channel (a `Lagged` error). Sized generously; a lagged client is disconnected and — if
/// it presented a device id — caught up from the durable buffer on reconnect.
const BROADCAST_CAPACITY: usize = 1024;

/// How often the compaction task drops buffered frames older than the retention window.
const COMPACTION_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);

/// Relay configuration.
pub struct RelayConfig {
    /// Shared secret every client must present in its first frame. An **empty** token disables
    /// authentication (development only) — the caller should warn loudly in that case.
    pub auth_token: String,
    /// Directory for the durable frame buffer. `None` runs the relay **ephemeral** — a pure
    /// in-memory broadcast where a device misses whatever was sent while it was offline
    /// (the pre-buffer behavior, still useful for tests and throwaway setups).
    pub data_dir: Option<PathBuf>,
    /// Days to keep buffered frames before compaction drops them (`0` = keep forever).
    /// Size this above the longest a device is expected to stay offline.
    pub retention_days: u64,
}

/// One frame on the fan-out bus, tagged with everything a forwarder needs to decide
/// delivery: the sending connection (echo suppression for device-less clients), the durable
/// sequence number (dedupe against replay; `None` when the relay runs ephemeral), and the
/// sender's device id (echo suppression across reconnects).
#[derive(Clone)]
struct Fanout {
    conn: u64,
    seq: Option<u64>,
    sender_device: Option<String>,
    text: String,
}

/// One persisted journal entry: a broadcast frame plus its delivery metadata.
#[derive(Debug, Serialize, Deserialize)]
struct BufferedFrame {
    seq: u64,
    #[serde(default)]
    sender: Option<String>,
    ts: DateTime<Utc>,
    frame: String,
}

/// The durable frame buffer: an append-only NDJSON journal plus per-device delivery
/// cursors. Methods take `&mut self` where they mutate; callers serialise all access
/// through a `Mutex`.
struct Buffer {
    dir: PathBuf,
    next_seq: u64,
    retention_days: u64,
}

impl Buffer {
    /// Open (creating if needed) the buffer under `dir`, compacting expired entries and
    /// recovering the next sequence number from the surviving journal.
    async fn open(dir: &Path, retention_days: u64) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(dir.join("cursors")).await?;
        let mut buffer = Self {
            dir: dir.to_path_buf(),
            next_seq: 1,
            retention_days,
        };
        buffer.compact().await?;
        Ok(buffer)
    }

    fn log_path(&self) -> PathBuf {
        self.dir.join("relay.log")
    }

    /// Path of a device's delivery cursor. The id has been validated by
    /// [`sanitize_device_id`] before it ever reaches here.
    fn cursor_path(&self, device: &str) -> PathBuf {
        self.dir.join("cursors").join(device)
    }

    /// Append one frame to the journal, returning its sequence number.
    async fn append(&mut self, sender: Option<&str>, frame: &str) -> anyhow::Result<u64> {
        let seq = self.next_seq;
        let entry = BufferedFrame {
            seq,
            sender: sender.map(str::to_string),
            ts: Utc::now(),
            frame: frame.to_string(),
        };
        let line = serde_json::to_string(&entry)? + "\n";
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;
        self.next_seq += 1;
        Ok(seq)
    }

    /// Every journal entry with `seq > cursor`, oldest first. Malformed lines are skipped
    /// with a warning so one torn write cannot wedge catch-up delivery.
    async fn frames_after(&self, cursor: u64) -> anyhow::Result<Vec<BufferedFrame>> {
        let raw = match tokio::fs::read_to_string(self.log_path()).await {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut frames = Vec::new();
        for line in raw.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<BufferedFrame>(line) {
                Ok(f) if f.seq > cursor => frames.push(f),
                Ok(_) => {}
                Err(e) => tracing::warn!("skipping malformed relay journal line: {e}"),
            }
        }
        frames.sort_by_key(|f| f.seq);
        Ok(frames)
    }

    /// The last sequence delivered to `device`, or `0` when the device is new.
    async fn read_cursor(&self, device: &str) -> u64 {
        match tokio::fs::read_to_string(self.cursor_path(device)).await {
            Ok(s) => s.trim().parse().unwrap_or(0),
            Err(_) => 0,
        }
    }

    /// Persist `device`'s delivery cursor (atomic temp-then-rename, so a crash cannot tear
    /// it; a lost cursor only causes safe re-delivery of idempotent frames).
    async fn write_cursor(&self, device: &str, seq: u64) -> anyhow::Result<()> {
        let path = self.cursor_path(device);
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, seq.to_string()).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    /// Drop journal entries older than the retention window (when one is set) and recover
    /// `next_seq` from what survives. Sequence numbers stay monotonic across compactions
    /// and restarts, so cursors never go backwards.
    async fn compact(&mut self) -> anyhow::Result<()> {
        let frames = self.frames_after(0).await?;
        let max_seq = frames.last().map_or(0, |f| f.seq);
        self.next_seq = self.next_seq.max(max_seq + 1);
        if self.retention_days == 0 {
            return Ok(());
        }
        let cutoff = Utc::now() - chrono::Duration::days(self.retention_days.min(36_500) as i64);
        let kept: Vec<&BufferedFrame> = frames.iter().filter(|f| f.ts >= cutoff).collect();
        if kept.len() == frames.len() {
            return Ok(());
        }
        let mut out = String::new();
        for f in &kept {
            out.push_str(&serde_json::to_string(f)?);
            out.push('\n');
        }
        let path = self.log_path();
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, out).await?;
        tokio::fs::rename(&tmp, &path).await?;
        tracing::info!(
            dropped = frames.len() - kept.len(),
            kept = kept.len(),
            "Compacted relay journal"
        );
        Ok(())
    }
}

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
    let (tx, _rx) = broadcast::channel::<Fanout>(BROADCAST_CAPACITY);

    let buffer = match &config.data_dir {
        Some(dir) => {
            let buffer = Buffer::open(dir, config.retention_days).await?;
            tracing::info!(dir = %dir.display(), retention_days = config.retention_days,
                "durable frame buffer enabled");
            Some(Arc::new(Mutex::new(buffer)))
        }
        None => {
            tracing::info!("running ephemeral: offline devices miss frames sent while away");
            None
        }
    };

    // Periodically drop expired journal entries so the buffer stays bounded by retention.
    let compactor = buffer.as_ref().map(|buffer| {
        let buffer = Arc::clone(buffer);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(COMPACTION_INTERVAL);
            tick.tick().await; // `open()` already compacted; skip the immediate tick
            loop {
                tick.tick().await;
                if let Err(e) = buffer.lock().await.compact().await {
                    tracing::warn!("relay journal compaction failed: {e}");
                }
            }
        })
    });

    let config = Arc::new(config);
    tokio::select! {
        _ = accept_loop(listener, config, tx, buffer) => {}
        _ = shutdown => {
            tracing::info!("Shutdown signal received, stopping relay");
        }
    }
    if let Some(task) = compactor {
        task.abort();
    }
    Ok(())
}

/// Accept connections forever, spawning [`handle_connection`] for each.
async fn accept_loop(
    listener: TcpListener,
    config: Arc<RelayConfig>,
    tx: broadcast::Sender<Fanout>,
    buffer: Option<Arc<Mutex<Buffer>>>,
) {
    let next_id = Arc::new(AtomicU64::new(0));
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let id = next_id.fetch_add(1, Ordering::Relaxed);
                let config = Arc::clone(&config);
                let tx = tx.clone();
                let buffer = buffer.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, id, config, tx, buffer).await {
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

/// Extract the token and optional device id from an auth frame
/// `{"type":"auth","token":"…","device_id":"…"}`. Returns `None` when the frame is not a
/// well-formed auth message. The device id is **client-supplied**, so it is validated by
/// [`sanitize_device_id`] before it can name a cursor file; an unusable id degrades to
/// token-only auth rather than an error.
fn parse_auth(text: &str) -> Option<(String, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("auth") {
        return None;
    }
    let token = v.get("token")?.as_str()?.to_string();
    let device = v
        .get("device_id")
        .and_then(|d| d.as_str())
        .and_then(sanitize_device_id);
    Some((token, device))
}

/// Accept a client-supplied device id only when it is a plausible identifier — 1 to 64
/// ASCII alphanumerics or hyphens (uuids qualify). Anything else (path separators, dots,
/// empty…) is rejected so an id can never traverse out of the cursors directory.
fn sanitize_device_id(raw: &str) -> Option<String> {
    let ok = !raw.is_empty()
        && raw.len() <= 64
        && raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    ok.then(|| raw.to_string())
}

/// Serve one client: perform the auth handshake, replay anything it missed (when it
/// presented a device id and the buffer is enabled), then bridge it onto the broadcast bus —
/// forwarding others' frames to it and its frames to everyone else.
async fn handle_connection(
    stream: TcpStream,
    id: u64,
    config: Arc<RelayConfig>,
    tx: broadcast::Sender<Fanout>,
    buffer: Option<Arc<Mutex<Buffer>>>,
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
    let device = match parse_auth(&first) {
        Some((token, device)) if token_ok(&config.auth_token, &token) => device,
        _ => {
            tracing::warn!(conn = id, "rejected: invalid or missing auth token");
            let _ = write.send(Message::Close(None)).await;
            anyhow::bail!("auth failed");
        }
    };
    tracing::info!(
        conn = id,
        device = device.as_deref().unwrap_or("<none>"),
        "client authenticated"
    );

    // Step 2: subscribe *before* replaying, so nothing broadcast during the replay is
    // missed; the `delivered` watermark dedupes any frame that shows up in both.
    let mut rx = tx.subscribe();

    // Step 3: catch-up delivery — replay every buffered frame from other devices past this
    // device's cursor, then advance the cursor past everything seen (its own frames
    // included, so they are never replayed back to it later).
    let mut delivered = 0u64;
    if let (Some(buffer), Some(device)) = (&buffer, &device) {
        let (cursor, frames) = {
            let guard = buffer.lock().await;
            let cursor = guard.read_cursor(device).await;
            let frames = guard.frames_after(cursor).await?;
            (cursor, frames)
        };
        delivered = cursor;
        let mut replayed = 0usize;
        for frame in frames {
            if frame.sender.as_deref() != Some(device.as_str()) {
                if write.send(Message::Text(frame.frame)).await.is_err() {
                    anyhow::bail!("client went away during catch-up");
                }
                replayed += 1;
            }
            delivered = frame.seq;
        }
        if delivered > cursor {
            buffer.lock().await.write_cursor(device, delivered).await?;
        }
        if replayed > 0 {
            tracing::info!(conn = id, device = %device, replayed, "caught client up from the buffer");
        }
    }

    // Step 4a: forward every *other* client's live frames to this client, advancing the
    // durable cursor as they are delivered.
    let forward_buffer = buffer.clone();
    let forward_device = device.clone();
    let forwarder = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(fanout) => {
                    // Never past-deliver: anything at or below the watermark was already
                    // replayed (or delivered) to this client.
                    if let Some(seq) = fanout.seq {
                        if seq <= delivered {
                            continue;
                        }
                    }
                    // Echo suppression: by device id when both sides have one (stable
                    // across reconnects), by connection id otherwise.
                    let own = match (&forward_device, &fanout.sender_device) {
                        (Some(mine), Some(sender)) => mine == sender,
                        _ => fanout.conn == id,
                    };
                    if !own && write.send(Message::Text(fanout.text)).await.is_err() {
                        break; // client went away
                    }
                    // Advance the cursor past own frames too — they must never replay.
                    if let (Some(buffer), Some(device), Some(seq)) =
                        (&forward_buffer, &forward_device, fanout.seq)
                    {
                        delivered = seq;
                        if let Err(e) = buffer.lock().await.write_cursor(device, seq).await {
                            tracing::warn!(device = %device, "could not persist cursor: {e}");
                        }
                    }
                }
                // Fell behind the channel: drop the client; a device-identified client is
                // caught up from the buffer on reconnect, others re-sync from scratch.
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(conn = id, "client lagged {n} frames; disconnecting");
                    let _ = write.send(Message::Close(None)).await;
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Step 4b: append every subsequent frame this client sends to the buffer, then fan it
    // out to all others.
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                let seq = match &buffer {
                    Some(buffer) => {
                        match buffer.lock().await.append(device.as_deref(), &text).await {
                            Ok(seq) => Some(seq),
                            Err(e) => {
                                // Never buffer-and-lose silently: drop the frame loudly so
                                // the sender's retry (sync is at-least-once) can succeed
                                // once the relay's disk recovers.
                                tracing::error!("could not persist frame; dropping it: {e}");
                                continue;
                            }
                        }
                    }
                    None => None,
                };
                // A send error means there are no live receivers right now; with the buffer
                // enabled the frame is already journaled for later catch-up.
                let _ = tx.send(Fanout {
                    conn: id,
                    seq,
                    sender_device: device.clone(),
                    text,
                });
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
            parse_auth(r#"{"type":"auth","token":"abc"}"#),
            Some(("abc".to_string(), None))
        );
        assert_eq!(
            parse_auth(r#"{"type":"auth","token":"abc","device_id":"dev-1"}"#),
            Some(("abc".to_string(), Some("dev-1".to_string())))
        );
        // An unusable device id degrades to token-only auth, never an error.
        assert_eq!(
            parse_auth(r#"{"type":"auth","token":"abc","device_id":"../../etc"}"#),
            Some(("abc".to_string(), None))
        );
        assert_eq!(
            parse_auth(r#"{"type":"auth","token":""}"#),
            Some((String::new(), None))
        );
        // Not an auth frame / malformed → None.
        assert_eq!(parse_auth(r#"{"type":"changes","changes":[]}"#), None);
        assert_eq!(parse_auth(r#"{"type":"auth"}"#), None);
        assert_eq!(parse_auth("not json"), None);
    }

    #[test]
    fn device_id_sanitisation() {
        assert_eq!(
            sanitize_device_id("0d9490ba-98a1-4a52-b3ce-1defa2a0b0ad"),
            Some("0d9490ba-98a1-4a52-b3ce-1defa2a0b0ad".to_string())
        );
        assert_eq!(sanitize_device_id(""), None);
        assert_eq!(sanitize_device_id("../escape"), None);
        assert_eq!(sanitize_device_id("a/b"), None);
        assert_eq!(sanitize_device_id("dot.dot"), None);
        assert_eq!(sanitize_device_id(&"x".repeat(65)), None);
    }
}
