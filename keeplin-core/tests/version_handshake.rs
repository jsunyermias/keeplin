// md:Overview

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::{routing::get, Json, Router};
use keeplin_core::collab::{CollabBackend, CollabConfig};
use keeplin_core::compat::PROTOCOL_VERSION;
use keeplin_core::models::Note;
use keeplin_core::storage::db::DbBackend;
use keeplin_core::storage::{HistoryRepository, NoteRepository, StorageBackend};
use tokio::net::TcpListener;

// md:fn spawn_version_server
async fn spawn_version_server(protocol_version: Option<u32>, hits: Arc<AtomicU64>) -> SocketAddr {
    let mut app = Router::new().route(
        "/api/notes/:id/history",
        get(|| async { Json(serde_json::json!([])) }),
    );
    if let Some(proto) = protocol_version {
        app = app.route(
            "/version",
            get(move || {
                let hits = hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!({
                        "name": "keeplin-srv",
                        "version": "0.0.0-test",
                        "protocol_version": proto,
                        "capabilities": ["history"],
                    }))
                }
            }),
        );
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

// md:fn fake_token
fn fake_token() -> String {
    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload = engine.encode(r#"{"device_id":"dev-1"}"#);
    format!("{}.{payload}.sig", engine.encode(r#"{"alg":"none"}"#))
}

// md:fn db_path
fn db_path() -> std::path::PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dev.db");
    std::mem::forget(dir);
    path
}

// md:fn compatible_version_connects_and_primes_capabilities
#[tokio::test]
async fn compatible_version_connects_and_primes_capabilities() {
    let hits = Arc::new(AtomicU64::new(0));
    let addr = spawn_version_server(Some(PROTOCOL_VERSION), hits.clone()).await;

    let be = DbBackend::new(db_path(), format!("ws://{addr}/api/sync"), "tok")
        .await
        .expect("a compatible server must not fail construction");
    assert_eq!(hits.load(Ordering::SeqCst), 1, "handshake fetched /version");

    let note = be.create_note(Note::new("T", "v1")).await.unwrap();
    let _ = be.note_history(note.id, 0).await.unwrap();
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "capability cache primed at startup; /version must not be refetched"
    );
}

// md:fn incompatible_version_fails_construction_loudly
#[tokio::test]
async fn incompatible_version_fails_construction_loudly() {
    let hits = Arc::new(AtomicU64::new(0));
    let addr = spawn_version_server(Some(PROTOCOL_VERSION + 7), hits.clone()).await;

    let msg = match DbBackend::new(db_path(), format!("ws://{addr}/api/sync"), "tok").await {
        Ok(_) => panic!("an incompatible server must fail construction"),
        Err(e) => e.to_string(),
    };
    assert!(msg.contains("incompatible"), "{msg}");
    assert!(
        msg.contains(&format!("protocol {}", PROTOCOL_VERSION + 7)),
        "{msg}"
    );
    assert!(msg.contains("upgrade"), "{msg}");
}

// md:fn missing_version_warns_and_continues
#[tokio::test]
async fn missing_version_warns_and_continues() {
    let hits = Arc::new(AtomicU64::new(0));
    let addr = spawn_version_server(None, hits.clone()).await;

    let be = DbBackend::new(db_path(), format!("ws://{addr}/api/sync"), "tok")
        .await
        .expect("an old server without /version must not fail construction");
    let note = be.create_note(Note::new("T", "v1")).await.unwrap();
    assert_eq!(be.read_note(note.id).await.unwrap().body, "v1");
}

// md:fn collab_start_applies_the_same_rule
#[tokio::test]
async fn collab_start_applies_the_same_rule() {
    let addr = spawn_version_server(Some(PROTOCOL_VERSION + 1), Arc::new(AtomicU64::new(0))).await;
    let db = DbBackend::new(db_path(), "", "").await.unwrap();
    let collab = Arc::new(
        CollabBackend::new(
            db,
            CollabConfig {
                api_url: format!("http://{addr}"),
                ws_url: format!("ws://{addr}/api/ws"),
                token: fake_token(),
            },
        )
        .unwrap(),
    );
    let top: Arc<dyn StorageBackend> = collab.clone();
    let err = collab
        .start(top)
        .await
        .expect_err("incompatible server must refuse the collab session");
    assert!(err.to_string().contains("incompatible"), "{err}");

    let addr = spawn_version_server(None, Arc::new(AtomicU64::new(0))).await;
    let db = DbBackend::new(db_path(), "", "").await.unwrap();
    let collab = Arc::new(
        CollabBackend::new(
            db,
            CollabConfig {
                api_url: format!("http://{addr}"),
                ws_url: format!("ws://{addr}/api/ws"),
                token: fake_token(),
            },
        )
        .unwrap(),
    );
    let top: Arc<dyn StorageBackend> = collab.clone();
    collab
        .start(top)
        .await
        .expect("a server without /version must not refuse the session");
}
