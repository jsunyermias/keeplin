# `tests/ws_sync.rs` — end-to-end WebSocket sync tests

Self-contained companion for `keeplin-core/tests/ws_sync.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file
must be able to understand it without opening anything else, so project-wide
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
```

**What it does** — The other suites drive `DbBackend` only offline. This one
stands up a real (in-process) **WebSocket relay** — a minimal stand-in for the
production sync server — and drives two `DbBackend` instances through the
genuine wire protocol: the `auth` handshake performed on construction,
`send_changes` (serialising a `changes` envelope onto the socket), the relay
forwarding the batch to the **other** device, and `receive_changes` (draining
and parsing incoming frames). It proves a change actually travels between two
devices over a socket, not just through the local database. A second section
covers **server-side history** (Front D stage 2): journal preference, offline
fallback, the 404 latch, and capability-gated skipping.

---

## fn spawn_relay

**Identification** — `async fn spawn_relay() -> SocketAddr`. Marker
`// md:fn spawn_relay`.

**Code** — complete and verbatim:

```rust
// md:fn spawn_relay
async fn spawn_relay() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    spawn_relay_on(listener).await
}
```

**What it does** — Binds an ephemeral 127.0.0.1 port and delegates to
`spawn_relay_on`.

**Used by** — most relay tests.

---

## fn spawn_relay_on

**Identification** — `async fn spawn_relay_on(listener: TcpListener) ->
SocketAddr`. Marker `// md:fn spawn_relay_on`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The relay proper, mimicking the production hub: accepts any
number of clients, **discards each client's first frame** (the `auth`
handshake), and forwards every subsequent text frame (a `changes` batch) to
**all other** connected clients — never echoing to the sender (a broadcast
channel tagged with a per-connection id). Taking an already-bound listener lets
a test reserve an address, run a device against it while nothing is listening,
and only then bring the relay up on that same address — the "relay was down,
then recovered" scenario.

**Used by** — `spawn_relay`, the relay-recovery test.

---

## fn device

**Identification** — `async fn device(url: &str) -> DbBackend`. Marker
`// md:fn device`.

**Code** — complete and verbatim:

```rust
// md:fn device
async fn device(url: &str) -> DbBackend {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(path, url, "test-token").await.unwrap()
}
```

**What it does** — A server-mode `DbBackend` connected to `url` with a test
token (temp dir leaked for the test's duration). Construction performs the
auth handshake; a dead relay is tolerated (starts disconnected, local CRUD
works).

**Used by** — every test.

---

## fn epoch

**Identification** — `fn epoch() -> DateTime<Utc>`. Marker `// md:fn epoch`.

**Code** — complete and verbatim:

```rust
// md:fn epoch
fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(0, 0).unwrap()
}
```

**What it does** — Unix epoch as `DateTime<Utc>` — the "collect everything"
watermark.

---

## fn push

**Identification** — `async fn push(dev: &DbBackend)`. Marker `// md:fn push`.

**Code** — complete and verbatim:

```rust
// md:fn push
async fn push(dev: &DbBackend) {
    let changes = dev.get_changes_since(epoch()).await.unwrap();
    dev.send_changes(changes).await.unwrap();
}
```

**What it does** — Collects every local change since epoch and `send_changes`
them to the relay.

---

## fn sync_until

**Identification** — `async fn sync_until(dev: &DbBackend, id: Uuid, want_body:
Option<&str>) -> bool`. Marker `// md:fn sync_until`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Repeatedly `receive_changes` (each call drains ~100 ms) for
up to ~3 s, applying every received change, until note `id` exists and — when
`want_body` is `Some` — its body matches; returns whether it converged. The
poll loop absorbs the asynchronous accept/forward scheduling without fixed
sleeps.

---

## fn note_create_syncs_between_two_devices

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn note_create_syncs_between_two_devices`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A creates a note and pushes; B receives it over the
WebSocket and reads the same title/body.

---

## fn update_propagates_and_converges

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn update_propagates_and_converges`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — After the create reaches B, A's update (new title/body,
fresh `updated_at`) is pushed, and B converges to the new body.

---

## fn failed_send_keeps_watermark_and_changes_are_resent_after_recovery

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn failed_send_keeps_watermark_and_changes_are_resent_after_recovery`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The watermark-safety contract: with the relay address
reserved but nothing listening, a `run_sync` cycle fails with
`SyncError::Storage(StorageError::WebSocket(_))` and the last-sync watermark
stays at epoch — returning `Ok` from an undelivered send would advance the
watermark past the batch and drop it from every future sync (the bug this
guards against). The relay then comes up on the **same** address; the next
cycle succeeds, advances the watermark, and the queued change reaches device B.

---

## fn send_without_configured_relay_is_a_noop

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn send_without_configured_relay_is_a_noop`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `send_changes` on a backend with `server_url = ""` is a
deliberate no-op, not an error: the backend is local-only and there is nowhere
to send to, so local sync cycles (and the daemon's `/api/sync`) keep working.

---

## fn malformed_frame_does_not_abort_receive

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn malformed_frame_does_not_abort_receive`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A raw WebSocket client sends its auth frame (dropped by the
relay), then a garbage text frame, then a valid `changes` envelope. The relay
forwards both non-auth frames in order; `receive_changes` must skip the garbage
with a warning — not error — and still deliver the valid batch, whose note then
applies on B.

---

## fn spawn_history_server

**Identification** — `async fn spawn_history_server(reply: serde_json::Value)
-> SocketAddr`. Marker `// md:fn spawn_history_server`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Serves a canned JSON reply at `GET /api/notes/:id/history`.
There is deliberately no `/api/sync` WebSocket: the backend runs offline for
sync, which history must not depend on.

**Used by** — `note_history_prefers_the_server_journal`.

---

## fn note_history_prefers_the_server_journal

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_history_prefers_the_server_journal`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — In server mode history comes from the **server journal**
(every device's changes): the canned reply carries two versions authored by
another device (newest a live entity, oldest a tombstone `entity: null`);
`note_history` returns them — full history including tombstones — even though
the local journal knows nothing about the note.

---

## fn note_history_falls_back_to_the_local_journal

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_history_falls_back_to_the_local_journal`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Against a WebSocket-only relay (the history GET fails),
`note_history` serves the local `entity_changes` journal — history keeps
working offline.

---

## fn a_404_history_endpoint_is_probed_only_once

**Identification** — `#[tokio::test]`. Marker
`// md:fn a_404_history_endpoint_is_probed_only_once`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Issue #113: after the server answers the history request
with 404 (an older server without the endpoint), the client latches
"unsupported": across three history reads the server is hit exactly once, the
rest use the local journal.

---

## fn history_is_skipped_when_the_server_capability_is_absent

**Identification** — `#[tokio::test]`. Marker
`// md:fn history_is_skipped_when_the_server_capability_is_absent`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — keeplin#114: when `/version` advertises capabilities
**without** `history`, the client never even sends a history request (server
hit count stays 0 across three reads; local journal used).

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

<!-- Data source: CI artifact or local graphify-out/graph.json from this exact commit.
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. Never
     present inference as fact. -->
**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `device()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `push()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `sync_until()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `failed_send_keeps_watermark_and_changes_are_resent_after_recovery()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `spawn_relay()` — defined here (EXTRACTED; file-local)
- `spawn_relay_on()` — defined here (EXTRACTED; file-local)
- `epoch()` — defined here (EXTRACTED; file-local)
- `note_create_syncs_between_two_devices()` — defined here (EXTRACTED; file-local)
- `update_propagates_and_converges()` — defined here (EXTRACTED; file-local)
- `send_without_configured_relay_is_a_noop()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/db/mod.rs` — defines `DbBackend` (INFERRED: the test reaches it through the fully-qualified `keeplin_core::storage::db::DbBackend`, which the AST pass does not link)
- `keeplin-core/src/sync/engine.rs` — SyncEngine (EXTRACTED: calls×1; e.g. `run_sync()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Drives the genuine wire protocol against an in-process relay: auth-first frame, `changes` envelopes, no echo to the sender.
- Undelivered batches must fail the send (watermark unchanged) and be re-sent after reconnect — offline resilience is the point of the suite.
- The capability-negotiation tests (`/version` without `history`, 404-latch) must keep asserting the probe happens at most once.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `fn spawn_relay` | `// md:fn spawn_relay` |
| 3 | `fn spawn_relay_on` | `// md:fn spawn_relay_on` |
| 4 | `fn device` | `// md:fn device` |
| 5 | `fn epoch` | `// md:fn epoch` |
| 6 | `fn push` | `// md:fn push` |
| 7 | `fn sync_until` | `// md:fn sync_until` |
| 8 | `fn note_create_syncs_between_two_devices` | `// md:fn note_create_syncs_between_two_devices` |
| 9 | `fn update_propagates_and_converges` | `// md:fn update_propagates_and_converges` |
| 10 | `fn failed_send_keeps_watermark_and_changes_are_resent_after_recovery` | `// md:fn failed_send_keeps_watermark_and_changes_are_resent_after_recovery` |
| 11 | `fn send_without_configured_relay_is_a_noop` | `// md:fn send_without_configured_relay_is_a_noop` |
| 12 | `fn malformed_frame_does_not_abort_receive` | `// md:fn malformed_frame_does_not_abort_receive` |
| 13 | `fn spawn_history_server` | `// md:fn spawn_history_server` |
| 14 | `fn note_history_prefers_the_server_journal` | `// md:fn note_history_prefers_the_server_journal` |
| 15 | `fn note_history_falls_back_to_the_local_journal` | `// md:fn note_history_falls_back_to_the_local_journal` |
| 16 | `fn a_404_history_endpoint_is_probed_only_once` | `// md:fn a_404_history_endpoint_is_probed_only_once` |
| 17 | `fn history_is_skipped_when_the_server_capability_is_absent` | `// md:fn history_is_skipped_when_the_server_capability_is_absent` |