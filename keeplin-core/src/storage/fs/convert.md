# `storage/fs/convert.rs` — filesystem journal serialization and Change conversion

Self-contained companion for `keeplin-core/src/storage/fs/convert.rs`. It documents every source block in source order with complete code embedded for every leaf.

**How to navigate**: each source marker `// md:<Header> > …` maps to the matching header chain below.

---

## Overview

**Identification** — file-level module declarations and imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{Change, Notebook, Resource, Tag};
use crate::storage::note_log::{NoteLogEntry, VersionVector};
```

**What it does** — Owns filesystem journal serialization and Change conversion. This is a structural relocation from the former monolithic filesystem module; storage behavior, on-disk format version 8, serialization shapes, conflict resolution, and public `storage::fs::FsBackend` API are unchanged.

**Dependencies** — every binding above is either a crate symbol used directly by the blocks below or a sibling item exposed as `pub(super)`; expects: those symbols to preserve the signatures and behavior the relocated bodies already relied on, with compile-time failure on drift.

**Used by** — sibling modules under `storage/fs/` and callers of `crate::storage::fs::FsBackend`.

**Repeated context** — `FsBackend::FORMAT_VERSION` remains 8. Rust makes `mod.rs` fields visible to descendant modules; sibling-defined helpers used across files carry only `pub(super)`.

---

## LogEntry

**Identification** — private serde struct; marker `// md:LogEntry`.

**Code** — complete and verbatim:

```rust
// md:LogEntry
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct LogEntry {
    pub(super) timestamp: DateTime<Utc>,
    #[serde(default = "default_entity_type")]
    pub(super) entity_type: String,
    #[serde(alias = "note_id")]
    pub(super) entity_id: Uuid,
    pub(super) operation: String,
    pub(super) data: serde_json::Value,
}
```

**What it does** — One line of a per-device NDJSON global log: `timestamp`,
`entity_type` (defaults to `"note"` — v1 logs had no field), `entity_id`
(alias `"note_id"` for v1), `operation`, `data`. Plain-text lines that Syncthing
replicates.

**Used by** — `append_log`, the readers, `log_entry_to_change`, the snapshot
builders.

---

## fn default_entity_type

**Identification** — marker `// md:fn default_entity_type`.

**Code** — complete and verbatim:

```rust
// md:fn default_entity_type
pub(super) fn default_entity_type() -> String {
    "note".to_string()
}
```

**What it does** — `"note"` — the v1 serde default.

---

## EpochHeader

**Identification** — private serde struct; marker `// md:EpochHeader`.

**Code** — complete and verbatim:

```rust
// md:EpochHeader
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct EpochHeader {
    #[serde(rename = "__keeplin_epoch__")]
    pub(super) epoch: u64,
}
```

**What it does** — The first line of a compacted global log: a
`{"__keeplin_epoch__": n}` generation marker. The epoch increments on every
compaction so a byte-offset reader can notice the rewrite and restart.

**Used by** — `compact_global_log_locked`, `parse_epoch_header`,
`read_log_header`.

---

## fn parse_epoch_header

**Identification** — marker `// md:fn parse_epoch_header`.

**Code** — complete and verbatim:

```rust
// md:fn parse_epoch_header
pub(super) fn parse_epoch_header(line: &str) -> Option<u64> {
    serde_json::from_str::<EpochHeader>(line)
        .ok()
        .map(|h| h.epoch)
}
```

**What it does** — Parses a line as an `EpochHeader`, `None` for a normal
`LogEntry` line (which lacks the field).

**Used by** — every log reader (to skip/detect headers).

---

## fn fs_tombstone_value

**Identification** — marker `// md:fn fs_tombstone_value`.

**Code** — complete and verbatim:

```rust
// md:fn fs_tombstone_value
pub(super) fn fs_tombstone_value(
    deleted_at: DateTime<Utc>,
    vv: &VersionVector,
    last_writer: &str,
) -> serde_json::Value {
    serde_json::json!({
        "deleted_at": deleted_at,
        "vv": vv,
        "last_writer": last_writer,
    })
}
```

**What it does** — The global-log `data` payload for a delete:
`{deleted_at, vv, last_writer}` so `log_entry_to_change` reconstructs a delete
`Change` carrying everything `resolve` needs on the receiving device.

**Used by** — the delete paths and `snapshot_entry_from_value`.

---

## fn fs_assoc_value

**Identification** — marker `// md:fn fs_assoc_value`.

**Code** — complete and verbatim:

```rust
// md:fn fs_assoc_value
pub(super) fn fs_assoc_value(
    tag_id: Uuid,
    updated_at: DateTime<Utc>,
    vv: &VersionVector,
    last_writer: &str,
) -> serde_json::Value {
    serde_json::json!({
        "tag_id": tag_id,
        "updated_at": updated_at,
        "vv": vv,
        "last_writer": last_writer,
    })
}
```

**What it does** — The `data` payload for a note↔tag add/remove:
`{tag_id, updated_at, vv, last_writer}`.

**Used by** — `add/remove_note_tag`, the snapshot builder.

---

## fn snapshot_entry_from_sidecar

**Identification** — generic fn; marker `// md:fn snapshot_entry_from_sidecar`.

**Code** — complete and verbatim:

```rust
// md:fn snapshot_entry_from_sidecar
pub(super) fn snapshot_entry_from_sidecar<T: serde::Serialize + serde::de::DeserializeOwned>(
    bytes: &[u8],
    kind: &str,
    id: Uuid,
    ts: DateTime<Utc>,
) -> Option<LogEntry> {
    let concrete: T = serde_json::from_slice(bytes).ok()?;
    let value = serde_json::to_value(&concrete).ok()?;
    Some(snapshot_entry_from_value(kind, id, ts, value))
}
```

**What it does** — Builds a snapshot `LogEntry` for a notebook/tag/resource by
decoding its NDJSON sidecar into the concrete type and re-serialising
through `serde_json` — the same encoding `append_log` uses, so the entry
round-trips through `log_entry_to_change` identically. `None` when the sidecar
cannot be decoded.

**Used by** — `build_global_snapshot`.

---

## fn snapshot_entry_from_value

**Identification** — marker `// md:fn snapshot_entry_from_value`.

**Code** — complete and verbatim:

```rust
// md:fn snapshot_entry_from_value
pub(super) fn snapshot_entry_from_value(
    kind: &str,
    id: Uuid,
    ts: DateTime<Utc>,
    value: serde_json::Value,
) -> LogEntry {
    let deleted_at = value
        .get("deleted_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok());
    match deleted_at {
        Some(del) => {
            let vv = value
                .get("vv")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let last_writer = value
                .get("last_writer")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            LogEntry {
                timestamp: ts,
                entity_type: kind.to_string(),
                entity_id: id,
                operation: "delete".to_string(),
                data: fs_tombstone_value(del, &vv, &last_writer),
            }
        }
        None => LogEntry {
            timestamp: ts,
            entity_type: kind.to_string(),
            entity_id: id,
            operation: "create".to_string(),
            data: value,
        },
    }
}
```

**What it does** — A snapshot entry from an entity's JSON value: a live entity
becomes a `create` carrying the full record; a soft-deleted one becomes a
`delete` tombstone carrying `(deleted_at, vv, last_writer)` — exactly the
shapes `log_entry_to_change` reconstructs.

**Used by** — `snapshot_entry_from_sidecar`.

---

## fn fs_assoc_from_data

**Identification** — marker `// md:fn fs_assoc_from_data`.

**Code** — complete and verbatim:

```rust
// md:fn fs_assoc_from_data
pub(super) fn fs_assoc_from_data(
    data: &serde_json::Value,
    fallback_ts: DateTime<Utc>,
) -> (DateTime<Utc>, VersionVector, String) {
    let updated_at = data
        .get("updated_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(fallback_ts);
    let vv = data
        .get("vv")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let last_writer = data
        .get("last_writer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (updated_at, vv, last_writer)
}
```

**What it does** — Reconstructs `(updated_at, vv, last_writer)` from a
global-log `data` value, falling back to the entry timestamp and empty
vector/writer for pre-version records.

**Used by** — `log_entry_to_change`.

---

## fn fs_tombstone_from_data

**Identification** — marker `// md:fn fs_tombstone_from_data`.

**Code** — complete and verbatim:

```rust
// md:fn fs_tombstone_from_data
pub(super) fn fs_tombstone_from_data(
    data: &serde_json::Value,
    fallback_ts: DateTime<Utc>,
) -> (DateTime<Utc>, VersionVector, String) {
    let deleted_at = data
        .get("deleted_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(fallback_ts);
    let vv = data
        .get("vv")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let last_writer = data
        .get("last_writer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (deleted_at, vv, last_writer)
}
```

**What it does** — Reconstructs `(deleted_at, vv, last_writer)`, same
fallbacks (v1 records stored `{ "id": … }`).

**Used by** — `log_entry_to_change`.

---

## fn log_entry_to_change

**Identification** — `fn log_entry_to_change(entry: LogEntry) -> Option<Change>`;
marker `// md:fn log_entry_to_change`.

**Code** — complete and verbatim:

```rust
// md:fn log_entry_to_change
pub(super) fn log_entry_to_change(entry: LogEntry) -> Option<Change> {
    let id = entry.entity_id;
    let ts = entry.timestamp;
    match (entry.entity_type.as_str(), entry.operation.as_str()) {
        ("note", "create") | ("note", "note_create") => serde_json::from_value(entry.data)
            .ok()
            .map(|note| Change::NoteCreate { note }),
        ("note", "update") | ("note", "note_update") => serde_json::from_value(entry.data)
            .ok()
            .map(|note| Change::NoteUpdate { note }),
        ("note", "delete") | ("note", "note_delete") => {
            let (deleted_at, vv, last_writer) = fs_tombstone_from_data(&entry.data, ts);
            Some(Change::NoteDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            })
        }
        ("notebook", "create") => serde_json::from_value(entry.data)
            .ok()
            .map(|notebook| Change::NotebookCreate { notebook }),
        ("notebook", "update") => serde_json::from_value(entry.data)
            .ok()
            .map(|notebook| Change::NotebookUpdate { notebook }),
        ("notebook", "delete") => {
            let (deleted_at, vv, last_writer) = fs_tombstone_from_data(&entry.data, ts);
            Some(Change::NotebookDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            })
        }
        ("tag", "create") => serde_json::from_value(entry.data)
            .ok()
            .map(|tag| Change::TagCreate { tag }),
        ("tag", "update") => serde_json::from_value(entry.data)
            .ok()
            .map(|tag| Change::TagUpdate { tag }),
        ("tag", "delete") => {
            let (deleted_at, vv, last_writer) = fs_tombstone_from_data(&entry.data, ts);
            Some(Change::TagDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            })
        }
        ("note_tag", "add") => {
            let tag_id: Uuid = entry.data["tag_id"].as_str()?.parse().ok()?;
            let (updated_at, vv, last_writer) = fs_assoc_from_data(&entry.data, ts);
            Some(Change::NoteTagAdd {
                note_id: id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            })
        }
        ("note_tag", "remove") => {
            let tag_id: Uuid = entry.data["tag_id"].as_str()?.parse().ok()?;
            let (updated_at, vv, last_writer) = fs_assoc_from_data(&entry.data, ts);
            Some(Change::NoteTagRemove {
                note_id: id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            })
        }
        ("resource", "create") => {
            serde_json::from_value(entry.data)
                .ok()
                .map(|resource| Change::ResourceCreate {
                    resource,
                    data: None,
                })
        }
        ("resource", "delete") => {
            let (deleted_at, vv, last_writer) = fs_tombstone_from_data(&entry.data, ts);
            Some(Change::ResourceDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            })
        }
        _ => None,
    }
}
```

**What it does** — Converts one log line into a typed `Change`. `None` for
unrecognised `(entity_type, operation)` pairs (corruption, or a newer build's
rows) — callers log and skip. v1 compatibility: `"note"` accepts both
`"create"` and `"note_create"` style operations. Note deletes parse their
tombstone's vv/writer from the data when present (v1 records fall back to an
empty vector + entry timestamp) so a replayed delete keeps its causal metadata
instead of an empty vector a peer would treat as stale (issue #70). Resource
entries carry metadata only, `data: None` — Syncthing replicates each
`notes/{note_id}/resources/{hash}.knrs` blob independently.

**Used by** — `get_changes_since`, `receive_changes`.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2; CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the same ignored `graphify-out/` layout locally.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `filesystem journal serialization and Change conversion` — defined or implemented in this focused filesystem module (INFERRED)

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
| 2 | `LogEntry` | `// md:LogEntry` |
| 3 | `fn default_entity_type` | `// md:fn default_entity_type` |
| 4 | `EpochHeader` | `// md:EpochHeader` |
| 5 | `fn parse_epoch_header` | `// md:fn parse_epoch_header` |
| 6 | `fn fs_tombstone_value` | `// md:fn fs_tombstone_value` |
| 7 | `fn fs_assoc_value` | `// md:fn fs_assoc_value` |
| 8 | `fn snapshot_entry_from_sidecar` | `// md:fn snapshot_entry_from_sidecar` |
| 9 | `fn snapshot_entry_from_value` | `// md:fn snapshot_entry_from_value` |
| 10 | `fn fs_assoc_from_data` | `// md:fn fs_assoc_from_data` |
| 11 | `fn fs_tombstone_from_data` | `// md:fn fs_tombstone_from_data` |
| 12 | `fn log_entry_to_change` | `// md:fn log_entry_to_change` |
