# `lib.rs` — keeplin-core crate root

## Purpose

This file is the crate root for `keeplin-core`, the library that all other Keeplin crates
depend on. It declares the public sub-modules that together form the complete Keeplin
storage, linking, ordering, and synchronisation layer. It contains no logic of its own; its sole role
is to make the sub-modules accessible to dependents.

## Module map

| Module | Public | Description |
|--------|--------|-------------|
| `collab` | yes | `CollabBackend` decorator — client of keeplin-srv's real-time note channel (line ops, presence, cursors) |
| `compat` | yes | keeplin-srv `GET /version` handshake: `PROTOCOL_VERSION`, the `compatible_with` rule, `negotiate` — the one place this repo defines server-protocol compatibility |
| `encryption` | yes | AES-256-GCM transparent encryption decorator for any `StorageBackend` |
| `error` | yes | All error types used across the crate (`StorageError`, `SyncError`) |
| `links` | yes | Pure bookmark/link types and the `#…` reference grammar (I/O-free) |
| `linking` | yes | `LinkingBackend` decorator + reference-resolution / alias helpers |
| `models` | yes | Domain data types (`Note`, `Notebook`, `Tag`, `Resource`, `Change`, …) |
| `ordering` | yes | The Inbox system notebook, pinning, manual `sort_key` ordering, and starring |
| `storage` | yes | `StorageBackend` supertrait plus `FsBackend` and `DbBackend` implementations |
| `sync` | yes | `SyncEngine` — orchestrates a full push/pull sync cycle |

## Dependency graph (intra-crate)

```
lib
 ├── error          (no intra-crate deps)
 ├── links          (uses models — pure types + grammar, no I/O)
 ├── models         (uses error, links)
 ├── ordering       (uses error, models, storage::backend)
 ├── storage
 │    ├── backend   (uses error, models)
 │    ├── note_log  (pure version-vector merge for FS notes)
 │    ├── fs        (uses error, models, storage::{backend, note_log})
 │    └── db        (uses error, models, storage::backend)
 ├── encryption     (uses error, models, storage::backend)
 ├── linking        (uses error, models, links, storage::backend)
 ├── collab         (uses error, models, storage::backend, note_log::VersionVector)
 │    ├── protocol  (wire types mirroring keeplin-srv; I/O-free)
 │    └── state     (client line mirror + body↔lines diff; I/O-free)
 └── sync
      └── engine    (uses error, models, storage::backend)
```

## Design notes

- The crate deliberately avoids re-exporting types at the crate root so that callers
  must use fully-qualified paths (e.g. `keeplin_core::models::Note`). This makes import
  origins obvious at a glance.
- Adding a new backend requires only implementing `StorageBackend` in a new sub-module;
  no changes are needed here.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- (no symbols extracted for this file — it contributes only its file node) (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- `lib.rs` declares modules only — no logic; every concrete type lives in a sub-module.
- Each public module keeps a companion `.md`; adding a module means adding it to `lib.rs`, its doc, and the module map here.

## Related files

- `keeplin-core/src/storage/backend.rs` — defines the `StorageBackend` trait that every
  storage implementation must satisfy
- `keeplin-daemon/src/main.rs` — the binary that consumes this crate
