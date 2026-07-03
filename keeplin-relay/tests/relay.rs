//! End-to-end tests for `keeplin-relay`: stand the real relay up on an ephemeral port and drive
//! genuine `DbBackend` instances through it over a socket, plus an auth-rejection check.

use std::time::Duration;

use keeplin_core::{
    models::Note,
    storage::{db::DbBackend, NoteRepository, SyncBackend},
};
use keeplin_relay::{serve, RelayConfig};
use tokio::net::TcpListener;
use uuid::Uuid;

/// Start an **ephemeral** relay (no durable buffer — the pre-buffer behavior) on an
/// ephemeral port with the given token and return its `ws://` URL. The relay task runs
/// until the test process exits.
async fn spawn_relay(token: &str) -> String {
    let (url, shutdown, _task) = spawn_relay_with(token, None).await;
    // Dropping the sender would resolve the shutdown future and stop the relay; leak it
    // so the relay runs until the test process exits.
    std::mem::forget(shutdown);
    url
}

/// Start a relay (durable when `data_dir` is `Some`) and return its URL plus a shutdown
/// trigger and the serve task's handle, so restart tests can stop it cleanly.
async fn spawn_relay_with(
    token: &str,
    data_dir: Option<std::path::PathBuf>,
) -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = token.to_string();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        serve(
            listener,
            RelayConfig {
                auth_token: token,
                data_dir,
                retention_days: 30,
            },
            async {
                let _ = shutdown_rx.await;
            },
        )
        .await
        .unwrap();
    });
    (format!("ws://{addr}"), shutdown_tx, task)
}

type RawWs =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A raw WebSocket client authenticated with `token`, optionally presenting a device id.
async fn raw_client(url: &str, token: &str, device: Option<&str>) -> RawWs {
    use futures_util::SinkExt;
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let mut auth = serde_json::json!({ "type": "auth", "token": token });
    if let Some(device) = device {
        auth["device_id"] = serde_json::Value::String(device.to_string());
    }
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        auth.to_string(),
    ))
    .await
    .unwrap();
    ws
}

/// The next text frame within `ms` milliseconds, or `None` on timeout/close.
async fn recv_text(ws: &mut RawWs, ms: u64) -> Option<String> {
    use futures_util::StreamExt;
    match tokio::time::timeout(Duration::from_millis(ms), ws.next()).await {
        Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => Some(t),
        _ => None,
    }
}

/// Send one text frame.
async fn send_text(ws: &mut RawWs, text: &str) {
    use futures_util::SinkExt;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        text.to_string(),
    ))
    .await
    .unwrap();
}

/// A server-mode `DbBackend` connected to `url` with `token`. The temp dir is leaked so it
/// outlives the open database for the test.
async fn device(url: &str, token: &str) -> DbBackend {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(path, url, token).await.unwrap()
}

fn epoch() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap()
}

/// Poll `receive_changes` + `apply_change` for up to ~3 s until note `id` is present on `dev`.
async fn sync_until_present(dev: &DbBackend, id: Uuid) -> bool {
    for _ in 0..30 {
        for change in dev.receive_changes().await.unwrap() {
            dev.apply_change(change).await.unwrap();
        }
        if dev.read_note(id).await.is_ok() {
            return true;
        }
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn note_created_on_one_device_reaches_another_through_the_relay() {
    let url = spawn_relay("s3cr3t").await;
    let a = device(&url, "s3cr3t").await;
    let b = device(&url, "s3cr3t").await;

    // Device A creates a note and pushes its changes to the relay.
    let note = Note::new("Shared", "over the real relay");
    let id = note.id;
    a.create_note(note).await.unwrap();
    let changes = a.get_changes_since(epoch()).await.unwrap();
    a.send_changes(changes).await.unwrap();

    // The relay forwards them to device B, which converges on the note.
    assert!(
        sync_until_present(&b, id).await,
        "device B must receive A's note through the relay"
    );
    let read = b.read_note(id).await.unwrap();
    assert_eq!(read.title, "Shared");
    assert_eq!(read.body, "over the real relay");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_with_the_wrong_token_cannot_sync() {
    let url = spawn_relay("correct-token").await;

    // The sender authenticates correctly; the receiver presents the wrong token, so the relay
    // closes its connection and it never receives the broadcast.
    let a = device(&url, "correct-token").await;
    let b = device(&url, "wrong-token").await;

    let note = Note::new("secret", "should not arrive");
    let id = note.id;
    a.create_note(note).await.unwrap();
    let changes = a.get_changes_since(epoch()).await.unwrap();
    a.send_changes(changes).await.unwrap();

    // Give the (rejected) receiver time to attempt draining, then assert it saw nothing.
    for _ in 0..5 {
        for change in b.receive_changes().await.unwrap() {
            b.apply_change(change).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        b.read_note(id).await.is_err(),
        "a device that failed auth must not receive changes"
    );
}

// ── Durable buffer tests ──────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_device_catches_up_from_the_durable_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let (url, _shutdown, _task) = spawn_relay_with("tok", Some(dir.path().to_path_buf())).await;

    // Device A sends a batch while device B is offline.
    let mut a = raw_client(&url, "tok", Some("device-a")).await;
    send_text(&mut a, r#"{"type":"changes","marker":"missed-1"}"#).await;
    tokio::time::sleep(Duration::from_millis(200)).await; // let the relay journal it

    // B connects later and is caught up from the buffer.
    let mut b = raw_client(&url, "tok", Some("device-b")).await;
    let got = recv_text(&mut b, 3_000).await.expect("B must be caught up");
    assert!(got.contains("missed-1"), "got: {got}");

    // The sender never receives its own frame back — live or replayed.
    assert_eq!(recv_text(&mut a, 400).await, None);
    drop(a);
    let mut a2 = raw_client(&url, "tok", Some("device-a")).await;
    assert_eq!(
        recv_text(&mut a2, 400).await,
        None,
        "own frames must not replay to their sender on reconnect"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delivery_cursor_prevents_redelivery() {
    let dir = tempfile::tempdir().unwrap();
    let (url, _shutdown, _task) = spawn_relay_with("tok", Some(dir.path().to_path_buf())).await;

    let mut a = raw_client(&url, "tok", Some("device-a")).await;
    send_text(&mut a, r#"{"type":"changes","marker":"once-only"}"#).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // First connection replays the frame and advances B's cursor…
    let mut b = raw_client(&url, "tok", Some("device-b")).await;
    assert!(recv_text(&mut b, 3_000).await.is_some());
    tokio::time::sleep(Duration::from_millis(200)).await; // let the cursor persist
    drop(b);

    // …so a reconnect delivers nothing.
    let mut b2 = raw_client(&url, "tok", Some("device-b")).await;
    assert_eq!(recv_text(&mut b2, 400).await, None, "no re-delivery");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffer_survives_a_relay_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (url, shutdown, task) = spawn_relay_with("tok", Some(dir.path().to_path_buf())).await;

    let mut a = raw_client(&url, "tok", Some("device-a")).await;
    send_text(&mut a, r#"{"type":"changes","marker":"survives-restart"}"#).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(a);

    // Stop the relay, then start a fresh one over the same data directory.
    let _ = shutdown.send(());
    task.await.unwrap();
    let (url2, _shutdown2, _task2) = spawn_relay_with("tok", Some(dir.path().to_path_buf())).await;

    let mut b = raw_client(&url2, "tok", Some("device-b")).await;
    let got = recv_text(&mut b, 3_000)
        .await
        .expect("the journal must survive a restart");
    assert!(got.contains("survives-restart"), "got: {got}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_without_device_id_gets_live_broadcast_only() {
    let dir = tempfile::tempdir().unwrap();
    let (url, _shutdown, _task) = spawn_relay_with("tok", Some(dir.path().to_path_buf())).await;

    // A frame sent before the legacy client connects…
    let mut a = raw_client(&url, "tok", Some("device-a")).await;
    send_text(&mut a, r#"{"type":"changes","marker":"before"}"#).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // …is not replayed to it (no device id → no cursor, no catch-up)…
    let mut legacy = raw_client(&url, "tok", None).await;
    assert_eq!(recv_text(&mut legacy, 400).await, None);

    // …but live frames still reach it.
    send_text(&mut a, r#"{"type":"changes","marker":"after"}"#).await;
    let got = recv_text(&mut legacy, 3_000).await.expect("live broadcast");
    assert!(got.contains("after"), "got: {got}");
}
