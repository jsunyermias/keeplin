//! End-to-end test of the `DbBackend` WebSocket synchronisation path.
//!
//! Earlier tests exercise `DbBackend` only in offline mode. This suite stands up a real
//! (in-process) WebSocket relay — a minimal stand-in for the production sync server — and
//! drives two `DbBackend` instances through the genuine wire protocol: the `auth`
//! handshake performed on construction, `send_changes` (which serialises a `changes`
//! envelope and writes it to the socket), the relay forwarding the batch to the *other*
//! device, and `receive_changes` (which drains and parses incoming frames). This proves
//! that a change actually travels between two devices over a socket, not just through the
//! local database.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use keeplin_core::{
    error::{StorageError, SyncError},
    models::{Change, Note},
    storage::{db::DbBackend, NoteRepository, SyncBackend},
    sync::run_sync,
};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

/// Start an in-process WebSocket relay on an ephemeral port and return its bound address.
async fn spawn_relay() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    spawn_relay_on(listener).await
}

/// Start the in-process WebSocket relay on an already-bound listener.
///
/// The relay mimics the production hub: it accepts any number of client connections,
/// discards each client's first frame (the `auth` handshake), and forwards every
/// subsequent text frame (a `changes` batch) to **all other** connected clients — never
/// echoing it back to the sender. Fan-out uses a `broadcast` channel tagged with a
/// per-connection id so a device never receives its own batch.
///
/// Taking the listener (rather than binding internally) lets a test reserve an address,
/// run a device against it while nothing is listening, and only then bring the relay up
/// on that same address — the "relay was down, then recovered" scenario.
async fn spawn_relay_on(listener: TcpListener) -> SocketAddr {
    let addr = listener.local_addr().unwrap();
    let (tx, _rx) = broadcast::channel::<(u64, String)>(256);
    let next_id = Arc::new(AtomicU64::new(0));

    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let tx = tx.clone();
            let my_id = next_id.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(async move {
                let ws = match tokio_tungstenite::accept_async(stream).await {
                    Ok(ws) => ws,
                    Err(_) => return,
                };
                let (mut write, mut read) = ws.split();
                let mut rx = tx.subscribe();

                // Forward other devices' batches to this client.
                let forwarder = tokio::spawn(async move {
                    while let Ok((sender, text)) = rx.recv().await {
                        if sender != my_id && write.send(Message::Text(text)).await.is_err() {
                            break;
                        }
                    }
                });

                // Read loop: drop the first (auth) frame, broadcast the rest.
                let mut seen_auth = false;
                while let Some(Ok(msg)) = read.next().await {
                    if let Message::Text(text) = msg {
                        if !seen_auth {
                            seen_auth = true;
                            continue;
                        }
                        let _ = tx.send((my_id, text));
                    }
                }
                forwarder.abort();
            });
        }
    });

    addr
}

/// Create a server-mode `DbBackend` connected to `url`. The temp dir is leaked so it
/// outlives the open database for the duration of the test.
async fn device(url: &str) -> DbBackend {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(path, url, "test-token").await.unwrap()
}

fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(0, 0).unwrap()
}

/// Push every local change of `dev` to the relay.
async fn push(dev: &DbBackend) {
    let changes = dev.get_changes_since(epoch()).await.unwrap();
    dev.send_changes(changes).await.unwrap();
}

/// Repeatedly `receive_changes` (each call drains ~100 ms) for up to ~3 s, applying every
/// received change, until note `id` is present and — when `want_body` is `Some` — its body
/// matches. Returns whether it converged. The poll loop absorbs the asynchronous
/// accept/forward scheduling without fixed sleeps.
async fn sync_until(dev: &DbBackend, id: Uuid, want_body: Option<&str>) -> bool {
    for _ in 0..30 {
        let remote = dev.receive_changes().await.unwrap();
        for change in remote {
            dev.apply_change(change).await.unwrap();
        }
        if let Ok(note) = dev.read_note(id).await {
            match want_body {
                None => return true,
                Some(body) if note.body == body => return true,
                Some(_) => {}
            }
        }
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn note_create_syncs_between_two_devices() {
    let addr = spawn_relay().await;
    let url = format!("ws://{addr}");
    let a = device(&url).await;
    let b = device(&url).await;

    let note = Note::new("Shared", "over the wire");
    let id = note.id;
    a.create_note(note).await.unwrap();
    push(&a).await;

    assert!(
        sync_until(&b, id, None).await,
        "device B must receive A's note over the websocket"
    );
    let read = b.read_note(id).await.unwrap();
    assert_eq!(read.title, "Shared");
    assert_eq!(read.body, "over the wire");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_propagates_and_converges() {
    let addr = spawn_relay().await;
    let url = format!("ws://{addr}");
    let a = device(&url).await;
    let b = device(&url).await;

    let mut note = Note::new("v1", "body v1");
    let id = note.id;
    a.create_note(note.clone()).await.unwrap();
    push(&a).await;
    assert!(
        sync_until(&b, id, None).await,
        "B must first receive the created note"
    );

    note.title = "v2".to_string();
    note.body = "body v2".to_string();
    note.updated_at = Utc::now();
    a.update_note(note).await.unwrap();
    push(&a).await;

    assert!(
        sync_until(&b, id, Some("body v2")).await,
        "B must converge to A's update over the websocket"
    );
}

/// A sync cycle whose send cannot reach the relay must FAIL — leaving the last-sync
/// watermark unchanged — so the same changes are re-collected and delivered once the
/// relay comes back. Returning `Ok` from an undelivered send would advance the watermark
/// past the batch and drop it from every future sync (the bug this guards against).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_send_keeps_watermark_and_changes_are_resent_after_recovery() {
    // Reserve an address, then drop the listener so nothing is accepting on it.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let url = format!("ws://{addr}");
    // Construction tolerates the dead relay (starts disconnected, local CRUD works).
    let a = device(&url).await;
    let note = Note::new("Queued", "while relay was down");
    let id = note.id;
    a.create_note(note).await.unwrap();

    // The cycle must abort with a WebSocket error and leave the watermark untouched.
    let err = run_sync(&a, |_, _| {}).await.unwrap_err();
    assert!(
        matches!(err, SyncError::Storage(StorageError::WebSocket(_))),
        "expected a WebSocket storage error, got: {err}"
    );
    assert_eq!(
        a.get_last_sync_time().await.unwrap(),
        epoch(),
        "a failed send must not advance the last-sync watermark"
    );

    // Relay recovers on the SAME address; the next cycle re-collects and delivers.
    let listener = TcpListener::bind(addr).await.unwrap();
    spawn_relay_on(listener).await;
    let b = device(&url).await;

    run_sync(&a, |_, _| {})
        .await
        .expect("sync must succeed once the relay is back");
    assert!(
        a.get_last_sync_time().await.unwrap() > epoch(),
        "a successful cycle advances the watermark"
    );
    assert!(
        sync_until(&b, id, None).await,
        "the change queued while the relay was down must reach device B"
    );
}

/// `send_changes` on a backend with no relay configured (`server_url = ""`) is a
/// deliberate no-op, not an error: the backend is local-only and there is nowhere to
/// send to, so local sync cycles (and the daemon's `/api/sync`) must keep working.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_without_configured_relay_is_a_noop() {
    let a = device("").await;
    a.create_note(Note::new("local", "only")).await.unwrap();
    let changes = a.get_changes_since(epoch()).await.unwrap();
    assert!(!changes.is_empty());
    a.send_changes(changes)
        .await
        .expect("no configured relay → skipping the send is not a failure");
}

/// One malformed text frame from the relay must be skipped with a warning — not abort
/// `receive_changes` — so the well-formed batches behind it still come through.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_frame_does_not_abort_receive() {
    let addr = spawn_relay().await;
    let url = format!("ws://{addr}");
    let b = device(&url).await;

    // A raw WebSocket client: auth frame (dropped by the relay), then garbage, then a
    // valid `changes` envelope. The relay forwards both non-auth frames to `b` in order.
    let note = Note::new("Survivor", "arrived after garbage");
    let id = note.id;
    let envelope = serde_json::json!({
        "type": "changes",
        "batch_id": Uuid::new_v4(),
        "device_id": "raw-client",
        "changes": [Change::NoteCreate { note }],
    })
    .to_string();
    let (mut raw, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    raw.send(Message::Text(r#"{"type":"auth","token":"t"}"#.into()))
        .await
        .unwrap();
    raw.send(Message::Text("this is not json {".into()))
        .await
        .unwrap();
    raw.send(Message::Text(envelope)).await.unwrap();

    // Drain until the valid batch arrives (the garbage frame must not error the call).
    let mut received = Vec::new();
    for _ in 0..30 {
        let batch = b
            .receive_changes()
            .await
            .expect("a malformed frame must not fail receive_changes");
        received.extend(batch);
        if !received.is_empty() {
            break;
        }
    }
    assert_eq!(received.len(), 1, "the valid batch must still be delivered");
    for change in received {
        b.apply_change(change).await.unwrap();
    }
    assert_eq!(b.read_note(id).await.unwrap().title, "Survivor");
}
