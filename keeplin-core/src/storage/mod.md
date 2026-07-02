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

## `SortableRfc3339` — fixed-precision timestamps for text comparison

The backends store timestamps as RFC 3339 TEXT and order them lexicographically (SQLite
`WHERE created_at > ?` / `ORDER BY`, and the `"<ts>|<id>"` keyset cursors). Plain
`DateTime::to_rfc3339()` emits a *variable* number of fractional digits (3/6/9, whatever
the instant needs — platform clock precision leaks into the format), so equal instants can
be unequal strings and the cursor's `created_at = ?` equality branch silently fails across
precisions. The crate-private `SortableRfc3339::to_sortable_rfc3339` extension pins the
shape — always nine fractional digits, `+00:00` offset — and is what `db.rs`, `fs.rs`, and
`backend.rs` use for every stored/compared timestamp. Rows written before this existed keep
their variable-precision text; ordering against them remains chronologically consistent
(proven by the `lexicographic_order_matches_chronological_even_mixed_with_old_format` test).

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
