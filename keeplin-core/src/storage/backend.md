# `storage/backend.rs` — the `StorageBackend` supertrait

## Purpose

Defines the single contract every storage layer fulfils. The rest of the codebase (daemon,
sync engine, decorators) is written against `Arc<dyn StorageBackend>` and never names a
concrete backend, so `FsBackend`, `DbBackend`, and the decorators (`EncryptedBackend`,
`LinkingBackend`, `EventBackend`) are freely interchangeable.

## Structure — one supertrait over five sub-traits

Rather than one giant trait, the contract is split by domain and re-composed:

| Sub-trait | Covers |
|-----------|--------|
| `NoteRepository` | note CRUD + `list_notes` + `note_backlinks` |
| `NotebookRepository` | notebook CRUD |
| `TagRepository` | tag CRUD + note↔tag associations |
| `ResourceRepository` | resource metadata + binary payload |
| `SyncBackend` | device id, sync timestamps, change journal, push/pull, prune |

```rust
pub trait StorageBackend:
    NoteRepository + NotebookRepository + TagRepository + ResourceRepository + SyncBackend {}

impl<T: ?Sized> StorageBackend for T where T: /* all five */ {}
```

A **blanket impl** means any type implementing all five sub-traits automatically *is* a
`StorageBackend` — implementors never write `impl StorageBackend`. Splitting the trait keeps
each backend file focused (its five `impl` blocks read one domain at a time) while callers
still get every method on one object.

All methods are `async` (via `async-trait`) and return `Result<T, StorageError>`.

## Notes (`NoteRepository`)

| Method | Description |
|--------|-------------|
| `create_note(note) -> Note` | Persist a new note; returns the stored copy |
| `read_note(id) -> Note` | Fetch by id; `NotFound` if absent or soft-deleted |
| `update_note(note) -> Note` | Overwrite an existing note |
| `delete_note(id) -> ()` | Soft-delete (set `deleted_at`) |
| `list_notes(page_size, page_token) -> (Vec<Note>, Option<String>)` | Cursor-paginated list of live notes (`page_size = 0` → default 100) |
| `note_backlinks(target_id, page_size, page_token) -> (Vec<Note>, Option<String>)` | Live notes that link **to** `target_id`, paginated |

`note_backlinks` has a **default implementation** on the trait: it collects notes page by
page and keeps those whose `links` resolve to `target_id`, paginating via the `paginate_notes`
helper. `FsBackend` inherits this scan; `DbBackend` overrides it with an indexed query against
its `note_links` projection table. Both share the same cursor shape.

## Notebooks, Tags

`NotebookRepository`: `create/read/update/delete_notebook`, paginated `list_notebooks`.
`TagRepository`: `create/read/update/delete_tag`, paginated `list_tags`, plus the note↔tag
association methods:

| Method | Description |
|--------|-------------|
| `add_note_tag(note_tag) -> ()` | Attach a tag to a note (idempotent) |
| `remove_note_tag(note_id, tag_id) -> ()` | Detach a tag |
| `list_note_tags(note_id, page_size, page_token) -> (Vec<Tag>, Option<String>)` | Tags on a note, paginated |

## Resources (`ResourceRepository`)

| Method | Description |
|--------|-------------|
| `create_resource(resource, data) -> Resource` | Store metadata + binary payload |
| `read_resource(id) -> (Resource, Vec<u8>)` | Retrieve metadata and bytes together |
| `delete_resource(id) -> ()` | Soft-delete a resource (versioned tombstone; blob retained on both backends) |
| `list_resources(page_size, page_token) -> (Vec<Resource>, Option<String>)` | Metadata only, paginated |

## Synchronisation (`SyncBackend`)

| Method | Description |
|--------|-------------|
| `get_device_id() -> String` | Stable identifier for this installation |
| `get_last_sync_time()` / `update_sync_time(ts)` | Read / persist the last-sync timestamp |
| `get_changes_since(since) -> Vec<Change>` | Local changes recorded after `since` |
| `apply_change(change) -> ()` | Apply one incoming change locally (**idempotent**) |
| `send_changes(changes)` / `receive_changes() -> Vec<Change>` | Push / pull with the peer |
| `prune_change_journal(older_than) -> u64` | Drop journal entries older than a watermark (no-op on FS) |

## Design notes

- `Send + Sync + 'static` bounds let the object live in an `Arc`, cross `tokio::spawn`, and
  sit in the tonic server struct.
- `async-trait` boxes each future (one small heap alloc per call) so the trait stays
  object-safe — negligible next to the I/O each method performs.
- `apply_change` **must be idempotent**: a change arriving twice yields the same state as once.
  Backends satisfy this with version-vector resolution (`note_log::resolve` for current-state
  rows/sidecars, `merge` for FS per-device note logs) — re-applying a change the store already
  dominates is a no-op — plus `INSERT OR IGNORE/REPLACE` for the underlying writes.

## Related files

- `keeplin-core/src/storage/fs.rs` — filesystem implementation (inherits the default backlinks scan).
- `keeplin-core/src/storage/db.rs` — LibSQL implementation (overrides backlinks with an index).
- `keeplin-core/src/encryption.rs`, `linking.rs`, `event_backend.rs` — decorators that wrap any backend.
- `keeplin-core/src/sync/engine.rs` — drives the `SyncBackend` methods generically.
