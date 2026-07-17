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
    ResourceRepository, SyncBackend, NotebookSortProfile,
};
```

`NotebookSortProfile` is the compact per-notebook ordering summary (`pinned_keys`, `min_key`,
`max_normal_key`) the `ordering` placement rules read; each backend builds it natively.

## Page-size clamping — `effective_page_size`

Every list method sizes its page through `effective_page_size(page_size)`: `0` means the
`DEFAULT_PAGE_SIZE` (100), and any value above `MAX_PAGE_SIZE` (1000) is clamped down to it.
`page_size` arrives from the network as an arbitrary `u32`, so the cap stops a single request
for `u32::MAX` rows from making a backend materialize the whole store in one response (a
memory-exhaustion DoS); the reply's cursor lets a well-behaved client keep paging.

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

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `effective_page_size()` — defined here (EXTRACTED; file-local)
- `SortableRfc3339` — defined here (EXTRACTED; file-local)
- `chrono::DateTime<chrono::Utc>` — defined here (EXTRACTED; file-local)
- `.to_sortable_rfc3339()` — defined here (EXTRACTED; file-local)
- `effective_page_size_defaults_and_clamps()` — defined here (EXTRACTED; file-local)
- `sortable_rfc3339_has_fixed_shape()` — defined here (EXTRACTED; file-local)
- `lexicographic_order_matches_chronological_even_mixed_with_old_format()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Declares child modules and re-exports the `StorageBackend` supertrait + sub-traits at `storage` level; no logic here.
- New storage backends/decorators are added as child modules and re-exported here.

## Related files

- `keeplin-core/src/storage/backend.rs` — supertrait + sub-trait definitions
- `keeplin-core/src/storage/note_log.rs` — pure merge logic
- `keeplin-core/src/storage/fs.rs` — filesystem backend
- `keeplin-core/src/storage/db.rs` — database backend
