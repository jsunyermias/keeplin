# `collab/mod.rs` — client of the keeplin-srv collaborative channel

## Purpose

The daemon's client for keeplin-srv's real-time note channel. Frontends talk to this daemon; the
daemon — through [`CollabBackend`] — talks to keeplin-srv, which stores every note (lines +
versioned order + metadata) and is the durable source of truth. This module is a **storage
decorator**: it turns local note writes into line operations pushed over a WebSocket, applies remote
ops/snapshots back into the local database, and exposes presence/cursors to the daemon's surfaces.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `CollabConfig` | struct | connection settings: `api_url`, `ws_url`, per-device `token` |
| `CollabBackend<B>` | struct | `StorageBackend` decorator wrapping inner `B`; sits **below** `LinkingBackend`/`EventBackend` |
| `CollabHandle` | struct | cloneable, un-generic view for REST/gRPC: read presence, publish this device's cursor |
| `Shared` | struct (private) | the per-session state shared by the backend, handle, and connection task |

`CollabBackend` implements every `StorageBackend` sub-trait; only `NoteRepository` and `SyncBackend`
carry collab logic, the rest delegate to `inner`.

## Public API

| Function | Description |
|----------|-------------|
| `CollabBackend::new(inner, cfg)` | build the decorator; fails if the token has no `device_id` claim |
| `CollabBackend::handle()` | a cloneable `CollabHandle` for the daemon's surfaces |
| `CollabBackend::start(top) -> Result<(), StorageError>` | run the `GET /version` protocol handshake (`src/compat.rs`), then spawn the connection task; `top` must be the **outermost** backend so remote writes flow through every decorator once. **Incompatible server → `Err(StorageError::InvalidState)` with an actionable message and the connection task is never spawned** (no sync attempted); a server without a usable `/version` warns and proceeds (backward compatible); compatible logs the negotiated protocol + capabilities |
| `CollabHandle::presence(note_id)` | latest presence list the server broadcast (empty if none) |
| `CollabHandle::send_cursor(note_id, cursor)` | queue this device's caret; server fans it out |
| `CollabHandle::proxy_request(method, path, body)` | forward a permission-management request (share/transfer/list/revoke) to keeplin-srv (the authority) and return its `(status, json)`; the daemon's REST layer proxies its permission endpoints through this |
| `device_id_from_token(token)` | extract the `device_id` claim without verifying (the server verifies) |

## The decorator — write path

A local note write flows `frontend → LinkingBackend → EventBackend → CollabBackend → inner`. On the
way through, `CollabBackend`:

- **create_note**: create locally, then `POST /api/notes` (keeping the local id — a `409` is fine,
  the note exists on another device), `PATCH` metadata, mark the note **pending** and `Join`. The
  body is **not** pushed here — it is reconciled when the Join's `Welcome` arrives (see below).
- **update_note**: update locally, `PATCH` metadata only when title/notebook/todo fields changed,
  then diff the body into ops — unless the note is still **pending** its first `Welcome`, in which
  case the push is left to that reconcile.
- **delete_note**: delete locally, then `DELETE /api/notes/:id`; drop the in-memory mirror.
- The **body** is diffed into `LineOp`s (see `state.md`) and pushed over the socket; **title and
  metadata** go over REST. Notes are the collab channel's; notebooks/tags/resources delegate to
  `inner` unchanged.

### Pending push: reconcile on `Welcome`, don't clobber

A note with local content the server has not seen yet (freshly created, or edited before its first
`Welcome`) is held in a **`pending_push`** set instead of being pushed eagerly. When the note's
`Welcome` arrives, the handler diffs the **current local body against the snapshot** and pushes that
difference. This fixes an ordering bug: pushing before the `Welcome` let a late (empty) snapshot
overwrite the local body to blank. A note the client merely **caches** (discovered, not pending) is
not reconciled — it takes the server's body as-is, so a fresh device never diffs its empty placeholder
against real server content (which would delete it).

## The decorator — `SyncBackend` split

Note `Change`s (`NoteCreate/Update/Delete`) are **filtered out** of the relay path: `get_changes_since`
drops them and `apply_change` ignores them. The collab channel owns notes; the device relay
(`/api/sync`) keeps carrying notebooks, tags and resources. This is the split that keeps a note from
travelling both paths and double-applying.

### Resource binaries: out-of-band, never in the journal

`get_changes_since` also **strips the binary** from every relayed `ResourceCreate`
(`strip_resource_blob` → `data: None`): keeplin-srv holds resource blobs, so they must not
bloat the relay journal. The bytes travel `PUT /api/resources/:id/data` instead:

- `create_resource` first **eagerly pushes the blob-stripped `ResourceCreate` over the relay**
  (`inner.send_changes`), because the server rejects a blob upload (`404`) until the resource's
  metadata has been materialised from the journal — the periodic sync cycle would always lose
  that race for a brand-new resource. The periodic cycle may re-send the same change later;
  server materialisation is version-vector-idempotent, so the duplicate row is harmless.
- `upload_blob` then `PUT`s the bytes, retrying briefly with backoff on a non-success status
  (materialisation is asynchronous after the push). A transport failure or exhausted retries is
  logged and the metadata still syncs — best effort; the blob can be re-uploaded by a later
  create/replace.
- `read_resource` falls back to `GET /api/resources/:id/data` when the local cache has no bytes
  but the metadata says `size > 0` (blob authored on another device).

## Echo suppression

Remote state is written through `top` (the full stack) so links re-derive and the live feed fires.
To stop those writes from being diffed *back* into ops, `Shared.suppress` holds the note id for the
duration of the server-driven write; `create_note`/`update_note`/`delete_note` early-return when the
id is suppressed. Without this, every remote op would echo back to the server as a fresh local edit.

## The connection task

`run_connection` keeps one WebSocket alive forever with a capped exponential backoff (1s → 30s). Per
connection:

- The token travels in the **`Authorization` header**, never the URL (query strings leak into proxy
  and access logs).
- On connect and every `REDISCOVER_EVERY` (15s), `discover_and_join` calls `GET /api/notes` and joins
  any note not yet joined — so notes created on another device, or newly **shared** with this user,
  get picked up without a reconnect. Unknown notes are created locally with an empty body; the
  `Welcome` snapshot fills them in.
- Every reconnect re-discovers and re-joins, and state is rebuilt from each `Welcome` — there is no
  durable op log on the client.
- Inbound `Welcome`/`Op` rebuild the mirror and re-materialise the body; `Presence` updates the map
  `CollabHandle` serves; `Error` is logged.

## Design notes

- **`CollabHandle` is deliberately un-generic** (`Arc<Shared>`, no `B`): the REST/gRPC layer can read
  presence and send cursors without naming the storage type behind the decorator.
- **Metadata over REST, body over WS**: title/todo/notebook are single-value fields resolved
  server-side; only the body needs the line-level op protocol, so only it pays that cost.
- **Failures are logged, not fatal**: the local row already holds the change and the server is the
  durable truth, so a failed `POST`/`PATCH` is retried implicitly by the next edit rather than
  blocking the write.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `Shared` — defined here (EXTRACTED; 85 cross-file edge(s))
- `CollabBackend<B>` — defined here (EXTRACTED; 6 cross-file edge(s))
- `CollabBackend` — defined here (EXTRACTED; 5 cross-file edge(s))
- `.note_history()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `.notebook_history()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `.apply_from_server()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `CollabHandle` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.start()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.create_note()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.read_note()` — defined here (EXTRACTED; 2 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/collab/protocol.rs` — collaborative channel wire types (EXTRACTED: references×8; e.g. `CollabClientMsg`, `PresenceInfo`, `Cursor`)
- `keeplin-core/src/collab/state.rs` — client line state and body↔lines translation (EXTRACTED: imports_from×1, references×1; e.g. `NoteLines`)
- `keeplin-core/src/error.rs` — error types (EXTRACTED: imports_from×1, references×41; e.g. `StorageError`)
- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: references×31; e.g. `Note`, `Notebook`, `Tag`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: implements×6, references×5; e.g. `StorageBackend`, `NotebookRepository`, `NoteRepository`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/tests/collab_client.rs` — collaborative client tests (state machine + mock server e2e) (EXTRACTED: references×2; e.g. `client()`, `wait_body()`)
- `keeplin-daemon/src/main.rs` — daemon entry point (EXTRACTED: imports_from×1, references×3; e.g. `collab_config()`, `run_server_with()`, `collab_starter()`)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: references×82; e.g. `add_link()`, `add_note_tag()`, `auth_mw()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- `CollabBackend` owns note traffic; the relay must never carry note `Change`s (filtered in `get_changes_since`/`apply_change`) or a note would travel both paths and double-apply.
- Server-driven writes go through `top` (the outermost decorator) with the note id in the `suppress` set, so they are never diffed back into ops (no echo).
- A note pending its first `Welcome` is reconciled against the snapshot, never pushed eagerly — a late empty `Welcome` must not clobber local content.
- Resource binaries never ride the relay journal: `create_resource` eagerly relays the blob-stripped metadata, then uploads out-of-band with status-checked retries.
- `start` runs the `compat` handshake first; an incompatible server means no connection task is spawned (no sync attempted).

## Related files

- `collab/protocol.md` — the wire types this module sends/receives.
- `collab/state.md` — `NoteLines`: body↔lines materialise/apply/diff.
- `keeplin-daemon/src/rest.md` — the presence/cursor endpoints backed by `CollabHandle`.
- `keeplin-daemon/src/main.rs` — where the decorator is placed in the stack and `start`ed.
- `keeplin-core/src/storage/note_log.md` — the version-vector resolution this mirrors client-side.

> `create_note`'s `POST` and `patch_meta`'s `PATCH` check the HTTP **status**, not just transport success: a non-2xx (other than the expected `409` on create) is logged as a rejection rather than silently treated as delivered (issue #112).
