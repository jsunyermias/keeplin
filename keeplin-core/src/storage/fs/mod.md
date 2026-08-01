# `storage/fs/mod.rs` — filesystem backend root type and module topology

Self-contained companion for `keeplin-core/src/storage/fs/mod.rs`. It documents every source block in source order with complete code embedded for every leaf.

**How to navigate**: each source marker `// md:<Header> > …` maps to the matching header chain below.

---

## Overview

**Identification** — file-level module declarations and imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
mod convert;
mod history;
mod io;
mod journal;
mod lifecycle;
mod notebooks;
mod notes;
mod pagination;
mod resources;
mod sidecars;
mod sync;
mod tags;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use notes::NoteMetaIndex;
```

**What it does** — Owns filesystem backend root type and module topology. This is a structural relocation from the former monolithic filesystem module; storage behavior, on-disk format version 8, serialization shapes, conflict resolution, and public `storage::fs::FsBackend` API are unchanged.

**Dependencies** — every binding above is either a crate symbol used directly by the blocks below or a sibling item exposed as `pub(super)`; expects: those symbols to preserve the signatures and behavior the relocated bodies already relied on, with compile-time failure on drift.

**Used by** — sibling modules under `storage/fs/` and callers of `crate::storage::fs::FsBackend`.

**Repeated context** — `FsBackend::FORMAT_VERSION` remains 8. Rust makes `mod.rs` fields visible to descendant modules; sibling-defined helpers used across files carry only `pub(super)`.

---

## FsBackend

**Identification** — `pub struct FsBackend`; marker `// md:FsBackend`.

**Code** — complete and verbatim:

```rust
// md:FsBackend
pub struct FsBackend {
    root: PathBuf,
    device_id: String,
    note_write_lock: Arc<Mutex<()>>,
    global_log_lock: Arc<Mutex<()>>,
    note_index: Arc<RwLock<Option<NoteMetaIndex>>>,
}
```

**What it does** — The backend's state and the on-disk tree:

```text
{root}/
  notes/{uuid}/note.md                    — materialized body
  notes/{uuid}/meta.ndjson               — metadata + merged vv (cache)
  notes/{uuid}/log.{device_id}.ndjson    — that device's op log (source of truth)
  notes/{uuid}/resources/{hash}.knrs     — attachment bytes, original format
  notes/{uuid}/resources/{id}.meta.ndjson — attachment metadata (StoredResource)
  notebooks/{uuid}.ndjson                — sidecar
  tags/{uuid}.ndjson                     — sidecar
  note_tags/{note}/{tag}                  — versioned association state
  logs/{device_id}.log                    — global NDJSON log (optional epoch header)
  .keeplin/device_id | format_version | sync_state.ndjson | offsets/{device_id}
```

Fields: `root`; `device_id` (from `.keeplin/device_id`);
`note_write_lock: Mutex<()>` — serialises this device's note-log mutations
(read-modify-write + atomic rename: without it two concurrent writes to the
same note read the same log and the second rename silently drops an entry; the
vv model assumes a single writer per device log). One global mutex keeps it
simple; reads need no lock (the atomic rename gives a consistent view);
`global_log_lock: Mutex<()>` — serialises append + compaction of the global
log; `note_index: RwLock<Option<NoteMetaIndex>>` — the lazy listing index.

**Used by** — everything below.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2; CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the same ignored `graphify-out/` layout locally.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `FsBackend` — defined or implemented in this focused filesystem module (INFERRED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/fs/mod.rs` and sibling `storage/fs/` modules — shared backend state and relocated helpers (INFERRED)
- `keeplin-core/src/storage/backend.rs`, `models.rs`, `error.rs`, and `storage/note_log.rs` as imported above — unchanged storage contracts (INFERRED)

**Direct dependents** (files whose symbols reference this one)

- sibling `storage/fs/` modules and existing `FsBackend` callers — unchanged public module path (INFERRED)

**Invariants** (the rules this file must keep true — restated here even if stated elsewhere)

- This split does not change the on-disk format; `FsBackend::FORMAT_VERSION` remains 8.
- The public backend path remains `crate::storage::fs::FsBackend`.
- Filesystem writes, journal replay, version-vector resolution, tombstones, and resource hashing preserve their pre-split behavior.

---

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|

| 1 | `Overview` | `// md:Overview` |
| 2 | `FsBackend` | `// md:FsBackend` |
