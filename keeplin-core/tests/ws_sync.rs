// md:Overview

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use keeplin_core::{
    error::{StorageError, SyncError},
    models::{Change, Note},
    storage::{db::DbBackend, HistoryRepository, NoteRepository, SyncBackend},
    sync::run_sync,
};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

// md:fn spawn_relay
async fn spawn_relay() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    spawn_relay_on(listener).await
}

// md:fn spawn_relay_on
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

                let forwarder = tokio::spawn(async move {
                    while let Ok((sender, text)) = rx.recv().await {
                        if sender != my_id && write.send(Message::Text(text)).await.is_err() {
                            break;
                        }
                    }
                });

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

// md:fn device
async fn device(url: &str) -> DbBackend {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(path, url, "test-token").await.unwrap()
}

// md:fn epoch
fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(0, 0).unwrap()
}

// md:fn push
async fn push(dev: &DbBackend) {
    let changes = dev.get_changes_since(epoch()).await.unwrap();
    dev.send_changes(changes).await.unwrap();
}

// md:fn sync_until
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

// md:fn note_create_syncs_between_two_devices
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

// md:fn update_propagates_and_converges
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

// md:fn failed_send_keeps_watermark_and_changes_are_resent_after_recovery
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_send_keeps_watermark_and_changes_are_resent_after_recovery() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let url = format!("ws://{addr}");
    let a = device(&url).await;
    let note = Note::new("Queued", "while relay was down");
    let id = note.id;
    a.create_note(note).await.unwrap();

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

// md:fn send_without_configured_relay_is_a_noop
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

// md:fn malformed_frame_does_not_abort_receive
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_frame_does_not_abort_receive() {
    let addr = spawn_relay().await;
    let url = format!("ws://{addr}");
    let b = device(&url).await;

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

// md:fn spawn_history_server
async fn spawn_history_server(reply: serde_json::Value) -> SocketAddr {
    use axum::{routing::get, Router};
    let app = Router::new().route(
        "/api/notes/:id/history",
        get(move || {
            let reply = reply.clone();
            async move { axum::Json(reply) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

// md:fn note_history_prefers_the_server_journal
#[tokio::test]
async fn note_history_prefers_the_server_journal() {
    let remote = Note::new("T", "v2-from-server");
    let versions = serde_json::json!([
        { "timestamp": "2026-01-02T00:00:00Z", "device_id": "other-device",
          "entity": serde_json::to_value(&remote).unwrap() },
        { "timestamp": "2026-01-01T00:00:00Z", "device_id": "other-device", "entity": null },
    ]);
    let addr = spawn_history_server(versions).await;
    let be = device(&format!("ws://{addr}/api/sync")).await;

    let hist = be.note_history(remote.id, 0).await.unwrap();
    assert_eq!(hist.len(), 2);
    assert_eq!(hist[0].device_id, "other-device");
    assert_eq!(hist[0].entity.as_ref().unwrap().body, "v2-from-server");
    assert!(hist[1].entity.is_none(), "tombstones survive the trip");
}

// md:fn note_history_falls_back_to_the_local_journal
#[tokio::test]
async fn note_history_falls_back_to_the_local_journal() {
    let addr = spawn_relay().await;
    let be = device(&format!("ws://{addr}/api/sync")).await;
    let note = be.create_note(Note::new("T", "v1")).await.unwrap();

    let hist = be.note_history(note.id, 0).await.unwrap();
    assert_eq!(hist.len(), 1);
    assert_eq!(hist[0].entity.as_ref().unwrap().body, "v1");
}

// md:fn a_404_history_endpoint_is_probed_only_once
#[tokio::test]
async fn a_404_history_endpoint_is_probed_only_once() {
    use axum::http::StatusCode;
    use axum::{routing::get, Router};

    let hits = Arc::new(AtomicU64::new(0));
    let hits_srv = hits.clone();
    let app = Router::new().route(
        "/api/notes/:id/history",
        get(move || {
            let hits = hits_srv.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                StatusCode::NOT_FOUND
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let be = device(&format!("ws://{addr}/api/sync")).await;
    let note = be.create_note(Note::new("T", "v1")).await.unwrap();

    for _ in 0..3 {
        let hist = be.note_history(note.id, 0).await.unwrap();
        assert_eq!(hist.len(), 1, "falls back to the local journal");
    }
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "a 404 endpoint must be probed only once, then skipped"
    );
}

// md:fn history_is_skipped_when_the_server_capability_is_absent
#[tokio::test]
async fn history_is_skipped_when_the_server_capability_is_absent() {
    use axum::{routing::get, Json, Router};

    let hits = Arc::new(AtomicU64::new(0));
    let hits_srv = hits.clone();
    let app = Router::new()
        .route(
            "/version",
            get(|| async {
                Json(serde_json::json!({
                    "name": "keeplin-srv",
                    "version": "0.0.0",
                    "protocol_version": 1,
                    "capabilities": ["readiness"],
                }))
            }),
        )
        .route(
            "/api/notes/:id/history",
            get(move || {
                let hits = hits_srv.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!([]))
                }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let be = device(&format!("ws://{addr}/api/sync")).await;
    let note = be.create_note(Note::new("T", "v1")).await.unwrap();

    for _ in 0..3 {
        let hist = be.note_history(note.id, 0).await.unwrap();
        assert_eq!(hist.len(), 1, "uses the local journal");
    }
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "the history endpoint must never be called when the capability is absent"
    );
}
