# `lib.rs` — keeplin-core crate root

## Purpose

This file is the crate root for `keeplin-core`, the library that all other Keeplin crates
depend on. It declares the seven public sub-modules that together form the complete Keeplin
storage, linking, and synchronisation layer. It contains no logic of its own; its sole role
is to make the sub-modules accessible to dependents.

## Module map

| Module | Public | Description |
|--------|--------|-------------|
| `encryption` | yes | AES-256-GCM transparent encryption decorator for any `StorageBackend` |
| `error` | yes | All error types used across the crate (`StorageError`, `SyncError`) |
| `links` | yes | Pure bookmark/link types and the `#…` reference grammar (I/O-free) |
| `linking` | yes | `LinkingBackend` decorator + reference-resolution / alias helpers |
| `models` | yes | Domain data types (`Note`, `Notebook`, `Tag`, `Resource`, `Change`, …) |
| `storage` | yes | `StorageBackend` supertrait plus `FsBackend` and `DbBackend` implementations |
| `sync` | yes | `SyncEngine` — orchestrates a full push/pull sync cycle |

## Dependency graph (intra-crate)

```
lib
 ├── error          (no intra-crate deps)
 ├── links          (uses models — pure types + grammar, no I/O)
 ├── models         (uses error, links)
 ├── storage
 │    ├── backend   (uses error, models)
 │    ├── note_log  (pure version-vector merge for FS notes)
 │    ├── fs        (uses error, models, storage::{backend, note_log})
 │    └── db        (uses error, models, storage::backend)
 ├── encryption     (uses error, models, storage::backend)
 ├── linking        (uses error, models, links, storage::backend)
 └── sync
      └── engine    (uses error, models, storage::backend)
```

## Design notes

- The crate deliberately avoids re-exporting types at the crate root so that callers
  must use fully-qualified paths (e.g. `keeplin_core::models::Note`). This makes import
  origins obvious at a glance.
- Adding a new backend requires only implementing `StorageBackend` in a new sub-module;
  no changes are needed here.

## Related files

- `keeplin-core/src/storage/backend.rs` — defines the `StorageBackend` trait that every
  storage implementation must satisfy
- `keeplin-daemon/src/main.rs` — the binary that consumes this crate
