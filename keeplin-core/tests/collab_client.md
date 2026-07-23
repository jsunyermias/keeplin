# `tests/collab_client.rs` — collaborative client tests (state machine + mock server e2e)

Self-contained companion for `keeplin-core/tests/collab_client.rs`. It documents
**every code block of the source file, in source order** — a reader with only this
file must be able to understand it without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Block header>`; grep it in either direction. Each section covers
**Identification**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the crate doc and the imports. Marker
`// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview

use std::net::SocketAddr;
use std::sync::Arc;

use keeplin_core::collab::protocol::{
    CollabClientMsg, CollabServerMsg, LineSnapshot, NoteLinesSnapshot,
};
use keeplin_core::collab::state::NoteLines;
use keeplin_core::collab::{device_id_from_token, CollabBackend, CollabConfig};
use keeplin_core::models::{Change, Note, Resource, SYSTEM_RESOURCE_NOTE_ID};
use keeplin_core::storage::{
    db::DbBackend, NoteRepository, ResourceRepository, StorageBackend, SyncBackend,
};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use uuid::Uuid;
```

**What it does** — Tests two layers of the collaborative client (`src/collab/`):
(1) the **pure state machine** (`collab::state::NoteLines`) — diffing a note
body into line ops and replaying ops deterministically, no I/O; (2) an
**end-to-end round trip** through a mock keeplin-srv (in-process axum: REST note
listing, blob endpoints, and a `/api/ws` Join/Welcome/Op relay) between two
`CollabBackend<DbBackend>` stacks speaking the genuine protocol.

**Repeated context** — the *production* server end of the same wire protocol is
exercised in keeplin-srv's own e2e suite. The mock has no `/version` route, so
`CollabBackend::start` runs the warn-and-continue handshake path on purpose
(dedicated handshake tests live in `tests/version_handshake.rs`).

---

## fn token_for

**Identification** — `fn token_for(device: &str) -> String`. Marker
`// md:fn token_for`.

**Code** — complete and verbatim:

```rust
// md:fn token_for
fn token_for(device: &str) -> String {
    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload = engine.encode(format!(r#"{{"device_id":"{device}"}}"#));
    format!("{}.{payload}.sig", engine.encode(r#"{"alg":"none"}"#))
}
```

**What it does** — Forges an unsigned JWT (`header.payload.sig`, URL-safe
base64, no padding) whose payload carries a `device_id` claim — the client only
decodes, never verifies.

**Used by** — `token_device_id_decodes`, `client`.

---

## fn token_device_id_decodes

**Identification** — `#[test]`. Marker `// md:fn token_device_id_decodes`.

**Code** — complete and verbatim:

```rust
// md:fn token_device_id_decodes
#[test]
fn token_device_id_decodes() {
    let id = Uuid::new_v4().to_string();
    assert_eq!(device_id_from_token(&token_for(&id)), Some(id));
    assert_eq!(device_id_from_token("garbage"), None);
}
```

**What it does** — `device_id_from_token` extracts the claim from a forged
token; garbage input yields `None`.

---

## fn diff_roundtrip_materializes_new_body

**Identification** — `#[test]`. Marker
`// md:fn diff_roundtrip_materializes_new_body`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `NoteLines::diff_body` from empty to a 3-line body produces
3 ops and `materialize()` returns the body; a second diff (edit the middle
line, delete the last, append two) also materialises exactly.

---

## fn ops_replay_identically_on_another_mirror

**Identification** — `#[test]`. Marker
`// md:fn ops_replay_identically_on_another_mirror`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Ops collected from three successive diffs on mirror A,
applied in order on a fresh mirror B, converge byte-identically — the
deterministic-replay core assertion.

---

## fn mock_server

**Identification** — `async fn mock_server(note_id: Uuid) -> SocketAddr`. Marker
`// md:fn mock_server`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Minimal in-process stand-in for keeplin-srv on an ephemeral
port: `GET /api/notes` lists one pre-seeded note (POST/PATCH/DELETE accepted and
ignored); `PUT`/`GET /api/resources/:id/data` implement an in-memory blob store
(the out-of-band upload + lazy-download path); `/api/ws` answers `Join` with a
`Welcome` snapshot of the seeded (empty-lines) note, echoes `Cursor` back to the
sender as a `Presence` list, and relays `Op` frames to every **other**
connection (each socket gets a connection sequence number; the broadcast pair
`(from, text)` lets a socket skip its own ops).

**Used by** — the four e2e tests.

---

## fn client

**Identification** — `async fn client(addr: SocketAddr, device: &str) ->
Arc<CollabBackend<DbBackend>>`. Marker `// md:fn client`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A `CollabBackend` over an offline `DbBackend` in a leaked
temp dir, configured with the mock's `http`/`ws` URLs and a forged token, then
`start`ed **with itself as the top of the stack** (no linking/eventing in this
test). `start` is `Result` since the `/version` handshake; the mock's missing
`/version` is the warn-and-continue path.

**Used by** — the four e2e tests.

---

## fn wait_body

**Identification** — `async fn wait_body(backend: &Arc<CollabBackend<DbBackend>>,
id: Uuid, want: &str)`. Marker `// md:fn wait_body`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Polls the client's local note body every 100 ms until it
equals `want`, panicking with the last observed body after ~5 s.

**Used by** — `edits_travel_between_two_daemons`,
`cursor_updates_flow_into_presence`.

---

## fn created_note_body_survives_the_join_welcome

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn created_note_body_survives_the_join_welcome`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Regression: `create_note` used to push the body ops before
the Join's `Welcome`, so a late empty `Welcome` clobbered the local body to
`""`. The fix defers the push to the Welcome reconcile. The test creates a note
with a body, waits 400 ms for the Join/Welcome round trip (the mock does not
echo the sender's own op, so a clobber would be permanent), and asserts the
body survived.

---

## fn edits_travel_between_two_daemons

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn edits_travel_between_two_daemons`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Two clients joined to one note: discovery creates the note
locally on both; A's body edit is diffed into ops, relayed, and materialises on
B; then B's one-line edit converges back on A.

---

## fn resource_blob_uploads_out_of_band_and_downloads_on_read

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn resource_blob_uploads_out_of_band_and_downloads_on_read`.

**Code** — complete and verbatim:

```rust
// md:fn resource_blob_uploads_out_of_band_and_downloads_on_read
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resource_blob_uploads_out_of_band_and_downloads_on_read() {
    let note_id = Uuid::new_v4();
    let addr = mock_server(note_id).await;

    let a = client(addr, "dev-a").await;
    let bytes = b"opaque-ciphertext-bytes".to_vec();
    let resource = a
        .create_resource(
            Resource::new(
                SYSTEM_RESOURCE_NOTE_ID,
                "photo",
                "image/png",
                "photo.png",
                bytes.len() as u64,
            ),
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
```

**What it does** — `create_resource` through the collab stack uploads the
binary to the server (`PUT /api/resources/:id/data`) and the queued
`Change::ResourceCreate` carries `data: None` (blob stripped from the relay).
A second client applying that stripped change has no local blob, so
`read_resource` lazily downloads it from the server — bytes round-trip intact.

---

## fn cursor_updates_flow_into_presence

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn cursor_updates_flow_into_presence`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `CollabHandle::send_cursor` publishes a caret; the mock
echoes a presence list carrying it; polling `handle.presence(note_id)` (up to
~5 s) eventually sees a cursor at the sent column.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `client()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `wait_body()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `token_for()` — defined here (EXTRACTED; file-local)
- `token_device_id_decodes()` — defined here (EXTRACTED; file-local)
- `diff_roundtrip_materializes_new_body()` — defined here (EXTRACTED; file-local)
- `ops_replay_identically_on_another_mirror()` — defined here (EXTRACTED; file-local)
- `mock_server()` — defined here (EXTRACTED; file-local)
- `created_note_body_survives_the_join_welcome()` — defined here (EXTRACTED; file-local)
- `edits_travel_between_two_daemons()` — defined here (EXTRACTED; file-local)
- `resource_blob_uploads_out_of_band_and_downloads_on_read()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/collab/mod.rs` — client of the keeplin-srv collaborative channel (EXTRACTED: references×2; e.g. `CollabBackend`)
- `keeplin-core/src/collab/state.rs` — client line state and body↔lines translation (EXTRACTED: imports_from×1; e.g. `NoteLines`)
- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: references×2; e.g. `DbBackend`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- The state-machine layer is tested pure (no I/O); the e2e layer runs against an in-process mock keeplin-srv — the production server end is covered in keeplin-srv's own e2e suite.
- Deterministic replay (same ops → identical bodies on every mirror) is the core assertion and must stay covered.
- The mock has no `/version` route: `start` exercising the warn-and-continue handshake path is intentional.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `fn token_for` | `// md:fn token_for` |
| 3 | `fn token_device_id_decodes` | `// md:fn token_device_id_decodes` |
| 4 | `fn diff_roundtrip_materializes_new_body` | `// md:fn diff_roundtrip_materializes_new_body` |
| 5 | `fn ops_replay_identically_on_another_mirror` | `// md:fn ops_replay_identically_on_another_mirror` |
| 6 | `fn mock_server` | `// md:fn mock_server` |
| 7 | `fn client` | `// md:fn client` |
| 8 | `fn wait_body` | `// md:fn wait_body` |
| 9 | `fn created_note_body_survives_the_join_welcome` | `// md:fn created_note_body_survives_the_join_welcome` |
| 10 | `fn edits_travel_between_two_daemons` | `// md:fn edits_travel_between_two_daemons` |
| 11 | `fn resource_blob_uploads_out_of_band_and_downloads_on_read` | `// md:fn resource_blob_uploads_out_of_band_and_downloads_on_read` |
| 12 | `fn cursor_updates_flow_into_presence` | `// md:fn cursor_updates_flow_into_presence` |