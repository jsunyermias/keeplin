# `storage/fs/history.rs` — journal-derived HistoryRepository

Self-contained companion for `keeplin-core/src/storage/fs/history.rs`. It documents every source block in source order with complete code embedded for every leaf.

**How to navigate**: each source marker `// md:<Header> > …` maps to the matching header chain below.

---

## Overview

**Identification** — file-level module declarations and imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{Note, Notebook};
use crate::storage::backend::DEFAULT_HISTORY_LIMIT;
use crate::storage::note_log::NoteOp;
use crate::storage::{EntityVersion, HistoryRepository};

use super::convert::{parse_epoch_header, LogEntry};
use super::FsBackend;
```

**What it does** — Owns journal-derived HistoryRepository. This is a structural relocation from the former monolithic filesystem module; storage behavior, on-disk format version 8, serialization shapes, conflict resolution, and public `storage::fs::FsBackend` API are unchanged.

**Dependencies** — every binding above is either a crate symbol used directly by the blocks below or a sibling item exposed as `pub(super)`; expects: those symbols to preserve the signatures and behavior the relocated bodies already relied on, with compile-time failure on drift.

**Used by** — sibling modules under `storage/fs/` and callers of `crate::storage::fs::FsBackend`.

**Repeated context** — `FsBackend::FORMAT_VERSION` remains 8. Rust makes `mod.rs` fields visible to descendant modules; sibling-defined helpers used across files carry only `pub(super)`.

---

## impl FsBackend (global history)

**Identification** — the second inherent impl; marker
`// md:impl FsBackend (global history)`. One method.

**Code** — container: members documented as sub-blocks below: fn read_all_global_entries.

---

### fn read_all_global_entries

**Identification** — marker
`// md:impl FsBackend (global history) > fn read_all_global_entries`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend (global history) > fn read_all_global_entries
    pub(super) async fn read_all_global_entries(
        &self,
    ) -> Result<Vec<(String, LogEntry)>, StorageError> {
        let dir = self.root.join("logs");
        let mut out = Vec::new();
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(device) = name.strip_suffix(".log") else {
                continue;
            };
            let bytes = tokio::fs::read(entry.path()).await?;
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines() {
                if line.trim().is_empty() || parse_epoch_header(line).is_some() {
                    continue;
                }
                if let Ok(le) = serde_json::from_str::<LogEntry>(line) {
                    out.push((device.to_string(), le));
                }
            }
        }
        Ok(out)
    }
```

**What it does** — Reads every global log (`logs/*.log`) and returns each
change entry paired with the writing device (the file stem). Headers, blanks,
and unparseable lines skipped. Used only by notebook history.

---

## impl HistoryRepository for FsBackend

**Identification** — marker `// md:impl HistoryRepository for FsBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn note_history, fn notebook_history.

**What it does** — journal-derived history.

---

### fn note_history

**Identification** — marker
`// md:impl HistoryRepository for FsBackend > fn note_history`.

**Code** — complete and verbatim:

```rust
    // md:impl HistoryRepository for FsBackend > fn note_history
    async fn note_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Note>>, StorageError> {
        let logs = self.read_note_logs(id).await?;
        let mut versions: Vec<EntityVersion<Note>> = logs
            .into_iter()
            .flatten()
            .map(|e| EntityVersion {
                timestamp: e.timestamp,
                device_id: e.device_id,
                entity: match e.op {
                    NoteOp::Upsert(note) => Some(note),
                    NoteOp::Tombstone { .. } => None,
                },
            })
            .collect();
        sort_and_cap(&mut versions, limit);
        Ok(versions)
    }
```

**What it does** — The per-note op logs already *are* the history: every
entry becomes an `EntityVersion` (`Upsert` → snapshot, `Tombstone` → `None`),
sorted newest-first and capped by `sort_and_cap`. Depth is bounded by the
256-entry compaction.

---

### fn notebook_history

**Identification** — marker
`// md:impl HistoryRepository for FsBackend > fn notebook_history`.

**Code** — complete and verbatim:

```rust
    // md:impl HistoryRepository for FsBackend > fn notebook_history
    async fn notebook_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Notebook>>, StorageError> {
        let entries = self.read_all_global_entries().await?;
        let mut versions: Vec<EntityVersion<Notebook>> = entries
            .into_iter()
            .filter(|(_, le)| le.entity_type == "notebook" && le.entity_id == id)
            .filter_map(|(device, le)| {
                let entity = match le.operation.as_str() {
                    "create" | "update" => Some(serde_json::from_value::<Notebook>(le.data).ok()?),
                    "delete" => None,
                    _ => return None,
                };
                Some(EntityVersion {
                    timestamp: le.timestamp,
                    device_id: device,
                    entity,
                })
            })
            .collect();
        sort_and_cap(&mut versions, limit);
        Ok(versions)
    }
```

**What it does** — Notebooks are state-based sidecars, so their history lives
only in the global journal — which compacts to current state, so this is
**best-effort**: whatever versions the journal still holds
(create/update → snapshot, delete → tombstone, per writing device).

---

## fn sort_and_cap

**Identification** — `fn sort_and_cap<T>(versions, limit)`; marker
`// md:fn sort_and_cap`.

**Code** — complete and verbatim:

```rust
// md:fn sort_and_cap
fn sort_and_cap<T>(versions: &mut Vec<EntityVersion<T>>, limit: u32) {
    versions.sort_by(|a, b| {
        b.timestamp
            .cmp(&a.timestamp)
            .then_with(|| b.device_id.cmp(&a.device_id))
    });
    let cap = if limit == 0 {
        DEFAULT_HISTORY_LIMIT
    } else {
        limit
    } as usize;
    versions.truncate(cap);
}
```

**What it does** — Orders a history list newest-first
(`(timestamp, device_id)` descending — the same total order the merge
tiebreaks on) and truncates to `limit` (`0` → `DEFAULT_HISTORY_LIMIT`).

**Used by** — both history methods.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2; CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the same ignored `graphify-out/` layout locally.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `journal-derived HistoryRepository` — defined or implemented in this focused filesystem module (INFERRED)

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
| 2 | `impl FsBackend (global history)` (container) | `// md:impl FsBackend (global history)` |
| 3 | `fn read_all_global_entries` | `// md:impl FsBackend (global history) > fn read_all_global_entries` |
| 4 | `impl HistoryRepository for FsBackend` (container) | `// md:impl HistoryRepository for FsBackend` |
| 5 | `fn note_history` | `// md:impl HistoryRepository for FsBackend > fn note_history` |
| 6 | `fn notebook_history` | `// md:impl HistoryRepository for FsBackend > fn notebook_history` |
| 7 | `fn sort_and_cap` | `// md:fn sort_and_cap` |
