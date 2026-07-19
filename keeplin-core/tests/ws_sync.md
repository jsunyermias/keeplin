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

```rust
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use keeplin_core::{error::{StorageError, SyncError}, models::{Change, Note},
    storage::{db::DbBackend, HistoryRepository, NoteRepository, SyncBackend},
    sync::run_sync};
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

**What it does** — Binds an ephemeral 127.0.0.1 port and delegates to
`spawn_relay_on`.

**Used by** — most relay tests.

---

## fn spawn_relay_on

**Identification** — `async fn spawn_relay_on(listener: TcpListener) ->
SocketAddr`. Marker `// md:fn spawn_relay_on`.

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

**What it does** — A server-mode `DbBackend` connected to `url` with a test
token (temp dir leaked for the test's duration). Construction performs the
auth handshake; a dead relay is tolerated (starts disconnected, local CRUD
works).

**Used by** — every test.

---

## fn epoch

**Identification** — `fn epoch() -> DateTime<Utc>`. Marker `// md:fn epoch`.

**What it does** — Unix epoch as `DateTime<Utc>` — the "collect everything"
watermark.

---

## fn push

**Identification** — `async fn push(dev: &DbBackend)`. Marker `// md:fn push`.

**What it does** — Collects every local change since epoch and `send_changes`
them to the relay.

---

## fn sync_until

**Identification** — `async fn sync_until(dev: &DbBackend, id: Uuid, want_body:
Option<&str>) -> bool`. Marker `// md:fn sync_until`.

**What it does** — Repeatedly `receive_changes` (each call drains ~100 ms) for
up to ~3 s, applying every received change, until note `id` exists and — when
`want_body` is `Some` — its body matches; returns whether it converged. The
poll loop absorbs the asynchronous accept/forward scheduling without fixed
sleeps.

---

## fn note_create_syncs_between_two_devices

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn note_create_syncs_between_two_devices`.

**What it does** — A creates a note and pushes; B receives it over the
WebSocket and reads the same title/body.

---

## fn update_propagates_and_converges

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn update_propagates_and_converges`.

**What it does** — After the create reaches B, A's update (new title/body,
fresh `updated_at`) is pushed, and B converges to the new body.

---

## fn failed_send_keeps_watermark_and_changes_are_resent_after_recovery

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn failed_send_keeps_watermark_and_changes_are_resent_after_recovery`.

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

**What it does** — `send_changes` on a backend with `server_url = ""` is a
deliberate no-op, not an error: the backend is local-only and there is nowhere
to send to, so local sync cycles (and the daemon's `/api/sync`) keep working.

---

## fn malformed_frame_does_not_abort_receive

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn malformed_frame_does_not_abort_receive`.

**What it does** — A raw WebSocket client sends its auth frame (dropped by the
relay), then a garbage text frame, then a valid `changes` envelope. The relay
forwards both non-auth frames in order; `receive_changes` must skip the garbage
with a warning — not error — and still deliver the valid batch, whose note then
applies on B.

---

## fn spawn_history_server

**Identification** — `async fn spawn_history_server(reply: serde_json::Value)
-> SocketAddr`. Marker `// md:fn spawn_history_server`.

**What it does** — Serves a canned JSON reply at `GET /api/notes/:id/history`.
There is deliberately no `/api/sync` WebSocket: the backend runs offline for
sync, which history must not depend on.

**Used by** — `note_history_prefers_the_server_journal`.

---

## fn note_history_prefers_the_server_journal

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_history_prefers_the_server_journal`.

**What it does** — In server mode history comes from the **server journal**
(every device's changes): the canned reply carries two versions authored by
another device (newest a live entity, oldest a tombstone `entity: null`);
`note_history` returns them — full history including tombstones — even though
the local journal knows nothing about the note.

---

## fn note_history_falls_back_to_the_local_journal

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_history_falls_back_to_the_local_journal`.

**What it does** — Against a WebSocket-only relay (the history GET fails),
`note_history` serves the local `entity_changes` journal — history keeps
working offline.

---

## fn a_404_history_endpoint_is_probed_only_once

**Identification** — `#[tokio::test]`. Marker
`// md:fn a_404_history_endpoint_is_probed_only_once`.

**What it does** — Issue #113: after the server answers the history request
with 404 (an older server without the endpoint), the client latches
"unsupported": across three history reads the server is hit exactly once, the
rest use the local journal.

---

## fn history_is_skipped_when_the_server_capability_is_absent

**Identification** — `#[tokio::test]`. Marker
`// md:fn history_is_skipped_when_the_server_capability_is_absent`.

**What it does** — keeplin#114: when `/version` advertises capabilities
**without** `history`, the client never even sends a history request (server
hit count stays 0 across three reads; local journal used).

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

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

- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: references×3; e.g. `DbBackend`)
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
| 1 | crate doc + imports | `// md:Overview` |
| 2–3 | `fn spawn_relay`, `fn spawn_relay_on` | `// md:fn spawn_relay…` |
| 4–7 | `fn device`, `fn epoch`, `fn push`, `fn sync_until` | `// md:fn <name>` |
| 8–12 | the five relay `#[tokio::test]` fns | `// md:fn <name>` |
| 13 | `fn spawn_history_server` | `// md:fn spawn_history_server` |
| 14–17 | the four history `#[tokio::test]` fns | `// md:fn <name>` |
