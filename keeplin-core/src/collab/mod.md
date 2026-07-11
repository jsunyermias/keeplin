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
| `CollabBackend::start(top)` | spawn the connection task; `top` must be the **outermost** backend so remote writes flow through every decorator once |
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

## Related files

- `collab/protocol.md` — the wire types this module sends/receives.
- `collab/state.md` — `NoteLines`: body↔lines materialise/apply/diff.
- `keeplin-daemon/src/rest.md` — the presence/cursor endpoints backed by `CollabHandle`.
- `keeplin-daemon/src/main.rs` — where the decorator is placed in the stack and `start`ed.
- `keeplin-core/src/storage/note_log.md` — the version-vector resolution this mirrors client-side.
