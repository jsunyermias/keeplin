# `sync/mod.rs` — sync module root

## Purpose

This file is the root of the `sync` sub-module. It declares the `engine` child module
and re-exports `SyncEngine` so that callers can write
`use keeplin_core::sync::SyncEngine` instead of
`use keeplin_core::sync::engine::SyncEngine`.

## Module map

| Module | Visibility | Description |
|--------|------------|-------------|
| `engine` | private (re-exported) | `SyncEngine` — orchestrates a full push/pull sync cycle |

## Re-exports

```rust
pub use engine::SyncEngine;
```

## Design notes

- The module is intentionally minimal. Future sync strategies (e.g. peer-to-peer, CRDTs)
  could be added as sibling modules here without changing the public interface.
- `engine` is declared as a private module (`mod engine`) because its only public surface
  is `SyncEngine`. Private helpers inside `engine.rs` are not accessible to external code.

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

- Module root: declares `engine` and re-exports `SyncEngine`; no logic.

## Related files

- `keeplin-core/src/sync/engine.rs` — full implementation of the sync cycle
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` trait that `SyncEngine`
  depends on
