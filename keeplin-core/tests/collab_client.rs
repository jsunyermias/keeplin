//! Tests of the collaborative client: the body↔lines diffing state machine,
//! and an end-to-end round trip through a mock keeplin-srv (REST listing +
//! WebSocket Join/Welcome/Op relay) between two `CollabBackend`-wrapped
//! `DbBackend`s.

use std::net::SocketAddr;
use std::sync::Arc;

use keeplin_core::collab::protocol::{
    CollabClientMsg, CollabServerMsg, LineSnapshot, NoteLinesSnapshot,
};
use keeplin_core::collab::state::NoteLines;
use keeplin_core::collab::{device_id_from_token, CollabBackend, CollabConfig};
use keeplin_core::storage::{db::DbBackend, NoteRepository, StorageBackend};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Forge an unsigned JWT whose payload carries `device_id` (the client only
/// decodes, never verifies).
fn token_for(device: &str) -> String {
    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload = engine.encode(format!(r#"{{"device_id":"{device}"}}"#));
    format!("{}.{payload}.sig", engine.encode(r#"{"alg":"none"}"#))
}

// ── State machine ────────────────────────────────────────────────────────────

#[test]
fn token_device_id_decodes() {
    let id = Uuid::new_v4().to_string();
    assert_eq!(device_id_from_token(&token_for(&id)), Some(id));
    assert_eq!(device_id_from_token("garbage"), None);
}

#[test]
fn diff_roundtrip_materializes_new_body() {
    let mut lines = NoteLines::default();
    let ops = lines.diff_body("uno\ndos\ntres", "dev");
    assert_eq!(ops.len(), 3);
    assert_eq!(lines.materialize(), "uno\ndos\ntres");

    // Edit the middle line, delete the last, append two.
    let ops = lines.diff_body("uno\nDOS\ncuatro\ncinco", "dev");
    assert!(!ops.is_empty());
    assert_eq!(lines.materialize(), "uno\nDOS\ncuatro\ncinco");
}

#[test]
fn ops_replay_identically_on_another_mirror() {
    let mut a = NoteLines::default();
    let mut b = NoteLines::default();
    let mut all = Vec::new();
    all.extend(a.diff_body("x\ny", "dev-a"));
    all.extend(a.diff_body("x\nY\nz", "dev-a"));
    all.extend(a.diff_body("Y\nz", "dev-a"));
    for op in &all {
        b.apply(op);
    }
    assert_eq!(a.materialize(), b.materialize());
    assert_eq!(b.materialize(), "Y\nz");
}

// ── Mock keeplin-srv ─────────────────────────────────────────────────────────

/// Minimal stand-in for keeplin-srv: `GET /api/notes` lists one pre-seeded
/// note; `/api/ws` answers `Join` with a Welcome snapshot of that note and
/// relays `Op` frames to every *other* connection. POST/PATCH/DELETE are
/// accepted and ignored.
async fn mock_server(note_id: Uuid) -> SocketAddr {
    use axum::extract::ws::{Message as AxMsg, WebSocket, WebSocketUpgrade};
    use axum::routing::{any, get};
    use axum::Json;

    let (relay, _) = broadcast::channel::<(u64, String)>(64);
    let seeded = NoteLinesSnapshot {
        note_id,
        order: vec![],
        updated_at: chrono::Utc::now(),
        vv: Default::default(),
        last_writer: String::new(),
        lines: Vec::<LineSnapshot>::new(),
    };

    let conn_seq = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let app = axum::Router::new()
        .route(
            "/api/notes",
            get(move || async move {
                Json(serde_json::json!([{
                    "id": note_id,
                    "title": "seeded",
                    "notebook_id": null,
                    "is_todo": false,
                    "todo_due": null,
                    "todo_completed": null,
                    "created_at": chrono::Utc::now(),
                    "updated_at": chrono::Utc::now(),
                }]))
            })
            .post(|| async { Json(serde_json::json!({"ok": true})) }),
        )
        .route("/api/notes/:id", any(|| async { "{}" }))
        .route(
            "/api/ws",
            get(move |ws: WebSocketUpgrade| {
                let relay = relay.clone();
                let seeded = seeded.clone();
                let conn_seq = conn_seq.clone();
                async move {
                    ws.on_upgrade(move |mut socket: WebSocket| async move {
                        let me = conn_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let mut rx = relay.subscribe();
                        loop {
                            tokio::select! {
                                fanned = rx.recv() => {
                                    let Ok((from, text)) = fanned else { break };
                                    if from != me
                                        && socket.send(AxMsg::Text(text)).await.is_err()
                                    {
                                        break;
                                    }
                                }
                                frame = socket.recv() => {
                                    let Some(Ok(AxMsg::Text(text))) = frame else { break };
                                    match serde_json::from_str::<CollabClientMsg>(&text) {
                                        Ok(CollabClientMsg::Join { note_id }) => {
                                            let welcome = CollabServerMsg::Welcome {
                                                note_id,
                                                snapshot: seeded.clone(),
                                            };
                                            let _ = socket
                                                .send(AxMsg::Text(
                                                    serde_json::to_string(&welcome).unwrap(),
                                                ))
                                                .await;
                                        }
                                        Ok(CollabClientMsg::Op { note_id, ops }) => {
                                            let fan = CollabServerMsg::Op {
                                                server_seq: 1,
                                                note_id,
                                                user_id: "u".into(),
                                                ops,
                                            };
                                            let _ = relay.send((
                                                me,
                                                serde_json::to_string(&fan).unwrap(),
                                            ));
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    })
                }
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

/// A `CollabBackend<DbBackend>` in a temp dir, started with itself as the top
/// of the stack (no linking/eventing in this test).
async fn client(addr: SocketAddr, device: &str) -> Arc<CollabBackend<DbBackend>> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dev.db");
    std::mem::forget(dir);
    let db = DbBackend::new(path, "", "").await.unwrap();
    let collab = Arc::new(
        CollabBackend::new(
            db,
            CollabConfig {
                api_url: format!("http://{addr}"),
                ws_url: format!("ws://{addr}/api/ws"),
                token: token_for(device),
            },
        )
        .unwrap(),
    );
    let top: Arc<dyn StorageBackend> = collab.clone();
    collab.start(top).await;
    collab
}

/// Poll until the note's local body equals `want` (or panic after ~5s).
async fn wait_body(backend: &Arc<CollabBackend<DbBackend>>, id: Uuid, want: &str) {
    let mut last = String::new();
    for _ in 0..50 {
        if let Ok(note) = backend.read_note(id).await {
            last = note.body.clone();
            if last == want {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("body never became {want:?}; last {last:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edits_travel_between_two_daemons() {
    let note_id = Uuid::new_v4();
    let addr = mock_server(note_id).await;

    let a = client(addr, "dev-a").await;
    let b = client(addr, "dev-b").await;

    // Discovery creates the note locally on both daemons.
    wait_body(&a, note_id, "").await;
    wait_body(&b, note_id, "").await;

    // A writes a body → diffed into ops → relayed → B's local note updates.
    let mut note = a.read_note(note_id).await.unwrap();
    note.body = "hola\ndesde A".into();
    a.update_note(note).await.unwrap();
    wait_body(&b, note_id, "hola\ndesde A").await;

    // B edits one line → A converges to the merged state.
    let mut note = b.read_note(note_id).await.unwrap();
    note.body = "hola\ndesde B".into();
    b.update_note(note).await.unwrap();
    wait_body(&a, note_id, "hola\ndesde B").await;
}
