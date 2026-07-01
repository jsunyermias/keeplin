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
| `notes` | Note rows: soft-delete (`deleted_at`), plus `alias`, `bookmarks` (JSON), `links` (JSON) |
| `notebooks` | Notebook rows with soft-delete column and `alias` |
| `tags` | Tag rows with soft-delete column |
| `note_tags` | Many-to-many association; composite primary key `(note_id, tag_id)` |
| `note_links` | Projection of each note's resolved outgoing links (see below) — powers indexed backlinks |
| `resources` | Resource metadata + BLOB (`data` column) |
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
`server_url`. If `server_url` is empty or the connection fails, the backend starts in
offline mode (all local CRUD operations still work; sync operations are no-ops or return
empty results).  
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
dropped. If all retries fail, `StorageError::WebSocket` is returned.

### Receiving changes (`receive_changes`)
The client reads all available WebSocket messages in a non-blocking loop, stopping
after 100 ms of silence (`drain_timeout = 100 ms`). Each text message is deserialised
as a `Vec<Change>` and appended to the result. Binary messages and errors are ignored
(logged as warnings).

## `apply_change` — all 13 variants

| Variant | SQL |
|---------|-----|
| `NoteCreate` | `INSERT OR REPLACE INTO notes …` (shares the arm with `NoteUpdate`) |
| `NoteUpdate` | `INSERT OR REPLACE INTO notes …` |
| `NoteDelete` | `UPDATE notes SET deleted_at = ? WHERE id = ?` |
| `NotebookCreate` | `INSERT OR REPLACE INTO notebooks …` |
| `NotebookUpdate` | `INSERT OR REPLACE INTO notebooks …` |
| `NotebookDelete` | `UPDATE notebooks SET deleted_at = ? WHERE id = ?` |
| `TagCreate` | `INSERT OR REPLACE INTO tags …` |
| `TagUpdate` | `INSERT OR REPLACE INTO tags …` |
| `TagDelete` | `UPDATE tags SET deleted_at = ? WHERE id = ?` |
| `NoteTagAdd` | `INSERT OR IGNORE INTO note_tags …` |
| `NoteTagRemove` | `DELETE FROM note_tags WHERE note_id=? AND tag_id=?` |
| `ResourceCreate` | `INSERT OR IGNORE INTO resources (…, data) VALUES (…, ?)` with `data = payload.unwrap_or_default()` |
| `ResourceDelete` | `DELETE FROM resources WHERE id = ?` (resources use hard delete) |

All operations are idempotent by design. The create/update arms for notes, notebooks,
and tags are additionally guarded by `should_apply`, which reads the stored `updated_at`
and **skips** the write when the incoming change is not strictly newer — implementing
last-write-wins so a stale remote edit cannot clobber a newer local record.

The note create/update arm wraps `should_apply` + `INSERT OR REPLACE` + `refresh_note_links`
in a single `BEGIN IMMEDIATE … COMMIT` transaction (with `rollback` on error), exactly like
the interactive `create_note`/`update_note` paths. Without this, a crash between the row write
and the projection refresh could leave the `note_links` table stale relative to `notes`, and
backlinks would silently drift. The transaction keeps the row and its projection atomic while
staying idempotent.

This is last-write-wins for **every** entity, notes included. `DbBackend` does **not**
implement the per-note version-vector merge that `FsBackend` uses (see
`storage/note_log.md`): two devices editing the same note offline and then syncing keep
only the later-`updated_at` edit, with no merge. The difference between the backends is
documented in `SECURITY.md` ("Conflict resolution differs by backend").

## Design notes

- The backend shares a single `libsql::Connection` across all gRPC tasks, guarded by a
  `lock: Arc<RwLock<()>>`. Mutating methods (and `apply_change`, `update_sync_time`,
  `prune_change_journal`) take the **write** side for the duration of their transaction;
  read methods take the **read** side. This prevents three failure modes on the shared
  connection: overlapping `BEGIN IMMEDIATE`s (which fail with "cannot start a transaction
  within a transaction"), a bare write landing inside another task's open transaction,
  and a query observing another task's uncommitted rows mid-transaction. SQLite allows
  only one writer at a time, so the exclusive write side is free; readers still run
  concurrently. `should_apply` runs under the caller's write guard and therefore does not
  take its own.
- The `ws` field is wrapped in `Arc<Mutex<Option<WsStream>>>` so the backend can be
  shared across gRPC handler tasks (via `Arc<B>`) while still allowing exclusive write
  access to the WebSocket.
- `libsql` with `feature = "core"` uses an embedded SQLite library (no system libsql
  required). This keeps the binary self-contained.
- Resources use **hard delete** in `DbBackend`: the `resources` table has no `deleted_at`
  column, and both `delete_resource` and the `ResourceDelete` arm of `apply_change` run a
  physical `DELETE FROM resources`. This matches `FsBackend` (which removes the resource
  directory) and the documented model-wide choice to hard-delete resources because their BLOB
  payloads can be large (see `models.md`). Notes, notebooks, and tags, by contrast, use
  soft-delete (a `deleted_at` tombstone) so a delete can win last-write-wins against a
  concurrent edit. The `ResourceDelete` change still propagates normally through the journal;
  it just isn't retained as a tombstone row afterwards.

## Related files

- `keeplin-core/src/storage/backend.rs` — trait that `DbBackend` implements
- `keeplin-core/src/models.rs` — all types stored by this backend
- `keeplin-daemon/src/main.rs` — constructs `DbBackend` in server mode
- `SECURITY.md` — WebSocket auth token security considerations
