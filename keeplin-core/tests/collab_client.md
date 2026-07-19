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

```rust
use std::net::SocketAddr;
use std::sync::Arc;
use keeplin_core::collab::protocol::{CollabClientMsg, CollabServerMsg,
    LineSnapshot, NoteLinesSnapshot};
use keeplin_core::collab::state::NoteLines;
use keeplin_core::collab::{device_id_from_token, CollabBackend, CollabConfig};
use keeplin_core::models::{Change, Note, Resource};
use keeplin_core::storage::{db::DbBackend, NoteRepository, ResourceRepository,
    StorageBackend, SyncBackend};
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

**What it does** — Forges an unsigned JWT (`header.payload.sig`, URL-safe
base64, no padding) whose payload carries a `device_id` claim — the client only
decodes, never verifies.

**Used by** — `token_device_id_decodes`, `client`.

---

## fn token_device_id_decodes

**Identification** — `#[test]`. Marker `// md:fn token_device_id_decodes`.

**What it does** — `device_id_from_token` extracts the claim from a forged
token; garbage input yields `None`.

---

## fn diff_roundtrip_materializes_new_body

**Identification** — `#[test]`. Marker
`// md:fn diff_roundtrip_materializes_new_body`.

**What it does** — `NoteLines::diff_body` from empty to a 3-line body produces
3 ops and `materialize()` returns the body; a second diff (edit the middle
line, delete the last, append two) also materialises exactly.

---

## fn ops_replay_identically_on_another_mirror

**Identification** — `#[test]`. Marker
`// md:fn ops_replay_identically_on_another_mirror`.

**What it does** — Ops collected from three successive diffs on mirror A,
applied in order on a fresh mirror B, converge byte-identically — the
deterministic-replay core assertion.

---

## fn mock_server

**Identification** — `async fn mock_server(note_id: Uuid) -> SocketAddr`. Marker
`// md:fn mock_server`.

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

**What it does** — Polls the client's local note body every 100 ms until it
equals `want`, panicking with the last observed body after ~5 s.

**Used by** — `edits_travel_between_two_daemons`,
`cursor_updates_flow_into_presence`.

---

## fn created_note_body_survives_the_join_welcome

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn created_note_body_survives_the_join_welcome`.

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

**What it does** — Two clients joined to one note: discovery creates the note
locally on both; A's body edit is diffed into ops, relayed, and materialises on
B; then B's one-line edit converges back on A.

---

## fn resource_blob_uploads_out_of_band_and_downloads_on_read

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn resource_blob_uploads_out_of_band_and_downloads_on_read`.

**What it does** — `create_resource` through the collab stack uploads the
binary to the server (`PUT /api/resources/:id/data`) and the queued
`Change::ResourceCreate` carries `data: None` (blob stripped from the relay).
A second client applying that stripped change has no local blob, so
`read_resource` lazily downloads it from the server — bytes round-trip intact.

---

## fn cursor_updates_flow_into_presence

**Identification** — `#[tokio::test(flavor = "multi_thread")]`. Marker
`// md:fn cursor_updates_flow_into_presence`.

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
| 1 | crate doc + imports | `// md:Overview` |
| 2 | `fn token_for` | `// md:fn token_for` |
| 3–5 | the three state-machine `#[test]` fns | `// md:fn <name>` |
| 6 | `fn mock_server` | `// md:fn mock_server` |
| 7 | `fn client` | `// md:fn client` |
| 8 | `fn wait_body` | `// md:fn wait_body` |
| 9–12 | the four e2e `#[tokio::test]` fns | `// md:fn <name>` |
