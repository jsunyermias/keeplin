# `storage/backend.rs` — StorageBackend trait

## Purpose

This module defines the `StorageBackend` trait — the single contract that every storage
implementation (`FsBackend`, `DbBackend`, and `EncryptedBackend`) must fulfil. By
programming against this trait instead of a concrete type, the rest of the codebase
(daemon, sync engine, CLI) remains independent of which storage mechanism is in use.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `StorageBackend` | trait | Async interface for all CRUD and synchronisation operations |

## Public API

The trait is grouped into five logical sections. All methods are `async` (via
`async-trait`) and return `Result<T, StorageError>`.

### Notes

| Method | Description |
|--------|-------------|
| `create_note(note: Note) -> Result<Note, StorageError>` | Persist a new note; returns the stored copy (may differ if a backend sets extra fields) |
| `read_note(id: Uuid) -> Result<Note, StorageError>` | Fetch a note by ID; returns `NotFound` if absent or soft-deleted |
| `update_note(note: Note) -> Result<Note, StorageError>` | Overwrite an existing note; returns `NotFound` if not present |
| `delete_note(id: Uuid) -> Result<(), StorageError>` | Soft-delete a note by setting `deleted_at` to now |
| `list_notes() -> Result<Vec<Note>, StorageError>` | Return all notes that have not been soft-deleted |

### Notebooks

Same CRUD pattern as Notes: `create_notebook`, `read_notebook`, `update_notebook`,
`delete_notebook`, `list_notebooks`.

### Tags

Same CRUD pattern as Notes: `create_tag`, `read_tag`, `update_tag`, `delete_tag`,
`list_tags`.

### Note–Tag relations

| Method | Description |
|--------|-------------|
| `add_note_tag(note_tag: NoteTag) -> Result<(), StorageError>` | Attach a tag to a note (idempotent — duplicates are ignored) |
| `remove_note_tag(note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError>` | Detach a tag from a note |
| `list_note_tags(note_id: Uuid) -> Result<Vec<Tag>, StorageError>` | Return all tags currently attached to the given note |

### Resources

| Method | Description |
|--------|-------------|
| `create_resource(resource: Resource, data: Vec<u8>) -> Result<Resource, StorageError>` | Store resource metadata alongside its binary payload |
| `read_resource(id: Uuid) -> Result<(Resource, Vec<u8>), StorageError>` | Retrieve metadata and binary data together |
| `delete_resource(id: Uuid) -> Result<(), StorageError>` | Permanently remove a resource (hard delete) |
| `list_resources() -> Result<Vec<Resource>, StorageError>` | List all resource metadata (without binary data) |

### Synchronisation

| Method | Description |
|--------|-------------|
| `get_changes_since(since: DateTime<Utc>) -> Result<Vec<Change>, StorageError>` | Return all `Change` events recorded after `since` |
| `apply_change(change: Change) -> Result<(), StorageError>` | Apply one incoming change to the local store (idempotent) |
| `get_last_sync_time() -> Result<DateTime<Utc>, StorageError>` | Read the persisted last-sync timestamp; returns epoch start if never synced |
| `update_sync_time(ts: DateTime<Utc>) -> Result<(), StorageError>` | Overwrite the last-sync timestamp after a successful sync cycle |
| `send_changes(changes: Vec<Change>) -> Result<(), StorageError>` | Push local changes to the remote peer |
| `receive_changes() -> Result<Vec<Change>, StorageError>` | Pull incoming changes from the remote peer |
| `get_device_id() -> Result<String, StorageError>` | Return the stable identifier for this installation |

## Design notes

- The `Send + Sync + 'static` bounds on the trait ensure it can be used inside `Arc<>`,
  passed across `tokio::spawn` boundaries, and held in a `tonic` server struct.
- `async-trait` rewrites each `async fn` into a method returning `Pin<Box<dyn Future>>`
  so that trait objects remain possible. This incurs one small heap allocation per call,
  which is acceptable because these methods already perform I/O.
- `apply_change` must be idempotent: if the same change arrives twice (e.g. after a
  retry), the result must be the same as if it arrived once. All current implementations
  use `INSERT OR IGNORE`/`INSERT OR REPLACE` or check-then-write patterns to satisfy
  this requirement.

## Related files

- `keeplin-core/src/storage/fs.rs` — filesystem implementation
- `keeplin-core/src/storage/db.rs` — LibSQL + WebSocket implementation
- `keeplin-core/src/encryption.rs` — decorator that wraps any `StorageBackend`
- `keeplin-core/src/sync/engine.rs` — uses this trait generically
