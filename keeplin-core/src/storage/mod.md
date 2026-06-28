# `storage/mod.rs` — storage module root

## Purpose

This file is the root of the `storage` sub-module. It declares the three child modules
that together provide the complete storage layer and re-exports `StorageBackend` at the
`storage` level so that callers can write `use keeplin_core::storage::StorageBackend`
instead of `use keeplin_core::storage::backend::StorageBackend`.

## Module map

| Module | Visibility | Description |
|--------|------------|-------------|
| `backend` | private (re-exported) | `StorageBackend` trait definition |
| `db` | public | `DbBackend` — LibSQL local cache + WebSocket sync |
| `fs` | public | `FsBackend` — JSON files on disk, log-based change tracking |

## Re-exports

```rust
pub use backend::StorageBackend;
```

## Design notes

- `backend` is declared `mod backend` (not `pub mod`) because its only public surface is
  `StorageBackend`, which is re-exported at this level. This prevents external code from
  accidentally importing private helpers inside `backend.rs`.
- `db` and `fs` are `pub mod` so that users can refer to their concrete types as
  `keeplin_core::storage::db::DbBackend` and `keeplin_core::storage::fs::FsBackend`.

## Related files

- `keeplin-core/src/storage/backend.rs` — trait definition
- `keeplin-core/src/storage/fs.rs` — filesystem backend
- `keeplin-core/src/storage/db.rs` — database backend
