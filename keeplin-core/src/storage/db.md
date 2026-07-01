# `storage/db.rs` — DbBackend (LibSQL + WebSocket storage)

## Purpose

`DbBackend` provides server-connected storage. It uses a local LibSQL database (SQLite
format, opened via the `libsql` crate with `feature = "core"`) as a write-ahead cache and
connects to a central synchronisation server over a plain WebSocket. Local writes are
committed instantly and recorded in the `entity_changes` append-only journal; the
WebSocket is used to push those changes to the server and pull changes from other devices.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `DbBackend` | struct | LibSQL + WebSocket `StorageBackend` implementation |
| `WsStream` | type alias | `WebSocketStream<MaybeTlsStream<TcpStream>>` — the WebSocket connection type |

## Database schema

All tables are created idempotently by `run_migrations` on first connection.

| Table | Purpose |
|-------|---------|
| `notes` | Note rows: soft-delete (`deleted_at`), `alias`, `bookmarks`/`links` (JSON), and the conflict-resolution `vv` (JSON version vector) + `last_writer` |
| `notebooks` | Notebook rows with soft-delete column, `alias`, and `vv`/`last_writer` |
| `tags` | Tag rows with soft-delete column and `vv`/`last_writer` |
| `note_tags` | Many-to-many association (PK `(note_id, tag_id)`), **versioned**: `updated_at`, `deleted_at` (tombstone), `vv`, `last_writer` so add/remove converge like other entities |
| `note_links` | Projection of each note's resolved outgoing links (see below) — powers indexed backlinks |
| `resources` | Resource metadata + BLOB (`data` column), soft-delete (`deleted_at`), and `vv`/`last_writer` so deletes converge like other entities |
| `sync_state` | Key-value store for the last-sync timestamp |
| `device` | Single-row table holding the stable device UUID |
| `entity_changes` | Append-only change journal (see below) |

The `alias`, `bookmarks`, and `links` columns are added by `add_column_if_missing` after the
`CREATE TABLE IF NOT EXISTS` statements, so pre-existing databases gain them without a manual
migration. There is deliberately **no `UNIQUE` index on `alias`**: under at-rest encryption the
stored alias is per-write ciphertext (so an index could not detect duplicates anyway), and a
hard constraint would reject a duplicate alias arriving through sync, breaking the sync cycle.
Alias uniqueness is instead enforced in `LinkingBackend` against decrypted values.

### `note_links` table — indexed backlinks

```sql
CREATE TABLE note_links (
    source_note_id TEXT NOT NULL,   -- the note whose body contains the link
    target_note_id TEXT NOT NULL,   -- the resolved destination note
    PRIMARY KEY (source_note_id, target_note_id)
);
CREATE INDEX idx_note_links_target ON note_links(target_note_id);
```

`refresh_note_links(note)` rebuilds a note's rows on every note write (create/update **and**
applied sync change): it deletes the note's existing `source_note_id` rows and re-inserts one
row per link that has a resolved `target_note_id`. This lets `note_backlinks` answer "who links
to note X?" with an indexed lookup + cursor `LIMIT` instead of the trait's default full scan.

### `entity_changes` table

```sql
CREATE TABLE entity_changes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_type TEXT NOT NULL,   -- "note", "notebook", "tag", "note_tag", "resource"
    entity_id   TEXT NOT NULL,   -- UUID of the affected entity
    operation   TEXT NOT NULL,   -- "create", "update", "delete", "add", "remove"
    changed_at  TEXT NOT NULL,   -- RFC-3339 UTC timestamp
    data        TEXT             -- JSON payload; NULL for deletes; includes "_data_b64" for resources
);
CREATE INDEX idx_entity_changes_changed_at ON entity_changes(changed_at);
```

#### `data` column for resources

When a resource is created, `data` contains the full JSON serialisation of the
`Resource` metadata **plus** an extra key `"_data_b64"` holding the Base64-encoded
binary payload. This allows `row_to_change` to reconstruct a
`Change::ResourceCreate { resource, data: Some(bytes) }` that carries the binary inline,
so the receiving device can store the file without a separate gRPC call.

## Public API

### `DbBackend::new(db_path, server_url, auth_token) -> Result<Self, StorageError>`
**What it does:** Opens (or creates) a LibSQL database at `db_path`, runs all migrations,
reads or generates the device ID, and attempts to open a WebSocket connection to
`server_url`. If `server_url` is empty or the connection fails, the backend starts
disconnected: all local CRUD operations still work. The two cases then differ for sync:
with an **empty `server_url`** (no relay configured) `send_changes` is a deliberate no-op
and `receive_changes` returns empty; with a **configured but unreachable relay**,
`send_changes` of a non-empty batch returns `StorageError::WebSocket` so the sync cycle
fails and its watermark does not advance — the changes are re-sent once the relay is
reachable (see "Sending changes" below).  
**Parameters:**
- `db_path` — filesystem path to the `.db` file
- `server_url` — WebSocket URL such as `ws://host:port/sync`; empty string = offline
- `auth_token` — bearer token sent as the first WebSocket message after connecting  
**Returns:** A ready-to-use backend.  
**Errors:** `StorageError::Database` if migrations fail; `StorageError::Io` if the path
is not writable.

All other methods implement `StorageBackend` — see `storage/backend.md`.

## Change journal — `record_change`

Every **local-origin** mutating method (`create_note`, `update_note`, `delete_note`, and the
equivalents for the other entities) calls `record_change(entity_type, entity_id, operation,
data)` after a successful database write. This inserts one row into `entity_changes` stamped
with the current UTC timestamp. `get_changes_since(since)` then reads rows from this table with
`changed_at > since`, converting each to a `Change` via `row_to_change`, and `send_changes`
pushes them to the relay.

### `apply_change` does **not** journal — and that is deliberate

`apply_change` (the path that ingests **remote** changes pulled from the relay) writes the row
but does **not** call `record_change`. The invariant is: *the journal holds only changes that
originated on this device.* The sync server is a broadcast relay — it already forwards each
device's change to **every** other peer (see the relay in `tests/ws_sync.rs`, which forwards
to all `sender != self`) — so a device never needs to re-propagate a change it merely received.
Re-journaling applied changes would make every device re-send every change it saw, producing
redundant echo traffic for no benefit. (This trades away multi-hop/mesh re-propagation, which
the broadcast topology does not need.)

## WebSocket protocol

### Connection and authentication
After the TCP handshake, the client immediately sends a JSON message:
```json
{"type": "auth", "token": "<auth_token>"}
```
The server is expected to validate the token. No formal protocol is specified for the
server response; the client proceeds regardless.

### Sending changes (`send_changes`)
The client serialises the `Vec<Change>` as a JSON object:
```json
{"batch_id": "<uuid-v4>", "changes": [...]}
```
The `batch_id` is a fresh UUID generated per call; servers that implement deduplication
can use it to ignore duplicate batches from a retrying client.

Retry strategy: up to three retries with exponential backoff (2 s, 4 s, 8 s). Before
each retry, `ensure_ws()` is called to re-establish the WebSocket connection if it
dropped. If all retries fail — or the connection cannot be (re-)established at all, which
fails fast without sleeping through the backoff — `StorageError::WebSocket` is returned.

**An undeliverable batch must error, never silently succeed.** `run_sync` only advances
the last-sync watermark after `send_changes` returns `Ok`; if the send were a no-op while
disconnected, the watermark would move past changes the relay never received and
`get_changes_since` would skip them on every future cycle — a permanent, silent sync gap.
By erroring instead, the failed cycle leaves the watermark unchanged and the same batch is
re-collected and re-sent on the next cycle (covered end-to-end by
`failed_send_keeps_watermark_and_changes_are_resent_after_recovery` in
`tests/ws_sync.rs`). The one exception is an **empty `server_url`**: no relay is
configured, the backend is deliberately local-only, and skipping the send is correct.

### Receiving changes (`receive_changes`)
The client reads all available WebSocket messages in a non-blocking loop, stopping
after 100 ms of silence (`drain_timeout = 100 ms`). Each text message is deserialised
as a `Vec<Change>` and appended to the result. Binary messages, errors, and malformed
text frames are ignored (logged as warnings) — one bad frame from the relay must not
abort the sync cycle or block the well-formed batches behind it.

## `apply_change` — all 13 variants

| Variant | SQL |
|---------|-----|
| `NoteCreate` | `INSERT OR REPLACE INTO notes …` (shares the arm with `NoteUpdate`) |
| `NoteUpdate` | `INSERT OR REPLACE INTO notes …` |
| `NoteDelete` | `UPDATE notes SET deleted_at=?, updated_at=?, vv=?, last_writer=? WHERE id = ?` |
| `NotebookCreate` | `INSERT OR REPLACE INTO notebooks …` |
| `NotebookUpdate` | `INSERT OR REPLACE INTO notebooks …` |
| `NotebookDelete` | `UPDATE notebooks SET deleted_at=?, updated_at=?, vv=?, last_writer=? WHERE id = ?` |
| `TagCreate` | `INSERT OR REPLACE INTO tags …` |
| `TagUpdate` | `INSERT OR REPLACE INTO tags …` |
| `TagDelete` | `UPDATE tags SET deleted_at=?, updated_at=?, vv=?, last_writer=? WHERE id = ?` |
| `NoteTagAdd` | version-vector `resolve`, then `INSERT OR REPLACE` the present state (`deleted_at` NULL) |
| `NoteTagRemove` | version-vector `resolve`, then `INSERT OR REPLACE` a tombstone (`deleted_at` set) |
| `ResourceCreate` | `INSERT OR IGNORE INTO resources (…, data) VALUES (…, ?)` with `data = payload.unwrap_or_default()` |
| `ResourceDelete` | version-vector `resolve`, then `UPDATE resources SET deleted_at=?, vv=?, last_writer=? WHERE id = ?` (soft-delete tombstone; BLOB retained) |

All operations are idempotent by design. The create/update/delete arms for notes, notebooks,
and tags are guarded by **version-vector conflict resolution** (`incoming_wins` → `resolve`,
see `note_log.md`): the write is skipped unless the incoming `(vv, updated_at, last_writer)`
wins against the stored row's. `resolve` applies a strictly-dominating incoming write, keeps a
strictly-dominating local one, and breaks a genuine *concurrent* conflict with a deterministic
`(updated_at, device_id)` tiebreak. This replaced the old bare-`updated_at` last-write-wins,
which **diverged permanently** when two edits shared a timestamp (each device kept its own).

Each local write stamps the row's `vv`/`last_writer`: `next_local_vv` loads the current vector
and increments this device's component. Deletes carry the tombstone's own `vv`/`last_writer` in
the journal `data` (see `tombstone_data`), so a tombstone competes in `resolve` exactly like an
edit — a stale delete never overrides a newer edit, and a causal edit after a delete revives.

The note create/update arm wraps the resolve check + `INSERT OR REPLACE` + `refresh_note_links`
in a single `BEGIN IMMEDIATE … COMMIT` transaction (with `rollback` on error), exactly like
the interactive `create_note`/`update_note` paths, so a crash cannot leave the `note_links`
projection stale relative to `notes`.

`DbBackend`'s state-based `resolve` and `FsBackend`'s log `merge` share the **same** version
vectors and tiebreak, so both backends now converge deterministically on the same winner — no
more per-backend divergence. `FsBackend` keeps per-device logs (Syncthing needs single-writer
files); `DbBackend` keeps the current row plus its `vv`. See `SECURITY.md`.

## Design notes

- The backend shares a single `libsql::Connection` across all gRPC tasks, guarded by a
  `lock: Arc<RwLock<()>>`. Mutating methods (and `apply_change`, `update_sync_time`,
  `prune_change_journal`) take the **write** side for the duration of their transaction;
  read methods take the **read** side. This prevents three failure modes on the shared
  connection: overlapping `BEGIN IMMEDIATE`s (which fail with "cannot start a transaction
  within a transaction"), a bare write landing inside another task's open transaction,
  and a query observing another task's uncommitted rows mid-transaction. SQLite allows
  only one writer at a time, so the exclusive write side is free; readers still run
  concurrently. The version-vector resolution in `apply_change` (the `*_incoming_wins` helpers
  over `note_log::resolve`) runs under the caller's write guard and therefore does not take its
  own.
- The `ws` field is wrapped in `Arc<Mutex<Option<WsStream>>>` so the backend can be
  shared across gRPC handler tasks (via `Arc<B>`) while still allowing exclusive write
  access to the WebSocket.
- `libsql` with `feature = "core"` uses an embedded SQLite library (no system libsql
  required). This keeps the binary self-contained.
- Resources use **soft delete** in `DbBackend`, like every other entity: `delete_resource`
  and the `ResourceDelete` arm of `apply_change` stamp `deleted_at` plus a bumped `vv`/`last_writer`
  (resolved through `note_log::resolve`) rather than running a physical `DELETE`, so a concurrent
  delete-vs-recreate converges. `list_resources` filters `deleted_at IS NULL` and `read_resource`
  returns `NotFound` for a tombstoned row. The BLOB in the `data` column is **retained** after a
  soft delete (the tombstone must persist for convergence); reclaiming that space is left to
  out-of-band maintenance.

## Related files

- `keeplin-core/src/storage/backend.rs` — trait that `DbBackend` implements
- `keeplin-core/src/models.rs` — all types stored by this backend
- `keeplin-daemon/src/main.rs` — constructs `DbBackend` in server mode
- `SECURITY.md` — WebSocket auth token security considerations
