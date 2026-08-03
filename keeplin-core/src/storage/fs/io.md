# `storage/fs/io.rs` — atomic writes and generic sidecar I/O

Self-contained companion for `keeplin-core/src/storage/fs/io.rs`. It documents every source block in source order with complete code embedded for every leaf.

**How to navigate**: each source marker `// md:<Header> > …` maps to the matching header chain below.

---

## Overview

**Identification** — file-level module declarations and imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::path::Path;

use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::error::StorageError;

use super::FsBackend;
```

**What it does** — Owns atomic writes and generic sidecar I/O. This is a structural relocation from the former monolithic filesystem module; storage behavior, on-disk format version 8, serialization shapes, conflict resolution, and public `storage::fs::FsBackend` API are unchanged.

**Dependencies** — every binding above is either a crate symbol used directly by the blocks below or a sibling item exposed as `pub(super)`; expects: those symbols to preserve the signatures and behavior the relocated bodies already relied on, with compile-time failure on drift.

**Used by** — sibling modules under `storage/fs/` and callers of `crate::storage::fs::FsBackend`.

**Repeated context** — `FsBackend::FORMAT_VERSION` remains 8. Rust makes `mod.rs` fields visible to descendant modules; sibling-defined helpers used across files carry only `pub(super)`.

---

## fn atomic_write

**Identification** — `async fn atomic_write(path: &Path, bytes: &[u8])`; marker
`// md:fn atomic_write`.

**Code** — complete and verbatim:

```rust
// md:fn atomic_write
pub(super) async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let tmp = path.with_extension("tmp");
    let result = async {
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&tmp, path).await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}
```

**What it does** — Write a sibling `{path}.tmp`, **fsync**, then rename over
the destination: a reader never observes a half-written file; a failed write
leaves the previous contents intact; the fsync closes the power-loss window in
which the rename persists but the data does not. On failure the temp file is
best-effort removed (a crash can still orphan one — see
`sweep_orphan_tmp_files`).

**Used by** — every sidecar/projection/cursor write.

---

## impl FsBackend

**Identification** — the first inherent impl; marker `// md:impl FsBackend`.
Constructor, sweeps, format versioning, path helpers, log machinery,
sidecar/association/resource versioning, the note merge pipeline. Contains the
constants `FORMAT_VERSION = 8`, `NOTE_LOG_COMPACT_THRESHOLD = 256`,
`GLOBAL_LOG_COMPACT_THRESHOLD = 512`, `GLOBAL_LOG_SOFT_BYTES = 64 KiB`
(documented with the methods that use them).

**Code** — container: members documented as sub-blocks below: fn write_sidecar, fn read_sidecar.

---

### fn write_sidecar

**Identification** — marker `// md:impl FsBackend > fn write_sidecar`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn write_sidecar
    pub(super) async fn write_sidecar<T: serde::Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), StorageError> {
        let mut bytes = serde_json::to_vec(value)
            .map_err(|e| StorageError::InvalidState(format!("ndjson encode: {e}")))?;
        bytes.push(b'\n');
        atomic_write(path, &bytes).await
    }
```

**What it does** — JSON-encode the whole value as a single NDJSON line
(one object, trailing `\n`) + `atomic_write` (encode failure → `InvalidState`).
Single-entity sidecars (note/notebook/tag/resource metadata, `sync_state`,
note↔tag association state) are one JSON object per file; the per-device note
log is the multi-line case handled by `write_note_log`.

---

### fn read_sidecar

**Identification** — marker `// md:impl FsBackend > fn read_sidecar`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_sidecar
    pub(super) async fn read_sidecar<T: serde::de::DeserializeOwned>(
        &self,
        path: &Path,
        id: Uuid,
    ) -> Result<T, StorageError> {
        if !path.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let bytes = tokio::fs::read(path).await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| StorageError::CorruptedData(format!("ndjson decode: {e}")))
    }
```

**What it does** — Read + JSON-decode a single-entity sidecar; missing file →
`NotFound(id)`, bad bytes → `CorruptedData`. `serde_json::from_slice` tolerates
the trailing `\n` written by `write_sidecar`.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2; CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the same ignored `graphify-out/` layout locally.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `atomic writes and generic sidecar I/O` — defined or implemented in this focused filesystem module (INFERRED)

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
| 2 | `fn atomic_write` | `// md:fn atomic_write` |
| 3 | `impl FsBackend` (container) | `// md:impl FsBackend` |
| 4 | `fn write_sidecar` | `// md:impl FsBackend > fn write_sidecar` |
| 5 | `fn read_sidecar` | `// md:impl FsBackend > fn read_sidecar` |
