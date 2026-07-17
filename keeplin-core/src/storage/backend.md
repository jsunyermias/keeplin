# `storage/backend.rs` — the `StorageBackend` supertrait

## Purpose

Defines the single contract every storage layer fulfils. The rest of the codebase (daemon,
sync engine, decorators) is written against `Arc<dyn StorageBackend>` and never names a
concrete backend, so `FsBackend`, `DbBackend`, and the decorators (`EncryptedBackend`,
`LinkingBackend`, `EventBackend`) are freely interchangeable.

## Structure — one supertrait over six sub-traits

Rather than one giant trait, the contract is split by domain and re-composed:

| Sub-trait | Covers |
|-----------|--------|
| `NoteRepository` | note CRUD + `list_notes` + `note_backlinks` |
| `NotebookRepository` | notebook CRUD |
| `TagRepository` | tag CRUD + note↔tag associations |
| `ResourceRepository` | resource metadata + binary payload |
| `SyncBackend` | device id, sync timestamps, change journal, push/pull, prune |
| `HistoryRepository` | past versions of a note/notebook, derived from the change journal |

```rust
pub trait StorageBackend:
    NoteRepository + NotebookRepository + TagRepository + ResourceRepository
    + SyncBackend + HistoryRepository {}

impl<T: ?Sized> StorageBackend for T where T: /* all six */ {}
```

A **blanket impl** means any type implementing all six sub-traits automatically *is* a
`StorageBackend` — implementors never write `impl StorageBackend`. Splitting the trait keeps
each backend file focused (its `impl` blocks read one domain at a time) while callers
still get every method on one object.

### `HistoryRepository`

| Method | Description |
|--------|-------------|
| `note_history(id, limit) -> Vec<EntityVersion<Note>>` | past note versions, newest first (`limit = 0` → `DEFAULT_HISTORY_LIMIT` = 100) |
| `notebook_history(id, limit) -> Vec<EntityVersion<Notebook>>` | past notebook versions, newest first |

`EntityVersion<T>` is `{ timestamp, device_id, entity: Option<T> }` — `entity` is `None` for a
tombstone version. Unlike `SyncBackend::get_changes_since` (which passes ciphertext through for
the relay), history is a **read** surface, so `EncryptedBackend` decrypts each version. The
roll-back operations built on this live in `keeplin-core/src/history.rs`.

All methods are `async` (via `async-trait`) and return `Result<T, StorageError>`.

## Notes (`NoteRepository`)

| Method | Description |
|--------|-------------|
| `create_note(note) -> Note` | Persist a new note; returns the stored copy |
| `read_note(id) -> Note` | Fetch by id; `NotFound` if absent or soft-deleted |
| `update_note(note) -> Note` | Overwrite an existing note |
| `delete_note(id) -> ()` | Soft-delete (set `deleted_at`) |
| `list_notes(page_size, page_token) -> (Vec<Note>, Option<String>)` | Cursor-paginated list of live notes, `(created_at, id)` order (`page_size = 0` → default 100, clamped to `MAX_PAGE_SIZE`) |
| `list_notes_in_notebook(notebook_id, page_size, page_token)` | One notebook's live notes in **manual order** (`effective sort_key`, then id); the nil UUID is the Inbox |
| `list_starred_notes(page_size, page_token)` | Every live **starred** note, across all notebooks, `(created_at, id)` order |
| `notebook_sort_profile(notebook_id) -> NotebookSortProfile` | Compact `{pinned_keys, min_key, max_normal_key}` summary the `ordering` placement rules read instead of materialising the notebook |
| `note_backlinks(target_id, page_size, page_token) -> (Vec<Note>, Option<String>)` | Live notes that link **to** `target_id`, paginated |

`note_backlinks` has a **default implementation** on the trait: it collects notes page by
page and keeps those whose `links` resolve to `target_id`, paginating via the `paginate_notes`
helper. `FsBackend` inherits this scan; `DbBackend` overrides it with an indexed query against
its `note_links` projection table. Both share the same cursor shape.

The listing/ordering methods (`list_notes_in_notebook`, `list_starred_notes`,
`notebook_sort_profile`) back the pinning/ordering/starring feature; each backend implements
them natively (`DbBackend` with the `(notebook_id, sort_key, id)` index, `FsBackend` from an
in-memory metadata index). The placement rules that *decide* `sort_key`/`is_pinned` live in
`keeplin-core/src/ordering.rs` (`pin_note`, `unpin_note`, `reorder_note`, `place_new_note`,
`reconcile_notebook_move`, etc.), not here.

`NotebookSortProfile` (defined in this file) is a plaintext summary of one notebook's live
sort keys — `pinned_keys` (the used `1..=999` slots), `min_key`, and `max_normal_key` — built
by `from_effective_keys`. `ordering` reads it to pick the next pin slot or the end of the
normal band without loading any note bodies.

## Notebooks, Tags

`NotebookRepository`: `create/read/update/delete_notebook`, paginated `list_notebooks`.
`TagRepository`: `create/read/update/delete_tag`, paginated `list_tags`, plus the note↔tag
association methods:

| Method | Description |
|--------|-------------|
| `add_note_tag(note_tag) -> ()` | Attach a tag to a note (idempotent); `NotFound` when the note or tag is missing or soft-deleted — no dangling associations via the API (`apply_change` skips this check: sync delivery order is unordered) |
| `remove_note_tag(note_id, tag_id) -> ()` | Detach a tag |
| `list_note_tags(note_id, page_size, page_token) -> (Vec<Tag>, Option<String>)` | Tags on a note, paginated |

## Resources (`ResourceRepository`)

| Method | Description |
|--------|-------------|
| `create_resource(resource, data) -> Resource` | Store metadata + binary payload |
| `read_resource(id) -> (Resource, Vec<u8>)` | Retrieve metadata and bytes together |
| `delete_resource(id) -> ()` | Soft-delete a resource (versioned tombstone; blob retained on both backends) |
| `list_resources(page_size, page_token) -> (Vec<Resource>, Option<String>)` | Metadata only, paginated |
| `purge_deleted_resources(older_than) -> u64` | Reclaim the **binary payloads** of resources tombstoned before `older_than`; the tombstone metadata is always kept, so convergence is unaffected. Returns how many payloads were freed |

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

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `StorageBackend` — defined here (EXTRACTED; 56 cross-file edge(s))
- `EntityVersion` — defined here (EXTRACTED; 21 cross-file edge(s))
- `T` — defined here (EXTRACTED; 20 cross-file edge(s))
- `NotebookSortProfile` — defined here (EXTRACTED; 8 cross-file edge(s))
- `NoteRepository` — defined here (EXTRACTED; 7 cross-file edge(s))
- `NotebookRepository` — defined here (EXTRACTED; 7 cross-file edge(s))
- `TagRepository` — defined here (EXTRACTED; 7 cross-file edge(s))
- `ResourceRepository` — defined here (EXTRACTED; 7 cross-file edge(s))
- `SyncBackend` — defined here (EXTRACTED; 7 cross-file edge(s))
- `HistoryRepository` — defined here (EXTRACTED; 7 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: references×1; e.g. `Note`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/collab/mod.rs` — client of the keeplin-srv collaborative channel (EXTRACTED: implements×6, references×5; e.g. `Shared`, `CollabBackend<B>`, `.start()`)
- `keeplin-core/src/encryption.rs` — transparent at-rest encryption (EXTRACTED: implements×6, references×3; e.g. `EncryptedBackend<B>`, `.notebook_sort_profile()`, `.note_history()`)
- `keeplin-core/src/history.rs` — change history reads + forward-revert (EXTRACTED: references×7; e.g. `state_at()`, `revert_note()`, `revert_notebook()`)
- `keeplin-core/src/interop.rs` — vCard & iCalendar format compatibility (EXTRACTED: imports_from×1, references×15; e.g. `interop.rs`, `resources_with_mime()`, `resource_metas_with_mime()`)
- `keeplin-core/src/linking.rs` — `LinkingBackend` decorator + reference resolution (EXTRACTED: implements×6, references×16; e.g. `AliasConflict`, `LinkingBackend<B>`, `collect_notes()`)
- `keeplin-core/src/migrate.rs` — one-shot state copy between backends (EXTRACTED: references×2; e.g. `migrate()`, `collect()`)
- `keeplin-core/src/ordering.rs` — the Inbox, pinning, manual ordering, and starring (EXTRACTED: references×12; e.g. `ensure_inbox()`, `place_new_note()`, `pin_note()`)
- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: implements×6, references×8; e.g. `DbBackend`, `.notebook_sort_profile()`, `.entity_history()`)
- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: implements×6, references×12; e.g. `FsBackend`, `.notebook_sort_profile()`, `.note_history()`)
- `keeplin-core/src/sync/engine.rs` — SyncEngine (EXTRACTED: references×2; e.g. `SyncEngine`, `.new()`)
- `keeplin-core/tests/migrate.rs` — cross-backend migration tests (EXTRACTED: references×2; e.g. `assert_migrated()`, `seed()`)
- `keeplin-daemon/src/event_backend.rs` — `EventBackend` change-publishing decorator (EXTRACTED: implements×6, references×3; e.g. `EventBackend<B>`, `.notebook_sort_profile()`, `.note_history()`)
- `keeplin-daemon/src/main.rs` — daemon entry point (EXTRACTED: references×1; e.g. `build_storage()`)
- `keeplin-daemon/src/metrics.rs` — operational metrics (EXTRACTED: implements×6, references×4; e.g. `MetricsBackend<B>`, `.notebook_sort_profile()`, `.note_history()`)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: references×4; e.g. `note_version_dto()`, `notebook_version_dto()`, `AppState`)
- `keeplin-daemon/src/search.rs` — daemon full-text search (EXTRACTED: imports_from×1, references×6; e.g. `apply_change()`, `denormalize()`, `index_note()`)
- `keeplin-daemon/src/server.rs` — gRPC service implementation (EXTRACTED: references×1; e.g. `ensure_not_deleted()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- The rest of the codebase is written against `Arc<dyn StorageBackend>` and never names a concrete backend — backends and decorators must stay freely interchangeable.
- Trait changes ripple through every backend AND every decorator; default methods are preferred for additive evolution.
- Repository sub-traits (notes, notebooks, tags, resources, sync, history) compose into the supertrait; implementors must implement all.

## Related files

- `keeplin-core/src/storage/fs.rs` — filesystem implementation (inherits the default backlinks scan).
- `keeplin-core/src/storage/db.rs` — LibSQL implementation (overrides backlinks with an index).
- `keeplin-core/src/encryption.rs`, `linking.rs`, `event_backend.rs` — decorators that wrap any backend.
- `keeplin-core/src/sync/engine.rs` — drives the `SyncBackend` methods generically.
