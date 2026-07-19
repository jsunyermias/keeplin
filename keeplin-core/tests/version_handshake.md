# `tests/version_handshake.rs` — startup protocol handshake tests

Self-contained companion for `keeplin-core/tests/version_handshake.rs`. It documents
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::{routing::get, Json, Router};
use keeplin_core::collab::{CollabBackend, CollabConfig};
use keeplin_core::compat::PROTOCOL_VERSION;
use keeplin_core::models::Note;
use keeplin_core::storage::db::DbBackend;
use keeplin_core::storage::{HistoryRepository, NoteRepository, StorageBackend};
use tokio::net::TcpListener;
```

**What it does** — Integration tests for the `GET /version` protocol handshake
(`src/compat.rs`) as wired into the two connect points: `DbBackend::new` (relay)
and `CollabBackend::start` (collaborative session). Fake keeplin-srv `/version`
endpoints (in-process axum servers on ephemeral ports) drive the three
contractual client behaviours: **compatible** `protocol_version` → construction
succeeds and the capability cache is primed (`/version` not fetched again);
**incompatible** → construction fails loudly with an actionable message before
any sync is attempted; **missing** `/version` (older server) → warn and
continue, exactly as before the handshake existed.

**Repeated context** — `keeplin_core::compat::PROTOCOL_VERSION = 1`;
compatibility is an **exact match**. These behaviours are cross-repo API (the
keeplin-srv `tests/integration.rs` pins the server end of the same wire
contract) and must not regress.

---

## fn spawn_version_server

**Identification** — `async fn spawn_version_server(protocol_version:
Option<u32>, hits: Arc<AtomicU64>) -> SocketAddr`. Marker
`// md:fn spawn_version_server`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Serves a canned `/version` reply (only when
`protocol_version` is `Some`; each hit bumps the counter) with
`name`/`version`/`protocol_version`/`capabilities: ["history"]`, plus an empty
`GET /api/notes/:id/history` endpoint so a client with a primed capability cache
can exercise a follow-up REST call without refetching `/version`. Binds an
ephemeral 127.0.0.1 port and serves on a spawned task.

**Used by** — all four tests.

---

## fn fake_token

**Identification** — `fn fake_token() -> String`. Marker `// md:fn fake_token`.

**Code** — complete and verbatim:

```rust
// md:fn fake_token
fn fake_token() -> String {
    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload = engine.encode(r#"{"device_id":"dev-1"}"#);
    format!("{}.{payload}.sig", engine.encode(r#"{"alg":"none"}"#))
}
```

**What it does** — A JWT-shaped token (`header.payload.sig`, URL-safe base64,
no padding) whose payload carries a `device_id` claim. `CollabBackend::new`
extracts the claim **without verifying** — only the server verifies signatures —
so an unsigned fake is enough.

**Used by** — `collab_start_applies_the_same_rule`.

---

## fn db_path

**Identification** — `fn db_path() -> std::path::PathBuf`. Marker
`// md:fn db_path`.

**Code** — complete and verbatim:

```rust
// md:fn db_path
fn db_path() -> std::path::PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dev.db");
    std::mem::forget(dir);
    path
}
```

**What it does** — A fresh LibSQL path (`dev.db`) inside a leaked tempdir
(`std::mem::forget` keeps the directory alive for the test's lifetime).

**Used by** — all four tests.

---

## fn compatible_version_connects_and_primes_capabilities

**Identification** — `#[tokio::test]`. Marker
`// md:fn compatible_version_connects_and_primes_capabilities`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Against a server answering `PROTOCOL_VERSION`:
`DbBackend::new` succeeds and `/version` was fetched exactly once; a later
`note_history` read consults the capability cache primed at startup, so the hit
counter **stays at 1** (no refetch on capability checks).

---

## fn incompatible_version_fails_construction_loudly

**Identification** — `#[tokio::test]`. Marker
`// md:fn incompatible_version_fails_construction_loudly`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Against `PROTOCOL_VERSION + 7`: `DbBackend::new` fails, and
the message contains "incompatible", the server's protocol number, and
"upgrade" (actionable: naming which side to bump). No sync is attempted.

---

## fn missing_version_warns_and_continues

**Identification** — `#[tokio::test]`. Marker
`// md:fn missing_version_warns_and_continues`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Against a server with no `/version` route (404 — an older
keeplin-srv): construction succeeds and local CRUD is fully usable
(offline-capable client, behaviour unchanged from before the handshake).

---

## fn collab_start_applies_the_same_rule

**Identification** — `#[tokio::test]`. Marker
`// md:fn collab_start_applies_the_same_rule`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The collaborative session start applies the same three-way
rule: with an incompatible server (`PROTOCOL_VERSION + 1`),
`CollabBackend::start` returns `Err` containing "incompatible" (the connection
task is never spawned); with a missing `/version`, `start` returns `Ok`
(warn and continue). Both cases run over a local-only `DbBackend` (empty
`server_url`) wrapped in `CollabBackend` with a `fake_token`.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `spawn_version_server()` — defined here (EXTRACTED; file-local)
- `fake_token()` — defined here (EXTRACTED; file-local)
- `db_path()` — defined here (EXTRACTED; file-local)
- `compatible_version_connects_and_primes_capabilities()` — defined here (EXTRACTED; file-local)
- `incompatible_version_fails_construction_loudly()` — defined here (EXTRACTED; file-local)
- `missing_version_warns_and_continues()` — defined here (EXTRACTED; file-local)
- `collab_start_applies_the_same_rule()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Pins the three-way handshake contract (compatible / incompatible / missing) for BOTH connect points; these behaviours are cross-repo API and must not regress.
- The compatible case must fetch `/version` exactly once (startup priming; no refetch on capability checks).

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `fn spawn_version_server` | `// md:fn spawn_version_server` |
| 3 | `fn fake_token` | `// md:fn fake_token` |
| 4 | `fn db_path` | `// md:fn db_path` |
| 5 | `fn compatible_version_connects_and_primes_capabilities` | `// md:fn compatible_version_connects_and_primes_capabilities` |
| 6 | `fn incompatible_version_fails_construction_loudly` | `// md:fn incompatible_version_fails_construction_loudly` |
| 7 | `fn missing_version_warns_and_continues` | `// md:fn missing_version_warns_and_continues` |
| 8 | `fn collab_start_applies_the_same_rule` | `// md:fn collab_start_applies_the_same_rule` |