//! Startup protocol handshake against `GET /version` (`compat` module wired
//! into `DbBackend::new` and `CollabBackend::start`).
//!
//! Three server behaviours, three contracts:
//! - compatible `protocol_version` → construction succeeds (and the
//!   capability cache is primed — `/version` is not fetched again);
//! - incompatible `protocol_version` → construction fails loudly with an
//!   actionable message, before any sync is attempted;
//! - no usable `/version` (older server) → warn and continue, exactly as
//!   before the handshake existed.

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

/// Serve a canned `/version` reply (counting hits) plus an empty history
/// endpoint, so a client with a primed capability cache can exercise a
/// follow-up REST call without refetching `/version`.
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

/// A JWT-shaped token with a `device_id` claim (`CollabBackend::new` extracts
/// it without verifying; only the server verifies signatures).
fn fake_token() -> String {
    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload = engine.encode(r#"{"device_id":"dev-1"}"#);
    format!("{}.{payload}.sig", engine.encode(r#"{"alg":"none"}"#))
}

fn db_path() -> std::path::PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dev.db");
    std::mem::forget(dir);
    path
}

/// A compatible server: construction succeeds, and the startup handshake
/// primes the capability cache — `/version` is fetched exactly once even when
/// a capability-gated feature (server history) is used afterwards.
#[tokio::test]
async fn compatible_version_connects_and_primes_capabilities() {
    let hits = Arc::new(AtomicU64::new(0));
    let addr = spawn_version_server(Some(PROTOCOL_VERSION), hits.clone()).await;

    let be = DbBackend::new(db_path(), format!("ws://{addr}/api/sync"), "tok")
        .await
        .expect("a compatible server must not fail construction");
    assert_eq!(hits.load(Ordering::SeqCst), 1, "handshake fetched /version");

    // The history read consults the capability cache primed at startup: no
    // second /version fetch.
    let note = be.create_note(Note::new("T", "v1")).await.unwrap();
    let _ = be.note_history(note.id, 0).await.unwrap();
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "capability cache primed at startup; /version must not be refetched"
    );
}

/// An incompatible server fails construction loudly, with a message naming
/// both protocol versions and which side to upgrade. No sync is attempted.
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

/// A server with no `/version` (an older keeplin-srv): warn and continue —
/// behaviour is unchanged from before the handshake existed.
#[tokio::test]
async fn missing_version_warns_and_continues() {
    let hits = Arc::new(AtomicU64::new(0));
    let addr = spawn_version_server(None, hits.clone()).await;

    let be = DbBackend::new(db_path(), format!("ws://{addr}/api/sync"), "tok")
        .await
        .expect("an old server without /version must not fail construction");
    // Fully usable locally (offline-capable client).
    let note = be.create_note(Note::new("T", "v1")).await.unwrap();
    assert_eq!(be.read_note(note.id).await.unwrap().body, "v1");
}

/// The collaborative session start applies the same three-way rule: an
/// incompatible server refuses the session (no connection task), a missing
/// `/version` proceeds.
#[tokio::test]
async fn collab_start_applies_the_same_rule() {
    // Incompatible → Err.
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

    // Missing /version → Ok (warn and continue).
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
