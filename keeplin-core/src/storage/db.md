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
| `notes` | Note rows with soft-delete column (`deleted_at`) |
| `notebooks` | Notebook rows with soft-delete column |
| `tags` | Tag rows with soft-delete column |
| `note_tags` | Many-to-many association; composite primary key `(note_id, tag_id)` |
| `resources` | Resource metadata + BLOB (`data` column) |
| `sync_state` | Key-value store for the last-sync timestamp |
| `device` | Single-row table holding the stable device UUID |
| `entity_changes` | Append-only change journal (see below) |

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

Every mutating method calls `record_change(entity_type, entity_id, operation, data)` after
a successful database write. This inserts one row into `entity_changes` stamped with the
current UTC timestamp. `get_changes_since(since)` then reads rows from this table with
`changed_at > since`, converting each to a `Change` via `row_to_change`.

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

## Design notes

- Every mutating method (and `apply_change`, `update_sync_time`, `prune_change_journal`)
  takes a `write_lock: Arc<Mutex<()>>` for the duration of its transaction. The backend
  shares a single `libsql::Connection`, so without this lock a second `BEGIN IMMEDIATE`
  issued before the first `COMMIT` would fail with "cannot start a transaction within a
  transaction". SQLite allows only one writer at a time, so serialising writes is correct
  and free.
- The `ws` field is wrapped in `Arc<Mutex<Option<WsStream>>>` so the backend can be
  shared across gRPC handler tasks (via `Arc<B>`) while still allowing exclusive write
  access to the WebSocket.
- `libsql` with `feature = "core"` uses an embedded SQLite library (no system libsql
  required). This keeps the binary self-contained.
- Resources use soft-delete in `DbBackend` (setting `deleted_at`) rather than physical
  row deletion, because the `ResourceDelete` change must survive in the journal long
  enough to propagate to peers. However, the BLOB column is not cleared on soft-delete;
  manual pruning of old data requires a separate maintenance call.

## Related files

- `keeplin-core/src/storage/backend.rs` — trait that `DbBackend` implements
- `keeplin-core/src/models.rs` — all types stored by this backend
- `keeplin-daemon/src/main.rs` — constructs `DbBackend` in server mode
- `SECURITY.md` — WebSocket auth token security considerations
