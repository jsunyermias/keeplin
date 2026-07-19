// md:Overview

use std::net::SocketAddr;
use std::sync::Arc;

use keeplin_core::collab::protocol::{
    CollabClientMsg, CollabServerMsg, LineSnapshot, NoteLinesSnapshot,
};
use keeplin_core::collab::state::NoteLines;
use keeplin_core::collab::{device_id_from_token, CollabBackend, CollabConfig};
use keeplin_core::models::{Change, Note, Resource};
use keeplin_core::storage::{
    db::DbBackend, NoteRepository, ResourceRepository, StorageBackend, SyncBackend,
};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use uuid::Uuid;

// md:fn token_for
fn token_for(device: &str) -> String {
    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload = engine.encode(format!(r#"{{"device_id":"{device}"}}"#));
    format!("{}.{payload}.sig", engine.encode(r#"{"alg":"none"}"#))
}

// md:fn token_device_id_decodes
#[test]
fn token_device_id_decodes() {
    let id = Uuid::new_v4().to_string();
    assert_eq!(device_id_from_token(&token_for(&id)), Some(id));
    assert_eq!(device_id_from_token("garbage"), None);
}

// md:fn diff_roundtrip_materializes_new_body
#[test]
fn diff_roundtrip_materializes_new_body() {
    let mut lines = NoteLines::default();
    let ops = lines.diff_body("uno\ndos\ntres", "dev");
    assert_eq!(ops.len(), 3);
    assert_eq!(lines.materialize(), "uno\ndos\ntres");

    let ops = lines.diff_body("uno\nDOS\ncuatro\ncinco", "dev");
    assert!(!ops.is_empty());
    assert_eq!(lines.materialize(), "uno\nDOS\ncuatro\ncinco");
}

// md:fn ops_replay_identically_on_another_mirror
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

// md:fn mock_server
async fn mock_server(note_id: Uuid) -> SocketAddr {
    use axum::extract::ws::{Message as AxMsg, WebSocket, WebSocketUpgrade};
    use axum::extract::Path;
    use axum::http::StatusCode;
    use axum::routing::{any, get, put};
    use axum::Json;
    use std::collections::HashMap;
    use tokio::sync::Mutex as TokioMutex;

    type Blobs = Arc<TokioMutex<HashMap<Uuid, Vec<u8>>>>;
    let blobs: Blobs = Arc::new(TokioMutex::new(HashMap::new()));

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
            "/api/resources/:id/data",
            {
                let put_blobs = blobs.clone();
                let get_blobs = blobs.clone();
                put(move |Path(id): Path<Uuid>, body: axum::body::Bytes| {
                    let blobs = put_blobs.clone();
                    async move {
                        blobs.lock().await.insert(id, body.to_vec());
                        Json(serde_json::json!({ "ok": true }))
                    }
                })
                .get(move |Path(id): Path<Uuid>| {
                    let blobs = get_blobs.clone();
                    async move {
                        match blobs.lock().await.get(&id) {
                            Some(bytes) => (StatusCode::OK, bytes.clone()),
                            None => (StatusCode::NOT_FOUND, Vec::new()),
                        }
                    }
                })
            },
        )
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
                                        Ok(CollabClientMsg::Cursor { note_id, cursor }) => {
                                            let presence = CollabServerMsg::Presence {
                                                note_id,
                                                users: vec![keeplin_core::collab::protocol::PresenceInfo {
                                                    user_id: "u".into(),
                                                    display_name: "mock".into(),
                                                    cursor: Some(cursor),
                                                }],
                                            };
                                            let _ = socket
                                                .send(AxMsg::Text(
                                                    serde_json::to_string(&presence).unwrap(),
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

// md:fn client
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
    collab.start(top).await.unwrap();
    collab
}

// md:fn wait_body
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

// md:fn created_note_body_survives_the_join_welcome
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn created_note_body_survives_the_join_welcome() {
    let seeded = Uuid::new_v4();
    let addr = mock_server(seeded).await;
    let a = client(addr, "dev-a").await;

    let note = a.create_note(Note::new("t", "hello world")).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(
        a.read_note(note.id).await.unwrap().body,
        "hello world",
        "the created note's local body must survive the Welcome snapshot"
    );
}

// md:fn edits_travel_between_two_daemons
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edits_travel_between_two_daemons() {
    let note_id = Uuid::new_v4();
    let addr = mock_server(note_id).await;

    let a = client(addr, "dev-a").await;
    let b = client(addr, "dev-b").await;

    wait_body(&a, note_id, "").await;
    wait_body(&b, note_id, "").await;

    let mut note = a.read_note(note_id).await.unwrap();
    note.body = "hola\ndesde A".into();
    a.update_note(note).await.unwrap();
    wait_body(&b, note_id, "hola\ndesde A").await;

    let mut note = b.read_note(note_id).await.unwrap();
    note.body = "hola\ndesde B".into();
    b.update_note(note).await.unwrap();
    wait_body(&a, note_id, "hola\ndesde B").await;
}

// md:fn resource_blob_uploads_out_of_band_and_downloads_on_read
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resource_blob_uploads_out_of_band_and_downloads_on_read() {
    let note_id = Uuid::new_v4();
    let addr = mock_server(note_id).await;

    let a = client(addr, "dev-a").await;
    let bytes = b"opaque-ciphertext-bytes".to_vec();
    let resource = a
        .create_resource(
            Resource::new("photo", "image/png", "photo.png", bytes.len() as u64),
            bytes.clone(),
        )
        .await
        .unwrap();

    let changes = a
        .get_changes_since(chrono::DateTime::from_timestamp(0, 0).unwrap())
        .await
        .unwrap();
    let create = changes
        .iter()
        .find(|c| matches!(c, Change::ResourceCreate { resource: r, .. } if r.id == resource.id))
        .expect("a ResourceCreate is queued");
    match create {
        Change::ResourceCreate { data, .. } => {
            assert!(data.is_none(), "binary is stripped from the relayed change")
        }
        _ => unreachable!(),
    }

    let b = client(addr, "dev-b").await;
    b.apply_change(create.clone()).await.unwrap();
    let (got_meta, got_bytes) = b.read_resource(resource.id).await.unwrap();
    assert_eq!(got_meta.id, resource.id);
    assert_eq!(got_bytes, bytes, "B downloaded the blob from the server");
}

// md:fn cursor_updates_flow_into_presence
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cursor_updates_flow_into_presence() {
    let note_id = Uuid::new_v4();
    let addr = mock_server(note_id).await;
    let a = client(addr, "dev-a").await;
    wait_body(&a, note_id, "").await;

    let handle = a.handle();
    assert!(handle.presence(note_id).await.is_empty());

    let line = Uuid::new_v4();
    handle.send_cursor(
        note_id,
        keeplin_core::collab::protocol::Cursor {
            line_id: line,
            column: 7,
        },
    );

    let mut seen = Vec::new();
    for _ in 0..50 {
        seen = handle.presence(note_id).await;
        if seen
            .iter()
            .any(|p| p.cursor.as_ref().is_some_and(|c| c.column == 7))
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("presence never carried the cursor: {seen:?}");
}
