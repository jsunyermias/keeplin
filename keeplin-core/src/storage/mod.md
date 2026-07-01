# `storage/mod.rs` — storage module root

## Purpose

This file is the root of the `storage` sub-module. It declares the child modules that
together provide the complete storage layer and re-exports the `StorageBackend` supertrait
(and its five sub-traits) at the `storage` level so callers can write
`use keeplin_core::storage::StorageBackend` instead of the longer `…::backend::StorageBackend`.

## Module map

| Module | Visibility | Description |
|--------|------------|-------------|
| `backend` | private (re-exported) | `StorageBackend` supertrait + the five sub-traits |
| `note_log` | public | Pure version-vector merge for FS per-note logs (I/O-free, unit-tested) |
| `db` | public | `DbBackend` — LibSQL local cache + WebSocket sync |
| `fs` | public | `FsBackend` — files on disk (msgpack sidecars + per-note VV logs), Syncthing replication |

## Re-exports

```rust
pub use backend::{
    StorageBackend, NoteRepository, NotebookRepository, TagRepository,
    ResourceRepository, SyncBackend,
};
```

## Design notes

- `backend` is declared `mod backend` (not `pub mod`) because its public surface is just the
  trait family, re-exported here. This keeps `backend.rs`'s private helpers (e.g.
  `paginate_notes`) out of the public path.
- `db`, `fs`, and `note_log` are `pub mod` so their concrete types/functions are reachable as
  `keeplin_core::storage::{db::DbBackend, fs::FsBackend, note_log::merge}`.

## Related files

- `keeplin-core/src/storage/backend.rs` — supertrait + sub-trait definitions
- `keeplin-core/src/storage/note_log.rs` — pure merge logic
- `keeplin-core/src/storage/fs.rs` — filesystem backend
- `keeplin-core/src/storage/db.rs` — database backend
