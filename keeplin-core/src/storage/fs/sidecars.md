# `storage/fs/sidecars.rs` — version-vector resolution for notebook and tag sidecars

Self-contained companion for `keeplin-core/src/storage/fs/sidecars.rs`. It documents every source block in source order with complete code embedded for every leaf.

**How to navigate**: each source marker `// md:<Header> > …` maps to the matching header chain below.

---

## Overview

**Identification** — file-level module declarations and imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::path::Path;

use chrono::{DateTime, Utc};

use crate::error::StorageError;
use crate::storage::note_log::{self, resolve, VersionVector, Winner};

use super::FsBackend;
```

**What it does** — Owns version-vector resolution for notebook and tag sidecars. This is a structural relocation from the former monolithic filesystem module; storage behavior, on-disk format version 8, serialization shapes, conflict resolution, and public `storage::fs::FsBackend` API are unchanged.

**Dependencies** — every binding above is either a crate symbol used directly by the blocks below or a sibling item exposed as `pub(super)`; expects: those symbols to preserve the signatures and behavior the relocated bodies already relied on, with compile-time failure on drift.

**Used by** — sibling modules under `storage/fs/` and callers of `crate::storage::fs::FsBackend`.

**Repeated context** — `FsBackend::FORMAT_VERSION` remains 8. Rust makes `mod.rs` fields visible to descendant modules; sibling-defined helpers used across files carry only `pub(super)`.

---

## impl FsBackend

**Identification** — the first inherent impl; marker `// md:impl FsBackend`.
Constructor, sweeps, format versioning, path helpers, log machinery,
sidecar/association/resource versioning, the note merge pipeline. Contains the
constants `FORMAT_VERSION = 8`, `NOTE_LOG_COMPACT_THRESHOLD = 256`,
`GLOBAL_LOG_COMPACT_THRESHOLD = 512`, `GLOBAL_LOG_SOFT_BYTES = 64 KiB`
(documented with the methods that use them).

**Code** — container: members documented as sub-blocks below: fn sidecar_vv, fn next_sidecar_vv, fn sidecar_incoming_wins.

---

### fn sidecar_vv

**Identification** — marker `// md:impl FsBackend > fn sidecar_vv`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn sidecar_vv
    pub(super) async fn sidecar_vv(&self, path: &Path) -> Result<VersionVector, StorageError> {
        #[derive(serde::Deserialize)]
        struct VvProbe {
            #[serde(default)]
            vv: VersionVector,
        }
        if !path.exists() {
            return Ok(VersionVector::new());
        }
        let bytes = tokio::fs::read(path).await?;
        let probe: VvProbe = serde_json::from_slice(&bytes)
            .map_err(|e| StorageError::CorruptedData(format!("ndjson decode: {e}")))?;
        Ok(probe.vv)
    }
```

**What it does** — Deserialises only the `vv` field of a notebook/tag sidecar
(empty when the file is absent), to base a local write's incremented vector on
current state.

---

### fn next_sidecar_vv

**Identification** — marker `// md:impl FsBackend > fn next_sidecar_vv`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn next_sidecar_vv
    pub(super) async fn next_sidecar_vv(&self, path: &Path) -> Result<VersionVector, StorageError> {
        let mut vv = self.sidecar_vv(path).await?;
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }
```

**What it does** — `sidecar_vv` + increment this device's component.

---

### fn sidecar_incoming_wins

**Identification** — marker `// md:impl FsBackend > fn sidecar_incoming_wins`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn sidecar_incoming_wins
    pub(super) async fn sidecar_incoming_wins(
        &self,
        path: &Path,
        incoming_vv: &VersionVector,
        incoming_updated: DateTime<Utc>,
        incoming_writer: &str,
    ) -> Result<bool, StorageError> {
        #[derive(serde::Deserialize)]
        struct MetaProbe {
            updated_at: DateTime<Utc>,
            #[serde(default)]
            vv: VersionVector,
            #[serde(default)]
            last_writer: String,
        }
        if !path.exists() {
            return Ok(true);
        }
        let bytes = tokio::fs::read(path).await?;
        let m: MetaProbe = serde_json::from_slice(&bytes)
            .map_err(|e| StorageError::CorruptedData(format!("ndjson decode: {e}")))?;
        Ok(matches!(
            resolve(
                &m.vv,
                m.updated_at,
                &m.last_writer,
                incoming_vv,
                incoming_updated,
                incoming_writer,
            ),
            Winner::Incoming
        ))
    }
```

**What it does** — `resolve` over the stored vs incoming
`(vv, updated_at, last_writer)` of a notebook/tag sidecar; `true` when no
local sidecar exists.

**Used by** — `apply_change` (notebooks/tags).

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2; CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the same ignored `graphify-out/` layout locally.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `version-vector resolution for notebook and tag sidecars` — defined or implemented in this focused filesystem module (INFERRED)

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
| 2 | `impl FsBackend` (container) | `// md:impl FsBackend` |
| 3 | `fn sidecar_vv` | `// md:impl FsBackend > fn sidecar_vv` |
| 4 | `fn next_sidecar_vv` | `// md:impl FsBackend > fn next_sidecar_vv` |
| 5 | `fn sidecar_incoming_wins` | `// md:impl FsBackend > fn sidecar_incoming_wins` |
