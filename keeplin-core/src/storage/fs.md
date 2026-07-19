# `storage/fs.rs` — FsBackend (filesystem storage)

Self-contained companion for `keeplin-core/src/storage/fs.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must
be able to understand it without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{new_id, now, Change, Note, NoteTag, Notebook, Resource, Tag},
};

use super::backend::DEFAULT_HISTORY_LIMIT;
use super::note_log::{self, resolve, NoteLogEntry, NoteOp, VersionVector, Winner};
use super::{
    EntityVersion, HistoryRepository, NoteRepository, NotebookRepository, NotebookSortProfile,
    ResourceRepository, SortableRfc3339, SyncBackend, TagRepository,
};
```

**What it does** — The filesystem `StorageBackend`: files under a user-chosen root
that Syncthing (or any equivalent) replicates between devices. Two storage models:

- **Notes — per-device logs with version-vector merge.** Each note is a directory
  `notes/{id}/` holding `note.md` (materialised body; ciphertext under
  encryption), `meta.msgpack` (materialised metadata + merged vector — a cache),
  and `log.{device_id}.msgpack` — an append-only op log written **only** by that
  device. Single-writer logs never conflict under Syncthing; a note's true state
  is the merge of all its logs (`note_log::merge`). Projections are regenerated on
  every write and sync; **reads materialise live from the logs** and never write.
- **Notebooks, tags, resources — sidecars + global change log.** One MessagePack
  sidecar per entity, every mutation appended as an NDJSON line to
  `logs/{device_id}.log`; `receive_changes` reads new foreign entries via a
  byte-offset cursor.

Log growth is bounded: per-note logs compact to their frontier past
`NOTE_LOG_COMPACT_THRESHOLD` (256) entries (`note_log::compact_own_log`, lossless
for `merge`); the global journal compacts to a **current-state snapshot** behind a
bumped generation-epoch header once this device's own log passes
`GLOBAL_LOG_COMPACT_THRESHOLD` (512) entries — a peer notices the epoch change and
re-reads the snapshot from the start; every entry is version-vector resolved and
idempotent, so replaying converges. `prune_change_journal` stays a no-op —
compaction, not time-based deletion, does the bounding.

**Dependencies** — `tokio::fs`/io, `rmp_serde` (MessagePack), `serde_json`
(NDJSON), `note_log`, the trait family, `SortableRfc3339`.

**Used by** — `keeplin-daemon/src/main.rs` (`storage = "filesystem"` mode, the
default), `migrate.rs`, in-crate tests of many modules (the cheapest real
backend), `tests/fs_backend.rs`.

**Repeated context** — soft-delete-always, idempotent `apply_change`, and the
`(timestamp, device_id)` tiebreak, exactly as in `DbBackend` — same decisions,
different storage shape.

---

## NoteMeta

**Identification** — private serde struct; marker `// md:NoteMeta`.

**Code** — complete and verbatim:

```rust
// md:NoteMeta
#[derive(Debug, Serialize, Deserialize)]
struct NoteMeta {
    note: Note,
    vv: VersionVector,
}
```

**What it does** — The materialised projection written to
`notes/{id}/meta.msgpack`: the merged note (body blanked — it lives in `note.md`)
plus the merged version vector. A local cache regenerated from the logs; never
the source of truth for resolution.

**Used by** — `persist_note_projection`, `read_note_projection`, `note_vv`.

---

## NoteMetaEntry

**Identification** — private struct; marker `// md:NoteMetaEntry`.

**Code** — complete and verbatim:

```rust
// md:NoteMetaEntry
#[derive(Debug, Clone)]
struct NoteMetaEntry {
    notebook_id: Uuid,
    created_at: DateTime<Utc>,
    effective_sort_key: u32,
    is_starred: bool,
}
```

**What it does** — One **live** note's listing/ordering metadata for the
in-memory index: `notebook_id`, `created_at`, `effective_sort_key` (the `0`
sentinel already mapped), `is_starred`. Deliberately tiny — no title or body —
so the index is bounded by note count, not corpus size.

**Used by** — `NoteMetaIndex`.

---

## impl NoteMetaEntry

**Identification** — inherent impl; marker `// md:impl NoteMetaEntry`. One
method.

**Code** — container: members documented as sub-blocks below: fn from_note.

### fn from_note

**Identification** — marker `// md:impl NoteMetaEntry > fn from_note`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteMetaEntry > fn from_note
    fn from_note(note: &Note) -> Self {
        Self {
            notebook_id: note.notebook_id,
            created_at: note.created_at,
            effective_sort_key: note.effective_sort_key(),
            is_starred: note.is_starred,
        }
    }
```

**What it does** — Projects a `Note` to its entry.

---

## NoteMetaIndex

**Identification** — private `#[derive(Debug, Default)]` struct; marker
`// md:NoteMetaIndex`.

**Code** — complete and verbatim:

```rust
// md:NoteMetaIndex
#[derive(Debug, Default)]
struct NoteMetaIndex {
    entries: std::collections::HashMap<Uuid, NoteMetaEntry>,
}
```

**What it does** — In-memory `note_id → NoteMetaEntry` map of every live note,
so `list_notes` / `list_notes_in_notebook` / `list_starred_notes` /
`notebook_sort_profile` select, order and paginate without re-merging every
note's logs per call. Built lazily on the first listing (from the cheap
projections, full merge only for notes with none), then maintained
incrementally through `persist_note_projection`. **Freshness**: listings
reflect the last *materialised* state — updated on every local write and every
sync cycle; a Syncthing-replicated peer edit appears after the next cycle,
matching `DbBackend` (whose rows also change only on `apply_change`).
Single-note `read_note` stays a live merge, so reading a specific note is
always current.

**Used by** — the listing methods via `with_note_index`.

---

## impl NoteMetaIndex

**Identification** — inherent impl; marker `// md:impl NoteMetaIndex`. One
method.

**Code** — container: members documented as sub-blocks below: fn apply.

### fn apply

**Identification** — marker `// md:impl NoteMetaIndex > fn apply`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteMetaIndex > fn apply
    fn apply(&mut self, note: &Note) {
        if note.deleted_at.is_some() {
            self.entries.remove(&note.id);
        } else {
            self.entries.insert(note.id, NoteMetaEntry::from_note(note));
        }
    }
```

**What it does** — Reflects a note's current state: live → (re-)insert;
tombstoned → drop (listings exclude soft-deleted notes).

---

## NoteTagState

**Identification** — private serde struct; marker `// md:NoteTagState`.

**Code** — complete and verbatim:

```rust
// md:NoteTagState
#[derive(Debug, Clone, Serialize, Deserialize)]
struct NoteTagState {
    updated_at: DateTime<Utc>,
    #[serde(default)]
    deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    vv: VersionVector,
    #[serde(default)]
    last_writer: String,
}
```

**What it does** — The versioned state of one note↔tag association, stored as
the MessagePack contents of `note_tags/{note}/{tag}` (previously an empty
marker file): `updated_at`, `deleted_at` (`None` = attached, `Some` = tombstone
kept so a remove can beat a concurrent add), `vv`, `last_writer` — all
`serde(default)` so old records parse.

**Used by** — the association helpers and `add/remove_note_tag`,
`apply_change`.

---

## LogEntry

**Identification** — private serde struct; marker `// md:LogEntry`.

**Code** — complete and verbatim:

```rust
// md:LogEntry
#[derive(Debug, Serialize, Deserialize)]
struct LogEntry {
    timestamp: DateTime<Utc>,
    #[serde(default = "default_entity_type")]
    entity_type: String,
    #[serde(alias = "note_id")]
    entity_id: Uuid,
    operation: String,
    data: serde_json::Value,
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
fn default_entity_type() -> String {
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
struct EpochHeader {
    #[serde(rename = "__keeplin_epoch__")]
    epoch: u64,
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
fn parse_epoch_header(line: &str) -> Option<u64> {
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
fn fs_tombstone_value(
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
fn fs_assoc_value(
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
fn snapshot_entry_from_sidecar<T: serde::Serialize + serde::de::DeserializeOwned>(
    bytes: &[u8],
    kind: &str,
    id: Uuid,
    ts: DateTime<Utc>,
) -> Option<LogEntry> {
    let concrete: T = rmp_serde::from_slice(bytes).ok()?;
    let value = serde_json::to_value(&concrete).ok()?;
    Some(snapshot_entry_from_value(kind, id, ts, value))
}
```

**What it does** — Builds a snapshot `LogEntry` for a notebook/tag/resource by
decoding its MessagePack sidecar into the concrete type and re-serialising
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
fn snapshot_entry_from_value(
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
fn fs_assoc_from_data(
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
fn fs_tombstone_from_data(
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
fn log_entry_to_change(entry: LogEntry) -> Option<Change> {
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
entries carry metadata only, `data: None` — Syncthing replicates
`resources/{id}/data` independently.

**Used by** — `get_changes_since`, `receive_changes`.

---

## fn atomic_write

**Identification** — `async fn atomic_write(path: &Path, bytes: &[u8])`; marker
`// md:fn atomic_write`.

**Code** — complete and verbatim:

```rust
// md:fn atomic_write
async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
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

## SyncState

**Identification** — private serde struct; marker `// md:SyncState`.

**Code** — complete and verbatim:

```rust
// md:SyncState
#[derive(Debug, Serialize, Deserialize)]
struct SyncState {
    last_sync: DateTime<Utc>,
}
```

**What it does** — The contents of `.keeplin/sync_state.msgpack`: `last_sync`,
the watermark `get_changes_since` filters against.

**Used by** — `get_last_sync_time`, `update_sync_time`.

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
  notes/{uuid}/meta.msgpack               — metadata + merged vv (cache)
  notes/{uuid}/log.{device_id}.msgpack    — that device's op log (source of truth)
  notebooks/{uuid}.msgpack                — sidecar
  tags/{uuid}.msgpack                     — sidecar
  note_tags/{note}/{tag}                  — versioned association state
  resources/{uuid}/meta.msgpack + data    — metadata + raw payload
  logs/{device_id}.log                    — global NDJSON log (optional epoch header)
  .keeplin/device_id | format_version | sync_state.msgpack | offsets/{device_id}
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

## impl FsBackend

**Identification** — the first inherent impl; marker `// md:impl FsBackend`.
Constructor, sweeps, format versioning, path helpers, log machinery,
sidecar/association/resource versioning, the note merge pipeline. Contains the
constants `FORMAT_VERSION = 5`, `NOTE_LOG_COMPACT_THRESHOLD = 256`,
`GLOBAL_LOG_COMPACT_THRESHOLD = 512`, `GLOBAL_LOG_SOFT_BYTES = 64 KiB`
(documented with the methods that use them).

**Code** — container: members documented as sub-blocks below: fn new, fn sweep_orphan_tmp_files, fn scan_sync_conflicts, fn sweep_tmp_in_dir, fn format_version_path, fn ensure_format_version, fn apply_format_migration, fn note_dir, fn note_md_path, fn note_meta_path, fn note_log_path, fn device_log_path, fn notebook_path, fn tag_path, fn note_tag_dir, fn note_tag_path, fn resource_dir, fn resource_meta_path, fn resource_data_path, fn read_or_create_device_id, fn append_log, fn maybe_compact_global_log_locked, fn own_log_entry_count, fn read_own_epoch, fn compact_global_log_locked, fn build_global_snapshot, fn log_offset_path, fn read_log_offset, fn write_log_offset, fn read_log_header, fn read_other_logs_since, fn read_new_entries, fn write_sidecar, fn read_sidecar, fn sidecar_vv, fn next_sidecar_vv, fn sidecar_incoming_wins, fn read_assoc_state, fn next_assoc_vv, fn assoc_incoming_wins, fn write_assoc_state, fn read_resource_meta, fn next_resource_vv, fn resource_incoming_wins, fn note_vv, fn read_note_logs, fn merge_note, fn materialize, fn persist_note_projection, fn read_note_projection, fn with_note_index, fn build_note_index, fn materialize_page, fn append_note_op, fn collect_advanced_notes.

### fn new

**Identification** — `pub async fn new(root) -> Result<Self, StorageError>`;
marker `// md:impl FsBackend > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn new
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let root: PathBuf = root.into();

        for dir in &[
            "notes",
            "resources",
            ".keeplin",
            ".keeplin/offsets",
            "logs",
            "notebooks",
            "tags",
            "note_tags",
        ] {
            tokio::fs::create_dir_all(root.join(dir)).await?;
        }

        let removed = Self::sweep_orphan_tmp_files(&root).await;
        if removed > 0 {
            tracing::info!(
                removed,
                "Removed orphaned .tmp files left by interrupted writes"
            );
        }

        let conflicts = Self::scan_sync_conflicts(&root).await;
        if !conflicts.is_empty() {
            for path in &conflicts {
                tracing::error!(path = %path.display(), "Syncthing conflict copy detected");
            }
            tracing::error!(
                count = conflicts.len(),
                "Syncthing '*.sync-conflict-*' files exist in this store. Every keeplin \
                 file has a single writer, so conflict copies mean two devices are \
                 fighting over the same files — almost always because `.keeplin/` (this \
                 device's identity) was replicated instead of excluded via .stignore. \
                 Fix the Syncthing ignore rules (see README, 'Multi-device setup with \
                 Syncthing'), then reconcile each conflict copy manually before trusting \
                 further writes."
            );
        }

        let (device_id, fresh) = Self::read_or_create_device_id(&root).await?;
        let backend = Self {
            root,
            device_id,
            note_write_lock: Arc::new(Mutex::new(())),
            global_log_lock: Arc::new(Mutex::new(())),
            note_index: Arc::new(RwLock::new(None)),
        };
        backend.ensure_format_version(fresh).await?;
        Ok(backend)
    }
```

**What it does** — Creates the directory tree, sweeps orphaned `*.tmp` files,
scans for Syncthing `*.sync-conflict-*` copies (reported at **error** level —
in a single-writer-per-file store they are the signature of a replicated
`.keeplin/` directory, i.e. two devices sharing one identity; nothing is
deleted), loads or creates the device id, and runs `ensure_format_version`
(`fresh` = the id was just created).

**Used by** — `main.rs::build_storage` (default mode); tests everywhere.

### fn sweep_orphan_tmp_files

**Identification** — marker `// md:impl FsBackend > fn sweep_orphan_tmp_files`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn sweep_orphan_tmp_files
    async fn sweep_orphan_tmp_files(root: &Path) -> usize {
        let mut removed = 0usize;
        for flat in ["notebooks", "tags", "logs", ".keeplin", ".keeplin/offsets"] {
            removed += Self::sweep_tmp_in_dir(&root.join(flat)).await;
        }
        for nested in ["notes", "note_tags", "resources"] {
            let Ok(mut rd) = tokio::fs::read_dir(root.join(nested)).await else {
                continue;
            };
            while let Ok(Some(entry)) = rd.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    removed += Self::sweep_tmp_in_dir(&entry.path()).await;
                }
            }
        }
        removed
    }
```

**What it does** — Best-effort startup removal of `*.tmp` files orphaned by
interrupted atomic writes, across the flat dirs and one level down inside
`notes/`/`note_tags/`/`resources/`. Syncthing's own `.syncthing.*.tmp`
in-flight temporaries are explicitly left alone. Errors ignored — hygiene,
never a startup blocker.

### fn scan_sync_conflicts

**Identification** — marker `// md:impl FsBackend > fn scan_sync_conflicts`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn scan_sync_conflicts
    async fn scan_sync_conflicts(root: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut dirs: Vec<PathBuf> = [
            "",
            "notebooks",
            "tags",
            "logs",
            ".keeplin",
            ".keeplin/offsets",
        ]
        .iter()
        .map(|d| root.join(d))
        .collect();
        for nested in ["notes", "note_tags", "resources"] {
            let Ok(mut rd) = tokio::fs::read_dir(root.join(nested)).await else {
                continue;
            };
            while let Ok(Some(entry)) = rd.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    dirs.push(entry.path());
                }
            }
        }
        for dir in dirs {
            let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
                continue;
            };
            while let Ok(Some(entry)) = rd.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.contains(".sync-conflict-")
                    && entry
                        .file_type()
                        .await
                        .map(|t| t.is_file())
                        .unwrap_or(false)
                {
                    found.push(entry.path());
                }
            }
        }
        found
    }
```

**What it does** — Read-only collection of every `*.sync-conflict-*` file in
the managed directories (and root). Nothing is deleted — the copies may hold
the only good version; the caller logs the findings with remediation guidance
(fix `.stignore`, reconcile manually).

### fn sweep_tmp_in_dir

**Identification** — marker `// md:impl FsBackend > fn sweep_tmp_in_dir`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn sweep_tmp_in_dir
    async fn sweep_tmp_in_dir(dir: &Path) -> usize {
        let mut removed = 0usize;
        let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
            return 0;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".tmp") || name.starts_with(".syncthing.") {
                continue;
            }
            if !entry
                .file_type()
                .await
                .map(|t| t.is_file())
                .unwrap_or(false)
            {
                continue;
            }
            if tokio::fs::remove_file(entry.path()).await.is_ok() {
                tracing::debug!(path = %entry.path().display(), "Removed orphaned temp file");
                removed += 1;
            }
        }
        removed
    }

    const FORMAT_VERSION: u32 = 5;

    const NOTE_LOG_COMPACT_THRESHOLD: usize = 256;

    const GLOBAL_LOG_COMPACT_THRESHOLD: usize = 512;

    const GLOBAL_LOG_SOFT_BYTES: u64 = 64 * 1024;
```

**What it does** — Non-recursive removal of orphaned `*.tmp` regular files in
one directory, skipping Syncthing temporaries.

### fn format_version_path

**Identification** — marker `// md:impl FsBackend > fn format_version_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn format_version_path
    fn format_version_path(&self) -> PathBuf {
        self.root.join(".keeplin").join("format_version")
    }
```

**What it does** — `.keeplin/format_version`.

### fn ensure_format_version

**Identification** — marker `// md:impl FsBackend > fn ensure_format_version`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn ensure_format_version
    async fn ensure_format_version(&self, fresh: bool) -> Result<(), StorageError> {
        let path = self.format_version_path();

        if fresh {
            tokio::fs::write(&path, Self::FORMAT_VERSION.to_string()).await?;
            return Ok(());
        }

        let current = if path.exists() {
            tokio::fs::read_to_string(&path)
                .await?
                .trim()
                .parse::<u32>()
                .unwrap_or(1)
        } else {
            1
        };

        if current > Self::FORMAT_VERSION {
            return Err(StorageError::InvalidState(format!(
                "on-disk format version {current} is newer than this build supports \
                 (max {}); upgrade keeplin to open this store",
                Self::FORMAT_VERSION
            )));
        }

        for version in (current + 1)..=Self::FORMAT_VERSION {
            self.apply_format_migration(version).await?;
            tokio::fs::write(&path, version.to_string()).await?;
            tracing::info!(version, "Applied filesystem format migration");
        }

        tokio::fs::write(&path, Self::FORMAT_VERSION.to_string()).await?;
        Ok(())
    }
```

**What it does** — Brings the store up to `FORMAT_VERSION` (5), stamping after
**each** step so a crash mid-ladder resumes from the last completed step. A
`fresh` store is stamped directly (no migration over empty data). A missing
stamp on an existing store means format `1`; a stamp **newer** than this build
is refused (`InvalidState`) so a downgrade cannot run against a layout it does
not understand. A final stamp write covers the already-current case.

### fn apply_format_migration

**Identification** — marker
`// md:impl FsBackend > fn apply_format_migration`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn apply_format_migration
    async fn apply_format_migration(&self, version: u32) -> Result<(), StorageError> {
        match version {
            2..=5 => Ok(()),
            other => Err(StorageError::InvalidState(format!(
                "no filesystem migration defined for format version {other}"
            ))),
        }
    }
```

**What it does** — The per-version step. Every bump so far is
parse-compatible, so v2–v5 are no-ops that advance the stamp: v2 = `LogEntry`
serde aliases; v3/v4 = versioned associations + resource tombstones via
`serde(default)`; v5 = optional `EpochHeader` + `epoch:offset` cursors (a
pre-v5 log is epoch 0, a bare-integer cursor is `(0, offset)`). A future
breaking change gets a real body here.

### fn note_dir

**Identification** — marker `// md:impl FsBackend > fn note_dir`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_dir
    fn note_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("notes").join(id.to_string())
    }
```

**What it does** — `{root}/notes/{id}`.

### fn note_md_path

**Identification** — marker `// md:impl FsBackend > fn note_md_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_md_path
    fn note_md_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("note.md")
    }
```

**What it does** — `…/note.md` (human-readable when unencrypted).

### fn note_meta_path

**Identification** — marker `// md:impl FsBackend > fn note_meta_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_meta_path
    fn note_meta_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("meta.msgpack")
    }
```

**What it does** — `…/meta.msgpack` (cache, not source of truth).

### fn note_log_path

**Identification** — marker `// md:impl FsBackend > fn note_log_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_log_path
    fn note_log_path(&self, id: Uuid, device_id: &str) -> PathBuf {
        self.note_dir(id).join(format!("log.{device_id}.msgpack"))
    }
```

**What it does** — `…/log.{device_id}.msgpack` (single-writer op log).

### fn device_log_path

**Identification** — marker `// md:impl FsBackend > fn device_log_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn device_log_path
    fn device_log_path(&self) -> PathBuf {
        self.root
            .join("logs")
            .join(format!("{}.log", self.device_id))
    }
```

**What it does** — `{root}/logs/{device_id}.log` (this device's global log).

### fn notebook_path

**Identification** — marker `// md:impl FsBackend > fn notebook_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn notebook_path
    fn notebook_path(&self, id: Uuid) -> PathBuf {
        self.root.join("notebooks").join(format!("{id}.msgpack"))
    }
```

**What it does** — `{root}/notebooks/{id}.msgpack`.

### fn tag_path

**Identification** — marker `// md:impl FsBackend > fn tag_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn tag_path
    fn tag_path(&self, id: Uuid) -> PathBuf {
        self.root.join("tags").join(format!("{id}.msgpack"))
    }
```

**What it does** — `{root}/tags/{id}.msgpack`.

### fn note_tag_dir

**Identification** — marker `// md:impl FsBackend > fn note_tag_dir`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_tag_dir
    fn note_tag_dir(&self, note_id: Uuid) -> PathBuf {
        self.root.join("note_tags").join(note_id.to_string())
    }
```

**What it does** — `{root}/note_tags/{note_id}`.

### fn note_tag_path

**Identification** — marker `// md:impl FsBackend > fn note_tag_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_tag_path
    fn note_tag_path(&self, note_id: Uuid, tag_id: Uuid) -> PathBuf {
        self.note_tag_dir(note_id).join(tag_id.to_string())
    }
```

**What it does** — `…/{tag_id}` — the association's versioned state file.

### fn resource_dir

**Identification** — marker `// md:impl FsBackend > fn resource_dir`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn resource_dir
    fn resource_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("resources").join(id.to_string())
    }
```

**What it does** — `{root}/resources/{id}`.

### fn resource_meta_path

**Identification** — marker `// md:impl FsBackend > fn resource_meta_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn resource_meta_path
    fn resource_meta_path(&self, id: Uuid) -> PathBuf {
        self.resource_dir(id).join("meta.msgpack")
    }
```

**What it does** — `…/meta.msgpack`.

### fn resource_data_path

**Identification** — marker `// md:impl FsBackend > fn resource_data_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn resource_data_path
    fn resource_data_path(&self, id: Uuid) -> PathBuf {
        self.resource_dir(id).join("data")
    }
```

**What it does** — `…/data` (raw payload; `nonce ‖ ciphertext` under
encryption).

### fn read_or_create_device_id

**Identification** — marker
`// md:impl FsBackend > fn read_or_create_device_id`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_or_create_device_id
    async fn read_or_create_device_id(root: &Path) -> Result<(String, bool), StorageError> {
        let path = root.join(".keeplin").join("device_id");
        if path.exists() {
            let id = tokio::fs::read_to_string(&path).await?;
            Ok((id.trim().to_string(), false))
        } else {
            let id = new_id().to_string();
            tokio::fs::write(&path, &id).await?;
            Ok((id, true))
        }
    }
```

**What it does** — Reads `.keeplin/device_id`, or generates + persists a UUID
v4. Returns `(id, fresh)` — the file is the first thing written on init, so
its absence reliably means "never initialised" (used to stamp new stores at
the current format). The id names this device's log file and is the Argon2id
salt for `EncryptedBackend`; it must stay stable.

### fn append_log

**Identification** — marker `// md:impl FsBackend > fn append_log`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn append_log
    async fn append_log(
        &self,
        entity_type: &str,
        entity_id: Uuid,
        operation: &str,
        data: serde_json::Value,
    ) -> Result<(), StorageError> {
        let _guard = self.global_log_lock.lock().await;
        let entry = LogEntry {
            timestamp: now(),
            entity_type: entity_type.to_string(),
            entity_id,
            operation: operation.to_string(),
            data,
        };
        let line = serde_json::to_string(&entry)? + "\n";
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.device_log_path())
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;
        drop(file);
        self.maybe_compact_global_log_locked().await
    }
```

**What it does** — Appends one `LogEntry` (one JSON line) to this device's
global log under `global_log_lock`, then may compact
(`maybe_compact_global_log_locked`). The lock ensures a concurrent append is
never lost to a compaction rewriting the file.

**Used by** — every notebook/tag/resource/association mutation.

### fn maybe_compact_global_log_locked

**Identification** — marker
`// md:impl FsBackend > fn maybe_compact_global_log_locked`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn maybe_compact_global_log_locked
    async fn maybe_compact_global_log_locked(&self) -> Result<(), StorageError> {
        let path = self.device_log_path();
        let size = match tokio::fs::metadata(&path).await {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        if size < Self::GLOBAL_LOG_SOFT_BYTES {
            return Ok(());
        }
        if self.own_log_entry_count().await? <= Self::GLOBAL_LOG_COMPACT_THRESHOLD {
            return Ok(());
        }
        self.compact_global_log_locked().await
    }
```

**What it does** — Compacts when past the threshold; a cheap `metadata` size
gate (`GLOBAL_LOG_SOFT_BYTES`, 64 KiB) skips the line count entirely for small
logs. Caller must hold `global_log_lock`.

### fn own_log_entry_count

**Identification** — marker `// md:impl FsBackend > fn own_log_entry_count`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn own_log_entry_count
    async fn own_log_entry_count(&self) -> Result<usize, StorageError> {
        let file = match tokio::fs::File::open(self.device_log_path()).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut count = 0usize;
        loop {
            line.clear();
            if reader.read_line(&mut line).await? == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() || parse_epoch_header(trimmed).is_some() {
                continue;
            }
            count += 1;
        }
        Ok(count)
    }
```

**What it does** — Counts change entries (excluding the epoch header and
blanks) in this device's log.

### fn read_own_epoch

**Identification** — marker `// md:impl FsBackend > fn read_own_epoch`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_own_epoch
    async fn read_own_epoch(&self) -> Result<u64, StorageError> {
        let (epoch, _len) = self.read_log_header(&self.device_log_path()).await?;
        Ok(epoch)
    }
```

**What it does** — This device's log's generation epoch (0 = never
compacted).

### fn compact_global_log_locked

**Identification** — marker
`// md:impl FsBackend > fn compact_global_log_locked`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn compact_global_log_locked
    async fn compact_global_log_locked(&self) -> Result<(), StorageError> {
        let new_epoch = self.read_own_epoch().await?.saturating_add(1);
        let (entries, unreadable) = self.build_global_snapshot().await?;
        if unreadable > 0 {
            tracing::error!(
                unreadable,
                "Global-log compaction skipped: unreadable sidecar file(s) would be \
                 silently dropped from the snapshot (see the errors above for paths). \
                 The journal keeps appending until they are repaired or restored from \
                 a backup or another device."
            );
            return Ok(());
        }

        let mut buf = serde_json::to_string(&EpochHeader { epoch: new_epoch })?;
        buf.push('\n');
        for entry in &entries {
            buf.push_str(&serde_json::to_string(entry)?);
            buf.push('\n');
        }

        atomic_write(&self.device_log_path(), buf.as_bytes()).await?;
        tracing::info!(
            epoch = new_epoch,
            entries = entries.len(),
            "Compacted global change log to a snapshot"
        );
        Ok(())
    }
```

**What it does** — Rewrites the global log as a current-state snapshot behind
a bumped epoch header (atomic write): notebooks/tags/resources/associations
each collapse to one entry (create/add or delete/remove tombstone), bounding
the log by entity count. Peers re-read the snapshot (epoch changed) and
converge because every entry is version-vector resolved and idempotent.
**Declines to run while any sidecar is unreadable** — the rewrite destroys
history, so an undecodable entity would silently vanish from the snapshot and
a lagging peer would never learn it existed; skipping is always safe (the
journal just keeps growing) and compaction resumes once the sidecar is
repaired.

### fn build_global_snapshot

**Identification** — marker `// md:impl FsBackend > fn build_global_snapshot`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn build_global_snapshot
    async fn build_global_snapshot(&self) -> Result<(Vec<LogEntry>, usize), StorageError> {
        let ts = now();
        let mut out = Vec::new();
        let mut unreadable = 0usize;

        for (dir, kind) in [("notebooks", "notebook"), ("tags", "tag")] {
            let mut rd = match tokio::fs::read_dir(self.root.join(dir)).await {
                Ok(rd) => rd,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            while let Some(e) = rd.next_entry().await? {
                let fname = e.file_name().to_string_lossy().into_owned();
                let Some(stem) = fname.strip_suffix(".msgpack") else {
                    continue;
                };
                let Ok(id) = Uuid::parse_str(stem) else {
                    continue;
                };
                let bytes = tokio::fs::read(e.path()).await?;
                let entry = match kind {
                    "notebook" => snapshot_entry_from_sidecar::<Notebook>(&bytes, kind, id, ts),
                    _ => snapshot_entry_from_sidecar::<Tag>(&bytes, kind, id, ts),
                };
                match entry {
                    Some(e) => out.push(e),
                    None => {
                        unreadable += 1;
                        tracing::error!(
                            path = %e.path().display(),
                            "Unreadable {kind} sidecar; restore it from a backup or \
                             another device (global-log compaction is paused until then)"
                        );
                    }
                }
            }
        }

        if let Ok(mut rd) = tokio::fs::read_dir(self.root.join("resources")).await {
            while let Some(e) = rd.next_entry().await? {
                let Ok(id) = Uuid::parse_str(&e.file_name().to_string_lossy()) else {
                    continue;
                };
                let meta_path = self.resource_meta_path(id);
                let bytes = match tokio::fs::read(&meta_path).await {
                    Ok(bytes) => bytes,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e.into()),
                };
                match snapshot_entry_from_sidecar::<Resource>(&bytes, "resource", id, ts) {
                    Some(e) => out.push(e),
                    None => {
                        unreadable += 1;
                        tracing::error!(
                            path = %meta_path.display(),
                            "Unreadable resource sidecar; restore it from a backup or \
                             another device (global-log compaction is paused until then)"
                        );
                    }
                }
            }
        }

        if let Ok(mut notes_rd) = tokio::fs::read_dir(self.root.join("note_tags")).await {
            while let Some(note_entry) = notes_rd.next_entry().await? {
                let Ok(note_id) = Uuid::parse_str(&note_entry.file_name().to_string_lossy()) else {
                    continue;
                };
                let mut tags_rd = tokio::fs::read_dir(note_entry.path()).await?;
                while let Some(tag_entry) = tags_rd.next_entry().await? {
                    let Ok(tag_id) = Uuid::parse_str(&tag_entry.file_name().to_string_lossy())
                    else {
                        continue;
                    };
                    let Some(state) = self.read_assoc_state(&tag_entry.path()).await? else {
                        continue;
                    };
                    let op = if state.deleted_at.is_some() {
                        "remove"
                    } else {
                        "add"
                    };
                    out.push(LogEntry {
                        timestamp: ts,
                        entity_type: "note_tag".to_string(),
                        entity_id: note_id,
                        operation: op.to_string(),
                        data: fs_assoc_value(
                            tag_id,
                            state.updated_at,
                            &state.vv,
                            &state.last_writer,
                        ),
                    });
                }
            }
        }

        Ok((out, unreadable))
    }
```

**What it does** — Builds the snapshot entries: notebooks + tags from their
sidecar directories, resources from their metadata sidecars (a missing meta is
a crashed-create orphan, skipped — not corruption), associations from their
state files. Notes are excluded — they sync through per-note logs. Returns
`(entries, unreadable)`; each unreadable sidecar is reported at error level
with its path and pauses compaction (see above).

### fn log_offset_path

**Identification** — marker `// md:impl FsBackend > fn log_offset_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn log_offset_path
    fn log_offset_path(&self, device_id: &str) -> PathBuf {
        self.root.join(".keeplin").join("offsets").join(device_id)
    }
```

**What it does** — `.keeplin/offsets/{device_id}` — the `"{epoch}:{offset}"`
cursor for a foreign log (a bare integer is a pre-v5 cursor, read as epoch 0).

### fn read_log_offset

**Identification** — marker `// md:impl FsBackend > fn read_log_offset`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_log_offset
    async fn read_log_offset(&self, device_id: &str) -> (u64, u64) {
        let raw = match tokio::fs::read_to_string(self.log_offset_path(device_id)).await {
            Ok(s) => s,
            Err(_) => return (0, 0),
        };
        let raw = raw.trim();
        match raw.split_once(':') {
            Some((epoch, offset)) => (
                epoch.trim().parse().unwrap_or(0),
                offset.trim().parse().unwrap_or(0),
            ),
            None => (0, raw.parse().unwrap_or(0)),
        }
    }
```

**What it does** — Reads the cursor, `(0, 0)` when absent/unreadable.

### fn write_log_offset

**Identification** — marker `// md:impl FsBackend > fn write_log_offset`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn write_log_offset
    async fn write_log_offset(
        &self,
        device_id: &str,
        epoch: u64,
        offset: u64,
    ) -> Result<(), StorageError> {
        atomic_write(
            &self.log_offset_path(device_id),
            format!("{epoch}:{offset}").as_bytes(),
        )
        .await
    }
```

**What it does** — Atomic write of `"{epoch}:{offset}"`. A torn cursor reads
as `(0, 0)` → safe re-delivery (apply is idempotent), just wasteful.

### fn read_log_header

**Identification** — marker `// md:impl FsBackend > fn read_log_header`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_log_header
    async fn read_log_header(&self, path: &Path) -> Result<(u64, u64), StorageError> {
        let file = match tokio::fs::File::open(path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((0, 0)),
            Err(e) => return Err(e.into()),
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        match parse_epoch_header(line.trim()) {
            Some(epoch) => Ok((epoch, n as u64)),
            None => Ok((0, 0)),
        }
    }
```

**What it does** — A log's `(epoch, header_byte_len)`; `(0, 0)` when there is
no header, so reading starts at byte 0. `header_byte_len` includes the newline
— exactly where the first change entry begins.

### fn read_other_logs_since

**Identification** — marker `// md:impl FsBackend > fn read_other_logs_since`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_other_logs_since
    async fn read_other_logs_since(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<LogEntry>, StorageError> {
        let mut entries = Vec::new();
        let logs_dir = self.root.join("logs");
        let mut dir = match tokio::fs::read_dir(&logs_dir).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
            Err(e) => return Err(e.into()),
        };
        while let Some(dir_entry) = dir.next_entry().await? {
            let fname = dir_entry.file_name().to_string_lossy().into_owned();
            if fname == format!("{}.log", self.device_id) {
                continue;
            }
            if !fname.ends_with(".log") {
                continue;
            }
            let file = tokio::fs::File::open(dir_entry.path()).await?;
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    break;
                }
                let trimmed = line.trim();
                if trimmed.is_empty() || parse_epoch_header(trimmed).is_some() {
                    continue;
                }
                match serde_json::from_str::<LogEntry>(trimmed) {
                    Ok(e) if e.timestamp > since => entries.push(e),
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!("Skipping malformed log line: {err}");
                    }
                }
            }
        }
        Ok(entries)
    }
```

**What it does** — Scans every **foreign** `.log` file from the beginning
(never advancing cursors) and returns entries with `timestamp > since`. Used
by `get_changes_since`, which needs a filtered view, not delivery tracking.
Own log skipped (local changes are already local state); non-`.log` files
skipped; malformed lines warned and skipped.

### fn read_new_entries

**Identification** — marker `// md:impl FsBackend > fn read_new_entries`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_new_entries
    async fn read_new_entries(&self) -> Result<Vec<LogEntry>, StorageError> {
        let mut entries = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("logs")).await?;
        while let Some(dir_entry) = dir.next_entry().await? {
            let fname = dir_entry.file_name().to_string_lossy().into_owned();
            if fname == format!("{}.log", self.device_id) {
                continue;
            }
            if !fname.ends_with(".log") {
                continue;
            }
            let device_id = fname.trim_end_matches(".log").to_owned();
            let (writer_epoch, header_len) = self.read_log_header(&dir_entry.path()).await?;
            let (stored_epoch, stored_offset) = self.read_log_offset(&device_id).await;
            let start = if writer_epoch != stored_epoch {
                header_len
            } else {
                stored_offset
            };

            let mut file = tokio::fs::File::open(dir_entry.path()).await?;
            file.seek(SeekFrom::Start(start)).await?;

            let mut reader = BufReader::new(file);
            let mut line = String::new();
            let mut new_offset = start;
            loop {
                line.clear();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    break;
                }
                new_offset += n as u64;
                let trimmed = line.trim();
                if trimmed.is_empty() || parse_epoch_header(trimmed).is_some() {
                    continue;
                }
                match serde_json::from_str::<LogEntry>(trimmed) {
                    Ok(e) => entries.push(e),
                    Err(err) => {
                        tracing::warn!("Skipping malformed log line: {err}");
                    }
                }
            }

            if writer_epoch != stored_epoch || new_offset > stored_offset {
                if let Err(e) = self
                    .write_log_offset(&device_id, writer_epoch, new_offset)
                    .await
                {
                    tracing::warn!("Could not save log offset for {device_id}: {e}");
                }
            }
        }
        Ok(entries)
    }
```

**What it does** — Reads all new entries from each foreign log since the last
call and advances the byte-offset cursor (exactly-once delivery). Generation
epochs: when the foreign log's epoch differs from the cursor's, the log was
compacted — the stale offset is discarded and reading restarts just past the
new header, re-delivering the snapshot (idempotent + resolved ⇒ converges). A
failed cursor write is only a warning: the entries re-deliver next call,
safely.

**Used by** — `receive_changes`.

### fn write_sidecar

**Identification** — marker `// md:impl FsBackend > fn write_sidecar`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn write_sidecar
    async fn write_sidecar<T: serde::Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), StorageError> {
        let bytes = rmp_serde::to_vec_named(value)
            .map_err(|e| StorageError::InvalidState(format!("msgpack encode: {e}")))?;
        atomic_write(path, &bytes).await
    }
```

**What it does** — MessagePack-encode + `atomic_write` (encode failure →
`InvalidState`).

### fn read_sidecar

**Identification** — marker `// md:impl FsBackend > fn read_sidecar`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_sidecar
    async fn read_sidecar<T: serde::de::DeserializeOwned>(
        &self,
        path: &Path,
        id: Uuid,
    ) -> Result<T, StorageError> {
        if !path.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let bytes = tokio::fs::read(path).await?;
        rmp_serde::from_slice(&bytes)
            .map_err(|e| StorageError::CorruptedData(format!("msgpack decode: {e}")))
    }
```

**What it does** — Read + decode; missing file → `NotFound(id)`, bad bytes →
`CorruptedData`.

### fn sidecar_vv

**Identification** — marker `// md:impl FsBackend > fn sidecar_vv`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn sidecar_vv
    async fn sidecar_vv(&self, path: &Path) -> Result<VersionVector, StorageError> {
        #[derive(serde::Deserialize)]
        struct VvProbe {
            #[serde(default)]
            vv: VersionVector,
        }
        if !path.exists() {
            return Ok(VersionVector::new());
        }
        let bytes = tokio::fs::read(path).await?;
        let probe: VvProbe = rmp_serde::from_slice(&bytes)
            .map_err(|e| StorageError::CorruptedData(format!("msgpack decode: {e}")))?;
        Ok(probe.vv)
    }
```

**What it does** — Deserialises only the `vv` field of a notebook/tag sidecar
(empty when the file is absent), to base a local write's incremented vector on
current state.

### fn next_sidecar_vv

**Identification** — marker `// md:impl FsBackend > fn next_sidecar_vv`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn next_sidecar_vv
    async fn next_sidecar_vv(&self, path: &Path) -> Result<VersionVector, StorageError> {
        let mut vv = self.sidecar_vv(path).await?;
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }
```

**What it does** — `sidecar_vv` + increment this device's component.

### fn sidecar_incoming_wins

**Identification** — marker `// md:impl FsBackend > fn sidecar_incoming_wins`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn sidecar_incoming_wins
    async fn sidecar_incoming_wins(
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
        let m: MetaProbe = rmp_serde::from_slice(&bytes)
            .map_err(|e| StorageError::CorruptedData(format!("msgpack decode: {e}")))?;
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

### fn read_assoc_state

**Identification** — marker `// md:impl FsBackend > fn read_assoc_state`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_assoc_state
    async fn read_assoc_state(&self, path: &Path) -> Result<Option<NoteTagState>, StorageError> {
        if !path.exists() {
            return Ok(None);
        }
        let marker = || NoteTagState {
            updated_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default(),
            deleted_at: None,
            vv: VersionVector::new(),
            last_writer: String::new(),
        };
        let bytes = tokio::fs::read(path).await?;
        if bytes.is_empty() {
            return Ok(Some(marker()));
        }
        match rmp_serde::from_slice(&bytes) {
            Ok(state) => Ok(Some(state)),
            Err(e) => {
                tracing::error!(
                    path = %path.display(),
                    "Unreadable note↔tag association state; treating it as attached with \
                     minimum priority so the next versioned peer state supersedes it \
                     (restore the file from a backup or another device to recover it): {e}"
                );
                Ok(Some(marker()))
            }
        }
    }
```

**What it does** — Reads an association's versioned state (`None` when
absent). Two degenerate shapes both fall back to a "present, minimum
priority" marker (epoch-0 timestamp, empty vector) for different reasons: an
**empty** file is the pre-versioning marker format (designed back-compat — any
versioned write dominates); a **non-empty unparseable** file is corruption,
and the same weakest-priority reading is the least-harm recovery — the
association stays visible locally instead of vanishing, and the next
versioned peer state supersedes it through `resolve`. The corrupt case is
reported at **error** level but stays non-fatal.

### fn next_assoc_vv

**Identification** — marker `// md:impl FsBackend > fn next_assoc_vv`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn next_assoc_vv
    async fn next_assoc_vv(&self, path: &Path) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .read_assoc_state(path)
            .await?
            .map(|s| s.vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }
```

**What it does** — Current association vector (empty if new) + increment.

### fn assoc_incoming_wins

**Identification** — marker `// md:impl FsBackend > fn assoc_incoming_wins`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn assoc_incoming_wins
    async fn assoc_incoming_wins(
        &self,
        path: &Path,
        incoming_vv: &VersionVector,
        incoming_updated: DateTime<Utc>,
        incoming_writer: &str,
    ) -> Result<bool, StorageError> {
        match self.read_assoc_state(path).await? {
            None => Ok(true),
            Some(s) => Ok(matches!(
                resolve(
                    &s.vv,
                    s.updated_at,
                    &s.last_writer,
                    incoming_vv,
                    incoming_updated,
                    incoming_writer,
                ),
                Winner::Incoming
            )),
        }
    }
```

**What it does** — `resolve` for association writes; `true` with no local
file.

### fn write_assoc_state

**Identification** — marker `// md:impl FsBackend > fn write_assoc_state`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn write_assoc_state
    async fn write_assoc_state(
        &self,
        note_id: Uuid,
        tag_id: Uuid,
        state: &NoteTagState,
    ) -> Result<(), StorageError> {
        tokio::fs::create_dir_all(self.note_tag_dir(note_id)).await?;
        self.write_sidecar(&self.note_tag_path(note_id, tag_id), state)
            .await
    }
```

**What it does** — Creates `note_tags/{note}` and writes the state sidecar.

### fn read_resource_meta

**Identification** — marker `// md:impl FsBackend > fn read_resource_meta`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_resource_meta
    async fn read_resource_meta(&self, id: Uuid) -> Result<Option<Resource>, StorageError> {
        match self
            .read_sidecar::<Resource>(&self.resource_meta_path(id), id)
            .await
        {
            Ok(r) => Ok(Some(r)),
            Err(StorageError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
```

**What it does** — The resource metadata sidecar, `None` when absent.

### fn next_resource_vv

**Identification** — marker `// md:impl FsBackend > fn next_resource_vv`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn next_resource_vv
    async fn next_resource_vv(&self, id: Uuid) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .read_resource_meta(id)
            .await?
            .map(|r| r.vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }
```

**What it does** — Current resource vector + increment.

### fn resource_incoming_wins

**Identification** — marker
`// md:impl FsBackend > fn resource_incoming_wins`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn resource_incoming_wins
    async fn resource_incoming_wins(
        &self,
        id: Uuid,
        incoming_vv: &VersionVector,
        incoming_ts: DateTime<Utc>,
        incoming_writer: &str,
    ) -> Result<bool, StorageError> {
        match self.read_resource_meta(id).await? {
            None => Ok(true),
            Some(r) => {
                let local_ts = r.deleted_at.unwrap_or(r.created_at);
                Ok(matches!(
                    resolve(
                        &r.vv,
                        local_ts,
                        &r.last_writer,
                        incoming_vv,
                        incoming_ts,
                        incoming_writer,
                    ),
                    Winner::Incoming
                ))
            }
        }
    }
```

**What it does** — `resolve` for resource changes; the tiebreak timestamp is
`deleted_at` when tombstoned else `created_at` (resources have no
`updated_at`); `true` with no local metadata.

### fn note_vv

**Identification** — marker `// md:impl FsBackend > fn note_vv`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_vv
    async fn note_vv(&self, id: Uuid) -> Result<VersionVector, StorageError> {
        match self
            .read_sidecar::<NoteMeta>(&self.note_meta_path(id), id)
            .await
        {
            Ok(meta) => Ok(meta.vv),
            Err(StorageError::NotFound(_)) => Ok(VersionVector::new()),
            Err(e) => Err(e),
        }
    }
```

**What it does** — A note's merged vector from its meta projection (empty when
none) — the "what did we last materialise" reference for
`collect_advanced_notes`.

### fn read_note_logs

**Identification** — marker `// md:impl FsBackend > fn read_note_logs`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_note_logs
    async fn read_note_logs(&self, id: Uuid) -> Result<Vec<Vec<NoteLogEntry>>, StorageError> {
        let dir = self.note_dir(id);
        let mut logs = Vec::new();
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(logs),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("log.") && name.ends_with(".msgpack") {
                let bytes = tokio::fs::read(entry.path()).await?;
                match rmp_serde::from_slice::<Vec<NoteLogEntry>>(&bytes) {
                    Ok(v) => logs.push(v),
                    Err(e) => tracing::error!(
                        note_id = %id,
                        path = %entry.path().display(),
                        "Unreadable note log excluded from merge — this device's history \
                         for the note is not being applied; restore the file from a backup \
                         or another device to recover it: {e}"
                    ),
                }
            }
        }
        Ok(logs)
    }
```

**What it does** — Reads every `log.*.msgpack` for a note. A missing directory
yields empty; an unreadable individual log is **excluded from the merge and
reported at error level** (that device's entire history is missing — a
silent-data-loss risk, not routine). The file is left in place (it belongs to
another device; a local rename would replicate back to its writer), so a
restored copy re-enters the merge on the next read.

### fn merge_note

**Identification** — marker `// md:impl FsBackend > fn merge_note`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn merge_note
    async fn merge_note(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
        let logs = self.read_note_logs(id).await?;
        Ok(note_log::merge(&logs).note)
    }
```

**What it does** — Merge without touching disk. Reads use this so a read never
rewrites projections (no write amplification) and never consumes a peer change
the next sync should report. `None` when the note has no entries.

### fn materialize

**Identification** — marker `// md:impl FsBackend > fn materialize`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn materialize
    async fn materialize(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
        let logs = self.read_note_logs(id).await?;
        let merged = note_log::merge(&logs);
        match merged.note {
            None => Ok(None),
            Some(note) => {
                if merged.conflict {
                    tracing::warn!(%id, "Concurrent note edit resolved by the (timestamp, device-id) tiebreak");
                }
                self.persist_note_projection(&note, &merged.vv).await?;
                Ok(Some(note))
            }
        }
    }
```

**What it does** — Merge + refresh the `note.md`/`meta.msgpack` projection
(used by write and sync paths, never reads); a resolved concurrent conflict is
logged.

### fn persist_note_projection

**Identification** — marker
`// md:impl FsBackend > fn persist_note_projection`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn persist_note_projection
    async fn persist_note_projection(
        &self,
        note: &Note,
        vv: &VersionVector,
    ) -> Result<(), StorageError> {
        tokio::fs::create_dir_all(self.note_dir(note.id)).await?;
        atomic_write(&self.note_md_path(note.id), note.body.as_bytes()).await?;
        let mut meta_note = note.clone();
        meta_note.body = String::new();
        self.write_sidecar(
            &self.note_meta_path(note.id),
            &NoteMeta {
                note: meta_note,
                vv: vv.clone(),
            },
        )
        .await?;
        if let Some(idx) = self.note_index.write().await.as_mut() {
            idx.apply(note);
        }
        Ok(())
    }
```

**What it does** — Writes the body to `note.md` and the blanked-body metadata
+ vector to `meta.msgpack` (both atomic). **The single choke point every note
write passes through**, so it also keeps the `NoteMetaIndex` current: on-disk
first, then the index entry (only if built — an unbuilt index misses nothing,
its eventual build reads the fresh projection). A crash between the two
leaves the index no staler than the projection.

### fn read_note_projection

**Identification** — marker
`// md:impl FsBackend > fn read_note_projection`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_note_projection
    async fn read_note_projection(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
        let meta: NoteMeta = match self.read_sidecar(&self.note_meta_path(id), id).await {
            Ok(m) => m,
            Err(StorageError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(Some(meta.note))
    }
```

**What it does** — The on-disk projection (metadata only; `None` when absent)
— used only to build the index cheaply.

### fn with_note_index

**Identification** — marker `// md:impl FsBackend > fn with_note_index`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn with_note_index
    async fn with_note_index<R>(
        &self,
        f: impl FnOnce(&NoteMetaIndex) -> R,
    ) -> Result<R, StorageError> {
        {
            let guard = self.note_index.read().await;
            if let Some(idx) = guard.as_ref() {
                return Ok(f(idx));
            }
        }
        let mut guard = self.note_index.write().await;
        if guard.is_none() {
            *guard = Some(self.build_note_index().await?);
        }
        Ok(f(guard.as_ref().expect("index was just built")))
    }
```

**What it does** — Runs `f` against the index, building it first when absent
(double-checked write lock → at most one concurrent build).

### fn build_note_index

**Identification** — marker `// md:impl FsBackend > fn build_note_index`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn build_note_index
    async fn build_note_index(&self) -> Result<NoteMetaIndex, StorageError> {
        let mut idx = NoteMetaIndex::default();
        let mut dir = match tokio::fs::read_dir(self.root.join("notes")).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(idx),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = dir.next_entry().await? {
            let Ok(id) = Uuid::parse_str(&entry.file_name().to_string_lossy()) else {
                continue;
            };
            let note = match self.read_note_projection(id).await {
                Ok(Some(note)) => Some(note),
                Ok(None) => self.merge_note(id).await?,
                Err(e) => {
                    tracing::warn!("Rebuilding note {id} from logs (projection unreadable): {e}");
                    self.merge_note(id).await?
                }
            };
            if let Some(note) = note {
                if note.deleted_at.is_none() {
                    idx.apply(&note);
                }
            }
        }
        Ok(idx)
    }
```

**What it does** — Scans every note directory; metadata from projections, full
merge for notes with none (a peer note never materialised here) or an
unreadable one. Only live notes are indexed.

### fn materialize_page

**Identification** — marker `// md:impl FsBackend > fn materialize_page`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn materialize_page
    async fn materialize_page(&self, ids: Vec<Uuid>) -> Result<Vec<Note>, StorageError> {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            match self.merge_note(id).await {
                Ok(Some(n)) if n.deleted_at.is_none() => out.push(n),
                Ok(_) => {}
                Err(e) => tracing::warn!("Could not merge note {id} for listing: {e}"),
            }
        }
        Ok(out)
    }
```

**What it does** — Merges a page's ids into full notes, skipping any that no
longer merge live (a race with concurrent delete/move). Page-bounded — merge
cost is paid only for the returned page.

### fn append_note_op

**Identification** — marker `// md:impl FsBackend > fn append_note_op`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn append_note_op
    async fn append_note_op(&self, id: Uuid, op: NoteOp) -> Result<Note, StorageError> {
        let _write_guard = self.note_write_lock.lock().await;
        tokio::fs::create_dir_all(self.note_dir(id)).await?;
        let mut vv = note_log::merge(&self.read_note_logs(id).await?).vv;
        note_log::increment(&mut vv, &self.device_id);
        let log_path = self.note_log_path(id, &self.device_id);
        let mut log: Vec<NoteLogEntry> = match self.read_sidecar(&log_path, id).await {
            Ok(v) => v,
            Err(StorageError::NotFound(_)) => Vec::new(),
            Err(e) => return Err(e),
        };
        log.push(NoteLogEntry {
            vv,
            timestamp: now(),
            device_id: self.device_id.clone(),
            op,
        });
        if log.len() > Self::NOTE_LOG_COMPACT_THRESHOLD {
            log = note_log::compact_own_log(&log);
        }
        self.write_sidecar(&log_path, &log).await?;
        self.materialize(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))
    }
```

**What it does** — **The single entry point for every local note mutation.**
Under `note_write_lock`: base the new entry's vector on the merge of every log
currently on disk (not the meta cache — so an edit causally follows all state
present at write time even though reads never refresh that cache) + increment;
read this device's log; append the entry; compact past
`NOTE_LOG_COMPACT_THRESHOLD` (single-writer log ⇒ `compact_own_log` is
lossless); atomic write; `materialize` and return the merged note (`NotFound`
if nothing merges).

### fn collect_advanced_notes

**Identification** — marker
`// md:impl FsBackend > fn collect_advanced_notes`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn collect_advanced_notes
    async fn collect_advanced_notes(&self) -> Result<Vec<Change>, StorageError> {
        let mut changes = Vec::new();
        let notes_dir = self.root.join("notes");
        let mut rd = match tokio::fs::read_dir(&notes_dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(changes),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            let id = match Uuid::parse_str(&entry.file_name().to_string_lossy()) {
                Ok(id) => id,
                Err(_) => continue,
            };
            let old_vv = self.note_vv(id).await?;
            let logs = self.read_note_logs(id).await?;
            let merged = note_log::merge(&logs);
            if merged.vv != old_vv {
                if let Some(note) = merged.note {
                    self.persist_note_projection(&note, &merged.vv).await?;
                    match note.deleted_at {
                        Some(deleted_at) => changes.push(Change::NoteDelete {
                            id,
                            deleted_at,
                            vv: merged.winner_vv.clone(),
                            last_writer: merged.winner_device.clone(),
                        }),
                        None => changes.push(Change::NoteUpdate { note }),
                    }
                }
            }
        }
        Ok(changes)
    }
```

**What it does** — Scans every note directory and re-materialises those whose
logs advanced beyond the stored projection (e.g. Syncthing just delivered a
peer's log). Comparison is by version vector, never file mtime — immune to
clock skew. Emits one `Change` per advanced note: `NoteUpdate` for live,
`NoteDelete` for tombstoned — carrying the **winning tombstone's own vv and
author**, not the joined frontier with an empty writer: a state-based peer
(`DbBackend`) resolves the delete by vector, and an empty vector would be
dominated by any local row, silently dropping the delete (issue #70).

**Used by** — `receive_changes`.

---

## KeyedItem

**Identification** — private `struct KeyedItem<T>`; marker `// md:KeyedItem`.

**Code** — complete and verbatim:

```rust
// md:KeyedItem
struct KeyedItem<T> {
    key: (String, Uuid),
    item: T,
}
```

**What it does** — An item tagged with its `(created_at_rfc3339, id)`
pagination key, ordered by the key alone so `PageCollector`'s max-heap can
evict the largest candidate.

---

## impl PartialEq for KeyedItem

**Identification** — marker `// md:impl PartialEq for KeyedItem`.

**Code** — complete and verbatim:

```rust
// md:impl PartialEq for KeyedItem
impl<T> PartialEq for KeyedItem<T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
```

**What it does** — Key equality.

---

## impl Eq for KeyedItem

**Identification** — marker `// md:impl Eq for KeyedItem`.

**Code** — complete and verbatim:

```rust
// md:impl Eq for KeyedItem
impl<T> Eq for KeyedItem<T> {}
```

**What it does** — Marker impl.

---

## impl PartialOrd for KeyedItem

**Identification** — marker `// md:impl PartialOrd for KeyedItem`.

**Code** — complete and verbatim:

```rust
// md:impl PartialOrd for KeyedItem
impl<T> PartialOrd for KeyedItem<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
```

**What it does** — Delegates to `cmp`.

---

## impl Ord for KeyedItem

**Identification** — marker `// md:impl Ord for KeyedItem`.

**Code** — complete and verbatim:

```rust
// md:impl Ord for KeyedItem
impl<T> Ord for KeyedItem<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}
```

**What it does** — Key ordering.

---

## PageCollector

**Identification** — private `struct PageCollector<T>`; marker
`// md:PageCollector`.

**Code** — complete and verbatim:

```rust
// md:PageCollector
struct PageCollector<T> {
    limit: usize,
    cursor: Option<(String, Uuid)>,
    heap: std::collections::BinaryHeap<KeyedItem<T>>,
}
```

**What it does** — Streaming replacement for collect-everything-then-paginate:
retains only the `limit + 1` smallest keys past the cursor in a max-heap, so
building one page holds O(page) items instead of the whole store; the `+1`
overflow slot is how it learns whether a next page exists. Cursor semantics
and the produced token match `paginate` exactly.

**Used by** — the note listing methods.

---

## impl PageCollector

**Identification** — inherent impl; marker `// md:impl PageCollector`. Three
methods.

**Code** — container: members documented as sub-blocks below: fn new, fn push, fn into_page.

### fn new

**Identification** — marker `// md:impl PageCollector > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl PageCollector > fn new
    fn new(limit: usize, token: Option<&str>) -> Self {
        let cursor = token
            .filter(|t| !t.is_empty())
            .and_then(|t| t.split_once('|'))
            .and_then(|(ts, id)| Uuid::parse_str(id).ok().map(|id| (ts.to_string(), id)));
        Self {
            limit,
            cursor,
            heap: std::collections::BinaryHeap::with_capacity(limit + 2),
        }
    }
```

**What it does** — Parses the `"<ts>|<uuid>"` cursor (`None`/empty/malformed
→ start at the beginning).

### fn push

**Identification** — marker `// md:impl PageCollector > fn push`.

**Code** — complete and verbatim:

```rust
    // md:impl PageCollector > fn push
    fn push(&mut self, key: (String, Uuid), item: T) {
        if let Some(cursor) = &self.cursor {
            if (key.0.as_str(), key.1) <= (cursor.0.as_str(), cursor.1) {
                return;
            }
        }
        if self.heap.len() <= self.limit {
            self.heap.push(KeyedItem { key, item });
        } else if let Some(top) = self.heap.peek() {
            if key < top.key {
                self.heap.pop();
                self.heap.push(KeyedItem { key, item });
            }
        }
    }
```

**What it does** — Offers one candidate: keys at or before the cursor are
skipped (the same predicate as `paginate`'s partition point); the rest compete
for the retained slots (heap eviction of the largest).

### fn into_page

**Identification** — marker `// md:impl PageCollector > fn into_page`.

**Code** — complete and verbatim:

```rust
    // md:impl PageCollector > fn into_page
    fn into_page(self) -> (Vec<T>, Option<String>) {
        let mut items = self.heap.into_sorted_vec();
        let has_more = items.len() > self.limit;
        items.truncate(self.limit);
        let next_token = if has_more {
            items
                .last()
                .map(|last| format!("{}|{}", last.key.0, last.key.1))
        } else {
            None
        };
        (items.into_iter().map(|k| k.item).collect(), next_token)
    }
```

**What it does** — The retained items in ascending key order, trimmed to
`limit`, with a next-cursor when the overflow slot proved more exist.

---

## fn paginate

**Identification** —
`fn paginate<T, F>(items, limit, token, key_fn) -> (Vec<T>, Option<String>)`;
marker `// md:fn paginate`.

**Code** — complete and verbatim:

```rust
// md:fn paginate
fn paginate<T, F>(
    items: Vec<T>,
    limit: usize,
    token: Option<&str>,
    key_fn: F,
) -> (Vec<T>, Option<String>)
where
    F: Fn(&T) -> (String, Uuid),
{
    let start = match token.filter(|t| !t.is_empty()) {
        Some(cursor) => {
            if let Some((ts, id_str)) = cursor.split_once('|') {
                if let Ok(cursor_id) = Uuid::parse_str(id_str) {
                    items.partition_point(|item| {
                        let (item_ts, item_id) = key_fn(item);
                        item_ts.as_str() < ts || (item_ts.as_str() == ts && item_id <= cursor_id)
                    })
                } else {
                    0
                }
            } else {
                0
            }
        }
        None => 0,
    };

    let remaining: Vec<T> = items.into_iter().skip(start).collect();
    let has_more = remaining.len() > limit;
    let page: Vec<T> = remaining.into_iter().take(limit).collect();

    let next_token = if has_more {
        page.last().map(|last| {
            let (ts, id) = key_fn(last);
            format!("{ts}|{id}")
        })
    } else {
        None
    };

    (page, next_token)
}
```

**What it does** — Cursor pagination over an already-sorted vec: partition
past the `"<ts>|<uuid>"` cursor (strictly after the cursor pair), take
`limit`, emit a next token from the page's last item when more remain.

**Used by** — the notebook/tag/resource listings (which sort small collected
vecs).

---

## impl NoteRepository for FsBackend

**Identification** — marker `// md:impl NoteRepository for FsBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_note, fn read_note, fn update_note, fn delete_note, fn list_notes, fn list_notes_in_notebook, fn list_starred_notes, fn notebook_sort_profile.

**What it does** — the note surface over the log pipeline.

### fn create_note

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn create_note`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for FsBackend > fn create_note
    async fn create_note(&self, note: Note) -> Result<Note, StorageError> {
        let merged = self.append_note_op(note.id, NoteOp::Upsert(note)).await?;
        tracing::info!(id = %merged.id, "Note created");
        Ok(merged)
    }
```

**What it does** — `append_note_op(Upsert(note))` — a create is just the first
op.

### fn read_note

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn read_note`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for FsBackend > fn read_note
    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        self.merge_note(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))
    }
```

**What it does** — Live merge (`merge_note`) — always current, even right
after Syncthing delivers a peer log — and never writes the projection back.
`NotFound` when nothing merges.

### fn update_note

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn update_note`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for FsBackend > fn update_note
    async fn update_note(&self, note: Note) -> Result<Note, StorageError> {
        if self.read_note_logs(note.id).await?.is_empty() {
            return Err(StorageError::NotFound(note.id.to_string()));
        }
        let merged = self.append_note_op(note.id, NoteOp::Upsert(note)).await?;
        tracing::info!(id = %merged.id, "Note updated");
        Ok(merged)
    }
```

**What it does** — `NotFound` when the note has no logs at all, else
`append_note_op(Upsert)`.

### fn delete_note

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn delete_note`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for FsBackend > fn delete_note
    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        if self.read_note_logs(id).await?.is_empty() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        self.append_note_op(id, NoteOp::Tombstone { deleted_at: now() })
            .await?;
        tracing::info!(%id, "Note deleted");
        Ok(())
    }
```

**What it does** — `NotFound` without logs, else
`append_note_op(Tombstone { deleted_at: now })`.

### fn list_notes

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn list_notes`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for FsBackend > fn list_notes
    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let (ids, next) = self
            .with_note_index(|idx| {
                let mut collector = PageCollector::new(limit, page_token.as_deref());
                for (id, entry) in &idx.entries {
                    collector.push((entry.created_at.to_sortable_rfc3339(), *id), *id);
                }
                collector.into_page()
            })
            .await?;
        Ok((self.materialize_page(ids).await?, next))
    }
```

**What it does** — Select + paginate ids from the in-memory index with a
`PageCollector` keyed by `(created_at, id)`, then `materialize_page` only that
page.

### fn list_notes_in_notebook

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn list_notes_in_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for FsBackend > fn list_notes_in_notebook
    async fn list_notes_in_notebook(
        &self,
        notebook_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let (ids, next) = self
            .with_note_index(|idx| {
                let mut collector = PageCollector::new(limit, page_token.as_deref());
                for (id, entry) in &idx.entries {
                    if entry.notebook_id == notebook_id {
                        collector.push((format!("{:010}", entry.effective_sort_key), *id), *id);
                    }
                }
                collector.into_page()
            })
            .await?;
        Ok((self.materialize_page(ids).await?, next))
    }
```

**What it does** — Same, filtered to the notebook, keyed by the effective sort
key **zero-padded to 10 digits** so lexicographic heap order is numeric.

### fn list_starred_notes

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn list_starred_notes`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for FsBackend > fn list_starred_notes
    async fn list_starred_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let (ids, next) = self
            .with_note_index(|idx| {
                let mut collector = PageCollector::new(limit, page_token.as_deref());
                for (id, entry) in &idx.entries {
                    if entry.is_starred {
                        collector.push((entry.created_at.to_sortable_rfc3339(), *id), *id);
                    }
                }
                collector.into_page()
            })
            .await?;
        Ok((self.materialize_page(ids).await?, next))
    }
```

**What it does** — Same, filtered to `is_starred`, keyed by
`(created_at, id)`.

### fn notebook_sort_profile

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn notebook_sort_profile`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for FsBackend > fn notebook_sort_profile
    async fn notebook_sort_profile(
        &self,
        notebook_id: Uuid,
    ) -> Result<NotebookSortProfile, StorageError> {
        let keys = self
            .with_note_index(|idx| {
                idx.entries
                    .values()
                    .filter(|e| e.notebook_id == notebook_id)
                    .map(|e| e.effective_sort_key)
                    .collect::<Vec<u32>>()
            })
            .await?;
        Ok(NotebookSortProfile::from_effective_keys(keys))
    }
```

**What it does** — The notebook's effective keys straight from the index into
`NotebookSortProfile::from_effective_keys`.

---

## impl NotebookRepository for FsBackend

**Identification** — marker `// md:impl NotebookRepository for FsBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_notebook, fn read_notebook, fn update_notebook, fn delete_notebook, fn list_notebooks.

**What it does** — sidecar CRUD + global-log journaling.

### fn create_notebook

**Identification** — marker
`// md:impl NotebookRepository for FsBackend > fn create_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl NotebookRepository for FsBackend > fn create_notebook
    async fn create_notebook(&self, mut notebook: Notebook) -> Result<Notebook, StorageError> {
        let path = self.notebook_path(notebook.id);
        notebook.vv = self.next_sidecar_vv(&path).await?;
        notebook.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &notebook).await?;
        self.append_log(
            "notebook",
            notebook.id,
            "create",
            serde_json::to_value(&notebook)?,
        )
        .await?;
        tracing::info!(id = %notebook.id, "Notebook created");
        Ok(notebook)
    }
```

**What it does** — Stamp vv/writer (`next_sidecar_vv`), write the sidecar,
append a `"create"` log entry with the full record.

### fn read_notebook

**Identification** — marker
`// md:impl NotebookRepository for FsBackend > fn read_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl NotebookRepository for FsBackend > fn read_notebook
    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        self.read_sidecar(&self.notebook_path(id), id).await
    }
```

**What it does** — Sidecar read (`NotFound`/`CorruptedData` semantics).

### fn update_notebook

**Identification** — marker
`// md:impl NotebookRepository for FsBackend > fn update_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl NotebookRepository for FsBackend > fn update_notebook
    async fn update_notebook(&self, mut notebook: Notebook) -> Result<Notebook, StorageError> {
        let path = self.notebook_path(notebook.id);
        if !path.exists() {
            return Err(StorageError::NotFound(notebook.id.to_string()));
        }
        notebook.vv = self.next_sidecar_vv(&path).await?;
        notebook.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &notebook).await?;
        self.append_log(
            "notebook",
            notebook.id,
            "update",
            serde_json::to_value(&notebook)?,
        )
        .await?;
        tracing::info!(id = %notebook.id, "Notebook updated");
        Ok(notebook)
    }
```

**What it does** — `NotFound` when the sidecar doesn't exist, else stamp +
write + `"update"` entry.

### fn delete_notebook

**Identification** — marker
`// md:impl NotebookRepository for FsBackend > fn delete_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl NotebookRepository for FsBackend > fn delete_notebook
    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        let path = self.notebook_path(id);
        let mut nb: Notebook = self.read_sidecar(&path, id).await?;
        let ts = now();
        nb.deleted_at = Some(ts);
        nb.updated_at = ts;
        note_log::increment(&mut nb.vv, &self.device_id);
        nb.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &nb).await?;
        self.append_log(
            "notebook",
            id,
            "delete",
            fs_tombstone_value(ts, &nb.vv, &nb.last_writer),
        )
        .await?;
        tracing::info!(%id, "Notebook deleted");
        Ok(())
    }
```

**What it does** — Soft delete: read, set `deleted_at`/`updated_at`, bump vv,
write back, `"delete"` entry with `fs_tombstone_value`.

### fn list_notebooks

**Identification** — marker
`// md:impl NotebookRepository for FsBackend > fn list_notebooks`.

**Code** — complete and verbatim:

```rust
    // md:impl NotebookRepository for FsBackend > fn list_notebooks
    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let mut notebooks = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("notebooks")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(stem) = fname.strip_suffix(".msgpack") {
                if let Ok(id) = Uuid::parse_str(stem) {
                    match self.read_sidecar::<Notebook>(&entry.path(), id).await {
                        Ok(nb) if nb.deleted_at.is_none() => notebooks.push(nb),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("Could not load notebook {id}: {e}"),
                    }
                }
            }
        }
        notebooks.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(paginate(notebooks, limit, page_token.as_deref(), |nb| {
            (nb.created_at.to_sortable_rfc3339(), nb.id)
        }))
    }
```

**What it does** — Scan the sidecar directory, keep live decodable notebooks
(failures warned and skipped), sort `(created_at, id)`, `paginate`.

---

## impl TagRepository for FsBackend

**Identification** — marker `// md:impl TagRepository for FsBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_tag, fn read_tag, fn update_tag, fn delete_tag, fn list_tags, fn add_note_tag, fn remove_note_tag, fn list_note_tags.

**What it does** — mirrors the notebook pattern, plus versioned associations.

### fn create_tag

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn create_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for FsBackend > fn create_tag
    async fn create_tag(&self, mut tag: Tag) -> Result<Tag, StorageError> {
        let path = self.tag_path(tag.id);
        tag.vv = self.next_sidecar_vv(&path).await?;
        tag.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &tag).await?;
        self.append_log("tag", tag.id, "create", serde_json::to_value(&tag)?)
            .await?;
        tracing::info!(id = %tag.id, "Tag created");
        Ok(tag)
    }
```

**What it does** — Stamp + sidecar + `"create"` entry.

### fn read_tag

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn read_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for FsBackend > fn read_tag
    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        self.read_sidecar(&self.tag_path(id), id).await
    }
```

**What it does** — Sidecar read.

### fn update_tag

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn update_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for FsBackend > fn update_tag
    async fn update_tag(&self, mut tag: Tag) -> Result<Tag, StorageError> {
        let path = self.tag_path(tag.id);
        if !path.exists() {
            return Err(StorageError::NotFound(tag.id.to_string()));
        }
        tag.vv = self.next_sidecar_vv(&path).await?;
        tag.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &tag).await?;
        self.append_log("tag", tag.id, "update", serde_json::to_value(&tag)?)
            .await?;
        tracing::info!(id = %tag.id, "Tag updated");
        Ok(tag)
    }
```

**What it does** — Existence check + stamp + sidecar + `"update"` entry.

### fn delete_tag

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn delete_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for FsBackend > fn delete_tag
    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        let path = self.tag_path(id);
        let mut tag: Tag = self.read_sidecar(&path, id).await?;
        let ts = now();
        tag.deleted_at = Some(ts);
        tag.updated_at = ts;
        note_log::increment(&mut tag.vv, &self.device_id);
        tag.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &tag).await?;
        self.append_log(
            "tag",
            id,
            "delete",
            fs_tombstone_value(ts, &tag.vv, &tag.last_writer),
        )
        .await?;
        tracing::info!(%id, "Tag deleted");
        Ok(())
    }
```

**What it does** — Soft delete + tombstone entry.

### fn list_tags

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn list_tags`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for FsBackend > fn list_tags
    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let mut tags = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("tags")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(stem) = fname.strip_suffix(".msgpack") {
                if let Ok(id) = Uuid::parse_str(stem) {
                    match self.read_sidecar::<Tag>(&entry.path(), id).await {
                        Ok(t) if t.deleted_at.is_none() => tags.push(t),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("Could not load tag {id}: {e}"),
                    }
                }
            }
        }
        tags.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(paginate(tags, limit, page_token.as_deref(), |t| {
            (t.created_at.to_sortable_rfc3339(), t.id)
        }))
    }
```

**What it does** — Scan, filter live, sort, `paginate`.

### fn add_note_tag

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn add_note_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for FsBackend > fn add_note_tag
    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        match self.merge_note(note_tag.note_id).await? {
            Some(n) if n.deleted_at.is_none() => {}
            _ => return Err(StorageError::NotFound(note_tag.note_id.to_string())),
        }
        let tag: Tag = self
            .read_sidecar(&self.tag_path(note_tag.tag_id), note_tag.tag_id)
            .await?;
        if tag.deleted_at.is_some() {
            return Err(StorageError::NotFound(note_tag.tag_id.to_string()));
        }
        let path = self.note_tag_path(note_tag.note_id, note_tag.tag_id);
        let vv = self.next_assoc_vv(&path).await?;
        let ts = now();
        let state = NoteTagState {
            updated_at: ts,
            deleted_at: None,
            vv: vv.clone(),
            last_writer: self.device_id.clone(),
        };
        self.write_assoc_state(note_tag.note_id, note_tag.tag_id, &state)
            .await?;
        self.append_log(
            "note_tag",
            note_tag.note_id,
            "add",
            fs_assoc_value(note_tag.tag_id, ts, &vv, &self.device_id),
        )
        .await?;
        Ok(())
    }
```

**What it does** — Both ends must exist and be live (merged note; live tag
sidecar) — `NotFound` otherwise; the API must not create dangling
associations (`apply_change` deliberately skips the check: sync delivery
order is not guaranteed). Then write the association's **present** state
(`deleted_at: None`, fresh vv) and an `"add"` log entry. Idempotent.

### fn remove_note_tag

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn remove_note_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for FsBackend > fn remove_note_tag
    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        let path = self.note_tag_path(note_id, tag_id);
        let vv = self.next_assoc_vv(&path).await?;
        let ts = now();
        let state = NoteTagState {
            updated_at: ts,
            deleted_at: Some(ts),
            vv: vv.clone(),
            last_writer: self.device_id.clone(),
        };
        self.write_assoc_state(note_id, tag_id, &state).await?;
        self.append_log(
            "note_tag",
            note_id,
            "remove",
            fs_assoc_value(tag_id, ts, &vv, &self.device_id),
        )
        .await?;
        Ok(())
    }
```

**What it does** — Write the **tombstone** state (kept so it can beat a
concurrent add) and a `"remove"` entry. Idempotent.

### fn list_note_tags

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn list_note_tags`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for FsBackend > fn list_note_tags
    async fn list_note_tags(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let dir_path = self.note_tag_dir(note_id);
        if !dir_path.exists() {
            return Ok((vec![], None));
        }
        let mut tags = Vec::new();
        let mut dir = tokio::fs::read_dir(&dir_path).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name().to_string_lossy().to_string();
            let Ok(tag_id) = Uuid::parse_str(&fname) else {
                continue;
            };
            match self.read_assoc_state(&entry.path()).await {
                Ok(Some(s)) if s.deleted_at.is_some() => continue,
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Could not read note_tag {note_id}/{tag_id}: {e}");
                    continue;
                }
            }
            match self.read_tag(tag_id).await {
                Ok(t) if t.deleted_at.is_none() => tags.push(t),
                Ok(_) => {}
                Err(e) => tracing::warn!("Could not load tag {tag_id} for note {note_id}: {e}"),
            }
        }
        tags.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(paginate(tags, limit, page_token.as_deref(), |t| {
            (t.created_at.to_sortable_rfc3339(), t.id)
        }))
    }
```

**What it does** — Walk `note_tags/{note}`, skip tombstoned associations and
deleted/unreadable tags, sort, `paginate`.

---

## impl ResourceRepository for FsBackend

**Identification** — marker `// md:impl ResourceRepository for FsBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_resource, fn read_resource, fn delete_resource, fn list_resources, fn purge_deleted_resources.

**What it does** — resources as `meta.msgpack` + `data`.

### fn create_resource

**Identification** — marker
`// md:impl ResourceRepository for FsBackend > fn create_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for FsBackend > fn create_resource
    async fn create_resource(
        &self,
        mut resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let dir = self.resource_dir(resource.id);
        tokio::fs::create_dir_all(&dir).await?;
        resource.vv = self.next_resource_vv(resource.id).await?;
        resource.last_writer = self.device_id.clone();
        tokio::fs::write(self.resource_data_path(resource.id), &data).await?;
        self.write_sidecar(&self.resource_meta_path(resource.id), &resource)
            .await?;
        self.append_log(
            "resource",
            resource.id,
            "create",
            serde_json::to_value(&resource)?,
        )
        .await?;
        tracing::info!(id = %resource.id, "Resource created");
        Ok(resource)
    }
```

**What it does** — Stamp vv/writer, write the **data file first, metadata
last**: `read_resource` treats `meta.msgpack` as proof of existence, so the
metadata write is the commit marker — a crash between the two leaves an
orphan data file (harmless, overwritten on retry) rather than metadata
pointing at a missing payload. Then a `"create"` log entry (metadata only).

### fn read_resource

**Identification** — marker
`// md:impl ResourceRepository for FsBackend > fn read_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for FsBackend > fn read_resource
    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let meta_path = self.resource_meta_path(id);
        if !meta_path.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let resource: Resource = self.read_sidecar(&meta_path, id).await?;
        if resource.deleted_at.is_some() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let data = tokio::fs::read(self.resource_data_path(id)).await?;
        Ok((resource, data))
    }
```

**What it does** — `NotFound` without metadata or when tombstoned (the
tombstone is kept for sync); else metadata + data bytes.

### fn delete_resource

**Identification** — marker
`// md:impl ResourceRepository for FsBackend > fn delete_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for FsBackend > fn delete_resource
    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        let meta_path = self.resource_meta_path(id);
        let mut resource: Resource = self.read_sidecar(&meta_path, id).await?;
        let ts = now();
        resource.deleted_at = Some(ts);
        note_log::increment(&mut resource.vv, &self.device_id);
        resource.last_writer = self.device_id.clone();
        self.write_sidecar(&meta_path, &resource).await?;
        self.append_log(
            "resource",
            id,
            "delete",
            fs_tombstone_value(ts, &resource.vv, &resource.last_writer),
        )
        .await?;
        tracing::info!(%id, "Resource deleted");
        Ok(())
    }
```

**What it does** — Soft delete: tombstone + bumped vv in the metadata; the
payload is retained; `"delete"` entry.

### fn list_resources

**Identification** — marker
`// md:impl ResourceRepository for FsBackend > fn list_resources`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for FsBackend > fn list_resources
    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let mut resources = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("resources")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let id_str = entry.file_name().to_string_lossy().to_string();
            if let Ok(id) = Uuid::parse_str(&id_str) {
                let meta_path = self.resource_meta_path(id);
                match self.read_sidecar::<Resource>(&meta_path, id).await {
                    Ok(r) if r.deleted_at.is_none() => resources.push(r),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("Could not load resource {id}: {e}"),
                }
            }
        }
        resources.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(paginate(resources, limit, page_token.as_deref(), |r| {
            (r.created_at.to_sortable_rfc3339(), r.id)
        }))
    }
```

**What it does** — Scan resource dirs, keep live decodable metadata, sort,
`paginate` (no payloads).

### fn purge_deleted_resources

**Identification** — marker
`// md:impl ResourceRepository for FsBackend > fn purge_deleted_resources`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for FsBackend > fn purge_deleted_resources
    async fn purge_deleted_resources(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, StorageError> {
        let mut purged = 0u64;
        let mut dir = match tokio::fs::read_dir(self.root.join("resources")).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = dir.next_entry().await? {
            let Ok(id) = Uuid::parse_str(&entry.file_name().to_string_lossy()) else {
                continue;
            };
            let meta = match self.read_resource_meta(id).await {
                Ok(Some(meta)) => meta,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!("Skipping resource {id} during purge (unreadable meta): {e}");
                    continue;
                }
            };
            let Some(deleted_at) = meta.deleted_at else {
                continue;
            };
            if deleted_at >= older_than {
                continue;
            }
            match tokio::fs::remove_file(self.resource_data_path(id)).await {
                Ok(()) => purged += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        if purged > 0 {
            tracing::info!(purged, "Reclaimed payloads of soft-deleted resources");
        }
        Ok(purged)
    }
```

**What it does** — Removes the `data` file of every resource whose readable
tombstone is older than the cutoff (live resources, crashed-create orphans,
and unreadable metas conservatively keep their payloads). The removal
replicates as a deletion through Syncthing — safe: every peer converges on
the same tombstone, and a late concurrent revive rewrites the file. Tombstone
metadata always survives.

---

## impl SyncBackend for FsBackend

**Identification** — marker `// md:impl SyncBackend for FsBackend`; per-method
markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn get_changes_since, fn apply_change, fn get_last_sync_time, fn update_sync_time, fn send_changes, fn receive_changes, fn get_device_id, fn prune_change_journal.

**What it does** — the passive-replication sync surface.

### fn get_changes_since

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn get_changes_since`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncBackend for FsBackend > fn get_changes_since
    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError> {
        let entries = self.read_other_logs_since(since).await?;
        let changes = entries
            .into_iter()
            .filter_map(|e| {
                let result = log_entry_to_change(e);
                if result.is_none() {
                    tracing::warn!("Skipped unrecognised log entry");
                }
                result
            })
            .collect();
        Ok(changes)
    }
```

**What it does** — Foreign-log entries after `since`
(`read_other_logs_since`) mapped through `log_entry_to_change`; unrecognised
entries warned and skipped. **Note**: only the global journal — notes are not
emitted here (they travel per-note logs), which is why `migrate.rs` uses
typed copies instead of a raw change bridge.

### fn apply_change

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn apply_change`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncBackend for FsBackend > fn apply_change
    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        match change {
            Change::NoteCreate { note } | Change::NoteUpdate { note } => {
                self.materialize(note.id).await?;
                tracing::debug!(id = %note.id, "Materialized remote note change");
            }
            Change::NoteDelete { id, .. } => {
                self.materialize(id).await?;
                tracing::debug!(%id, "Materialized remote note delete");
            }
            Change::NotebookCreate { notebook } | Change::NotebookUpdate { notebook } => {
                let path = self.notebook_path(notebook.id);
                if self
                    .sidecar_incoming_wins(
                        &path,
                        &notebook.vv,
                        notebook.updated_at,
                        &notebook.last_writer,
                    )
                    .await?
                {
                    self.write_sidecar(&path, &notebook).await?;
                    tracing::debug!(id = %notebook.id, "Applied remote notebook change");
                } else {
                    tracing::debug!(id = %notebook.id, "Skipped stale remote notebook change");
                }
            }
            Change::NotebookDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                let path = self.notebook_path(id);
                if self
                    .sidecar_incoming_wins(&path, &vv, deleted_at, &last_writer)
                    .await?
                {
                    let mut nb: Notebook = match self.read_sidecar(&path, id).await {
                        Ok(nb) => nb,
                        Err(StorageError::NotFound(_)) => Notebook {
                            id,
                            title: String::new(),
                            created_at: deleted_at,
                            updated_at: deleted_at,
                            deleted_at: None,
                            alias: None,
                            vv: VersionVector::new(),
                            last_writer: String::new(),
                        },
                        Err(e) => return Err(e),
                    };
                    nb.deleted_at = Some(deleted_at);
                    nb.updated_at = deleted_at;
                    nb.vv = vv;
                    nb.last_writer = last_writer;
                    self.write_sidecar(&path, &nb).await?;
                    tracing::debug!(%id, "Applied remote notebook delete");
                }
            }
            Change::TagCreate { tag } | Change::TagUpdate { tag } => {
                let path = self.tag_path(tag.id);
                if self
                    .sidecar_incoming_wins(&path, &tag.vv, tag.updated_at, &tag.last_writer)
                    .await?
                {
                    self.write_sidecar(&path, &tag).await?;
                    tracing::debug!(id = %tag.id, "Applied remote tag change");
                } else {
                    tracing::debug!(id = %tag.id, "Skipped stale remote tag change");
                }
            }
            Change::TagDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                let path = self.tag_path(id);
                if self
                    .sidecar_incoming_wins(&path, &vv, deleted_at, &last_writer)
                    .await?
                {
                    let mut t: Tag = match self.read_sidecar(&path, id).await {
                        Ok(t) => t,
                        Err(StorageError::NotFound(_)) => Tag {
                            id,
                            title: String::new(),
                            created_at: deleted_at,
                            updated_at: deleted_at,
                            deleted_at: None,
                            vv: VersionVector::new(),
                            last_writer: String::new(),
                        },
                        Err(e) => return Err(e),
                    };
                    t.deleted_at = Some(deleted_at);
                    t.updated_at = deleted_at;
                    t.vv = vv;
                    t.last_writer = last_writer;
                    self.write_sidecar(&path, &t).await?;
                    tracing::debug!(%id, "Applied remote tag delete");
                }
            }
            Change::NoteTagAdd {
                note_id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            } => {
                let path = self.note_tag_path(note_id, tag_id);
                if self
                    .assoc_incoming_wins(&path, &vv, updated_at, &last_writer)
                    .await?
                {
                    let state = NoteTagState {
                        updated_at,
                        deleted_at: None,
                        vv,
                        last_writer,
                    };
                    self.write_assoc_state(note_id, tag_id, &state).await?;
                    tracing::debug!(%note_id, %tag_id, "Applied remote note_tag add");
                }
            }
            Change::NoteTagRemove {
                note_id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            } => {
                let path = self.note_tag_path(note_id, tag_id);
                if self
                    .assoc_incoming_wins(&path, &vv, updated_at, &last_writer)
                    .await?
                {
                    let state = NoteTagState {
                        updated_at,
                        deleted_at: Some(updated_at),
                        vv,
                        last_writer,
                    };
                    self.write_assoc_state(note_id, tag_id, &state).await?;
                    tracing::debug!(%note_id, %tag_id, "Applied remote note_tag remove");
                }
            }
            Change::ResourceCreate { resource, data } => {
                let ts = resource.deleted_at.unwrap_or(resource.created_at);
                if self
                    .resource_incoming_wins(resource.id, &resource.vv, ts, &resource.last_writer)
                    .await?
                {
                    tokio::fs::create_dir_all(self.resource_dir(resource.id)).await?;
                    self.write_sidecar(&self.resource_meta_path(resource.id), &resource)
                        .await?;
                    if let Some(bytes) = data {
                        tokio::fs::write(self.resource_data_path(resource.id), &bytes).await?;
                    }
                    tracing::debug!(id = %resource.id, "Applied remote resource create");
                }
            }
            Change::ResourceDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                if self
                    .resource_incoming_wins(id, &vv, deleted_at, &last_writer)
                    .await?
                {
                    let mut resource = match self.read_resource_meta(id).await? {
                        Some(r) => r,
                        None => Resource {
                            id,
                            title: String::new(),
                            mime_type: String::new(),
                            file_name: String::new(),
                            size: 0,
                            created_at: deleted_at,
                            deleted_at: None,
                            vv: VersionVector::new(),
                            last_writer: String::new(),
                        },
                    };
                    resource.deleted_at = Some(deleted_at);
                    resource.vv = vv;
                    resource.last_writer = last_writer;
                    tokio::fs::create_dir_all(self.resource_dir(id)).await?;
                    self.write_sidecar(&self.resource_meta_path(id), &resource)
                        .await?;
                    tracing::debug!(%id, "Applied remote resource delete");
                }
            }
        }
        Ok(())
    }
```

**What it does** — Per variant:

- **Notes (create/update/delete)** — just `materialize(id)`: conflict
  resolution lives entirely in the per-device logs Syncthing already
  replicated, so applying is a re-materialisation — it never appends to this
  device's log (no spurious local edit) and is idempotent.
- **Notebook/tag create/update** — `sidecar_incoming_wins` gate, then write
  the sidecar (stale changes logged and skipped).
- **Notebook/tag delete** — gate, then tombstone the existing sidecar or —
  unknown locally — write a **minimal tombstone** so a later stale
  create/update loses in `resolve` instead of resurrecting it (issue #71).
- **NoteTagAdd/Remove** — `assoc_incoming_wins` gate, then write the
  present/tombstone state.
- **ResourceCreate** — gate; write metadata, and the payload only when the
  change carries one (`data = Some` from a DbBackend peer; `None` from an
  FsBackend peer whose data file Syncthing replicates independently).
- **ResourceDelete** — gate; tombstone existing metadata or write a minimal
  tombstone (issue #71); the blob is retained.

### fn get_last_sync_time

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn get_last_sync_time`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncBackend for FsBackend > fn get_last_sync_time
    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
        let path = self.root.join(".keeplin").join("sync_state.msgpack");
        match self.read_sidecar::<SyncState>(&path, Uuid::nil()).await {
            Ok(state) => Ok(state.last_sync),
            Err(StorageError::NotFound(_)) => {
                Ok(DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default())
            }
            Err(e) => Err(e),
        }
    }
```

**What it does** — `.keeplin/sync_state.msgpack`, epoch when absent.

### fn update_sync_time

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn update_sync_time`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncBackend for FsBackend > fn update_sync_time
    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        let state = SyncState { last_sync: ts };
        let path = self.root.join(".keeplin").join("sync_state.msgpack");
        self.write_sidecar(&path, &state).await
    }
```

**What it does** — Atomic sidecar write of the watermark.

### fn send_changes

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn send_changes`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncBackend for FsBackend > fn send_changes
    async fn send_changes(&self, _changes: Vec<Change>) -> Result<(), StorageError> {
        tracing::debug!("Offline mode: changes are replicated passively via the filesystem");
        Ok(())
    }
```

**What it does** — A no-op: Syncthing replicates `logs/` passively. Exists
(and succeeds) because `SyncEngine` calls it in the standard cycle.

### fn receive_changes

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn receive_changes`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncBackend for FsBackend > fn receive_changes
    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        let mut changes: Vec<Change> = self
            .read_new_entries()
            .await?
            .into_iter()
            .filter_map(|e| {
                let result = log_entry_to_change(e);
                if result.is_none() {
                    tracing::warn!("Skipped unrecognised log entry in receive_changes");
                }
                result
            })
            .collect();
        changes.extend(self.collect_advanced_notes().await?);
        Ok(changes)
    }
```

**What it does** — Cursor-advanced foreign-log entries
(notebooks/tags/resources via `read_new_entries` + `log_entry_to_change`)
**plus** `collect_advanced_notes` (notes whose per-note logs advanced — e.g. a
peer's log just arrived via Syncthing — re-materialised and reported as
changes). This is the call that *materialises* replicated peer notes, which is
why `LinkingBackend` invalidates its alias index on any note/notebook change
reported here.

### fn get_device_id

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn get_device_id`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncBackend for FsBackend > fn get_device_id
    async fn get_device_id(&self) -> Result<String, StorageError> {
        Ok(self.device_id.clone())
    }
```

**What it does** — The cached id.

### fn prune_change_journal

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn prune_change_journal`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncBackend for FsBackend > fn prune_change_journal
    async fn prune_change_journal(&self, _older_than: DateTime<Utc>) -> Result<u64, StorageError> {
        Ok(0)
    }
```

**What it does** — Always `Ok(0)`: peers track entries by byte offset, so
time-based deletion could permanently lose changes for a lagging peer;
epoch-snapshot compaction does the bounding instead.

---

## impl FsBackend (global history)

**Identification** — the second inherent impl; marker
`// md:impl FsBackend (global history)`. One method.

**Code** — container: members documented as sub-blocks below: fn read_all_global_entries.

### fn read_all_global_entries

**Identification** — marker
`// md:impl FsBackend (global history) > fn read_all_global_entries`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend (global history) > fn read_all_global_entries
    async fn read_all_global_entries(&self) -> Result<Vec<(String, LogEntry)>, StorageError> {
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

## mod tests

**Identification** — `#[cfg(test)] mod tests`; marker `// md:mod tests`.
Twelve tests.

**Code** — container: members documented as sub-blocks below: fn concurrent_same_note_updates_keep_every_log_entry, fn read_does_not_rewrite_projection, fn list_notes_pages_match_full_walk, fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones, fn failed_atomic_write_cleans_up_its_temp_file, fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it, fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair, fn detects_syncthing_conflict_copies_without_removing_them, fn purge_reclaims_old_tombstoned_payloads_only, fn fresh_store_is_stamped_current_version, fn migrates_a_legacy_stamp_and_preserves_data, fn refuses_to_open_a_newer_format.

**What it does** — Pins the concurrency, purity, pagination, hygiene,
corruption-recovery, compaction, purge, and format-version behaviours.

### fn concurrent_same_note_updates_keep_every_log_entry

**Identification** — multi-thread tokio test; marker
`// md:mod tests > fn concurrent_same_note_updates_keep_every_log_entry`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn concurrent_same_note_updates_keep_every_log_entry
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_same_note_updates_keep_every_log_entry() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(FsBackend::new(dir.path()).await.unwrap());
        let note = backend.create_note(Note::new("t", "v0")).await.unwrap();
        let id = note.id;

        let updates = 20usize;
        let mut handles = Vec::new();
        for i in 0..updates {
            let b = Arc::clone(&backend);
            let mut edited = note.clone();
            handles.push(tokio::spawn(async move {
                edited.body = format!("v{i}");
                b.update_note(edited).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let logs = backend.read_note_logs(id).await.unwrap();
        let total: usize = logs.iter().map(|l| l.len()).sum();
        assert_eq!(total, 1 + updates, "create + {updates} updates, none lost");
    }
```

**What it does** — Regression for the lost-update race: 20 concurrent updates
to one note all land in the single-device log (create + 20 entries, none
dropped by a racing rename) — the `note_write_lock` guarantee.

### fn read_does_not_rewrite_projection

**Identification** — tokio test; marker
`// md:mod tests > fn read_does_not_rewrite_projection`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn read_does_not_rewrite_projection
    #[tokio::test]
    async fn read_does_not_rewrite_projection() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FsBackend::new(dir.path()).await.unwrap();
        let note = backend.create_note(Note::new("t", "body")).await.unwrap();
        let meta = backend.note_meta_path(note.id);
        let md = backend.note_md_path(note.id);

        tokio::fs::remove_file(&meta).await.unwrap();
        tokio::fs::remove_file(&md).await.unwrap();

        let read = backend.read_note(note.id).await.unwrap();
        assert_eq!(read.body, "body");
        assert!(!meta.exists(), "read must not rewrite meta.msgpack");
        assert!(!md.exists(), "read must not rewrite note.md");
    }
```

**What it does** — Delete the projection files; `read_note` still answers
from the logs and does **not** recreate `note.md`/`meta.msgpack` (reads are
pure).

### fn list_notes_pages_match_full_walk

**Identification** — tokio test; marker
`// md:mod tests > fn list_notes_pages_match_full_walk`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn list_notes_pages_match_full_walk
    #[tokio::test]
    async fn list_notes_pages_match_full_walk() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FsBackend::new(dir.path()).await.unwrap();
        let total = 23usize;
        let mut created = Vec::new();
        for i in 0..total {
            created.push(
                backend
                    .create_note(Note::new(format!("t{i}"), "b"))
                    .await
                    .unwrap(),
            );
        }
        backend.delete_note(created[5].id).await.unwrap();
        created.remove(5);
        created.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

        let mut walked = Vec::new();
        let mut token = None;
        loop {
            let (page, next) = backend.list_notes(7, token).await.unwrap();
            assert!(page.len() <= 7);
            walked.extend(page);
            match next {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        assert_eq!(
            walked.iter().map(|n| n.id).collect::<Vec<_>>(),
            created.iter().map(|n| n.id).collect::<Vec<_>>(),
            "paged walk must reproduce the full (created_at, id) order"
        );
    }
```

**What it does** — 23 notes (one deleted) walked in pages of 7 reproduce the
full `(created_at, id)` order — the heap `PageCollector` paginates exactly
like sort-then-`paginate`.

### fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones

**Identification** — tokio test; marker
`// md:mod tests > fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones
    #[tokio::test]
    async fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones() {
        let dir = tempfile::tempdir().unwrap();
        let note_id = {
            let be = FsBackend::new(dir.path()).await.unwrap();
            be.create_note(Note::new("t", "b")).await.unwrap().id
        };
        let planted = [
            dir.path()
                .join("notes")
                .join(note_id.to_string())
                .join("meta.tmp"),
            dir.path().join("notebooks").join("junk.tmp"),
            dir.path().join(".keeplin").join("sync_state.tmp"),
            dir.path().join(".keeplin").join("offsets").join("dev.tmp"),
        ];
        for p in &planted {
            std::fs::write(p, b"junk").unwrap();
        }
        let syncthing = dir
            .path()
            .join("notebooks")
            .join(".syncthing.abc.msgpack.tmp");
        std::fs::write(&syncthing, b"in-flight transfer").unwrap();

        let be = FsBackend::new(dir.path()).await.unwrap();
        for p in &planted {
            assert!(!p.exists(), "must be swept: {}", p.display());
        }
        assert!(syncthing.exists(), "Syncthing temp must be left alone");
        assert_eq!(be.read_note(note_id).await.unwrap().body, "b");
    }
```

**What it does** — Planted `*.tmp` files in every managed dir are swept on
startup; a `.syncthing.*.tmp` survives; the store still reads.

### fn failed_atomic_write_cleans_up_its_temp_file

**Identification** — tokio test; marker
`// md:mod tests > fn failed_atomic_write_cleans_up_its_temp_file`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn failed_atomic_write_cleans_up_its_temp_file
    #[tokio::test]
    async fn failed_atomic_write_cleans_up_its_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("blocked");
        std::fs::create_dir(&dest).unwrap();

        assert!(atomic_write(&dest, b"payload").await.is_err());
        assert!(
            !dest.with_extension("tmp").exists(),
            "temp file must be removed after a failed write"
        );
        assert!(dest.is_dir(), "destination must be untouched");
    }
```

**What it does** — A rename-blocked `atomic_write` errors, removes its temp
file, and leaves the destination untouched.

### fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it

**Identification** — tokio test; marker
`// md:mod tests > fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it
    #[tokio::test]
    async fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it() {
        let dir = tempfile::tempdir().unwrap();
        let be = FsBackend::new(dir.path()).await.unwrap();
        let note = be.create_note(Note::new("n", "")).await.unwrap();
        let tag = be.create_tag(Tag::new("t")).await.unwrap();
        be.add_note_tag(NoteTag {
            note_id: note.id,
            tag_id: tag.id,
        })
        .await
        .unwrap();

        let path = dir
            .path()
            .join("note_tags")
            .join(note.id.to_string())
            .join(tag.id.to_string());
        std::fs::write(&path, b"not msgpack at all").unwrap();

        let (tags, _) = be.list_note_tags(note.id, 0, None).await.unwrap();
        assert_eq!(tags.len(), 1, "corrupt state must not hide the association");

        let mut vv = VersionVector::new();
        note_log::increment(&mut vv, "peer");
        be.apply_change(Change::NoteTagRemove {
            note_id: note.id,
            tag_id: tag.id,
            updated_at: now(),
            vv,
            last_writer: "peer".to_string(),
        })
        .await
        .unwrap();
        let (tags, _) = be.list_note_tags(note.id, 0, None).await.unwrap();
        assert!(
            tags.is_empty(),
            "a versioned remove must supersede the corrupt marker"
        );
    }
```

**What it does** — A corrupted association file still lists as attached
(least harm), and a versioned peer remove supersedes the epoch-0 fallback
marker.

### fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair

**Identification** — tokio test; marker
`// md:mod tests > fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair
    #[tokio::test]
    async fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair() {
        let dir = tempfile::tempdir().unwrap();
        let be = FsBackend::new(dir.path()).await.unwrap();
        let nb = be.create_notebook(Notebook::new("kept")).await.unwrap();
        be.create_tag(Tag::new("t")).await.unwrap();
        let log_path = be.device_log_path();
        let before = tokio::fs::read_to_string(&log_path).await.unwrap();

        let sidecar = be.notebook_path(nb.id);
        std::fs::write(&sidecar, b"definitely not msgpack").unwrap();
        {
            let _guard = be.global_log_lock.lock().await;
            be.compact_global_log_locked().await.unwrap();
        }
        let after = tokio::fs::read_to_string(&log_path).await.unwrap();
        assert_eq!(before, after, "journal must not be rewritten while corrupt");
        assert_eq!(
            be.read_own_epoch().await.unwrap(),
            0,
            "no snapshot generation was produced"
        );

        let bytes = rmp_serde::to_vec_named(&nb).unwrap();
        std::fs::write(&sidecar, bytes).unwrap();
        {
            let _guard = be.global_log_lock.lock().await;
            be.compact_global_log_locked().await.unwrap();
        }
        assert_eq!(
            be.read_own_epoch().await.unwrap(),
            1,
            "compaction resumes once the sidecar is readable"
        );
        let snapshot = tokio::fs::read_to_string(&log_path).await.unwrap();
        assert!(
            snapshot.contains(&nb.id.to_string()),
            "the repaired notebook is present in the snapshot"
        );
    }
```

**What it does** — With a corrupted notebook sidecar the journal is not
rewritten and no epoch is produced; after repair, compaction produces epoch 1
containing the notebook.

### fn detects_syncthing_conflict_copies_without_removing_them

**Identification** — tokio test; marker
`// md:mod tests > fn detects_syncthing_conflict_copies_without_removing_them`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn detects_syncthing_conflict_copies_without_removing_them
    #[tokio::test]
    async fn detects_syncthing_conflict_copies_without_removing_them() {
        let dir = tempfile::tempdir().unwrap();
        let note_id = {
            let be = FsBackend::new(dir.path()).await.unwrap();
            be.create_note(Note::new("t", "b")).await.unwrap().id
        };
        let conflicts = [
            dir.path()
                .join(".keeplin")
                .join("device_id.sync-conflict-20260702-120000-AAAAAAA"),
            dir.path()
                .join("notebooks")
                .join("junk.sync-conflict-20260702-120000-BBBBBBB.msgpack"),
            dir.path()
                .join("notes")
                .join(note_id.to_string())
                .join("log.dev.sync-conflict-20260702-120000-CCCCCCC.msgpack"),
        ];
        for p in &conflicts {
            std::fs::write(p, b"conflict copy").unwrap();
        }

        let found = FsBackend::scan_sync_conflicts(dir.path()).await;
        assert_eq!(
            found.len(),
            conflicts.len(),
            "all copies detected: {found:?}"
        );

        let be = FsBackend::new(dir.path()).await.unwrap();
        for p in &conflicts {
            assert!(
                p.exists(),
                "conflict copy must be preserved: {}",
                p.display()
            );
        }
        assert_eq!(be.read_note(note_id).await.unwrap().body, "b");
    }
```

**What it does** — Conflict copies in `.keeplin/`, `notebooks/`, and a note
dir are all detected, never deleted, and never block startup.

### fn purge_reclaims_old_tombstoned_payloads_only

**Identification** — tokio test; marker
`// md:mod tests > fn purge_reclaims_old_tombstoned_payloads_only`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn purge_reclaims_old_tombstoned_payloads_only
    #[tokio::test]
    async fn purge_reclaims_old_tombstoned_payloads_only() {
        let dir = tempfile::tempdir().unwrap();
        let be = FsBackend::new(dir.path()).await.unwrap();

        let dead = Resource::new("dead", "text/plain", "d.txt", 4);
        let dead_id = dead.id;
        be.create_resource(dead, b"dead".to_vec()).await.unwrap();
        be.delete_resource(dead_id).await.unwrap();

        let live = Resource::new("live", "text/plain", "l.txt", 4);
        let live_id = live.id;
        be.create_resource(live, b"live".to_vec()).await.unwrap();

        let epoch = DateTime::<Utc>::from_timestamp(0, 0).unwrap();
        assert_eq!(be.purge_deleted_resources(epoch).await.unwrap(), 0);
        assert!(be.resource_data_path(dead_id).exists());

        assert_eq!(be.purge_deleted_resources(now()).await.unwrap(), 1);
        assert!(!be.resource_data_path(dead_id).exists(), "dead bytes freed");
        assert!(
            be.resource_meta_path(dead_id).exists(),
            "tombstone metadata must survive the purge"
        );
        assert!(matches!(
            be.read_resource(dead_id).await,
            Err(StorageError::NotFound(_))
        ));
        let (_, bytes) = be.read_resource(live_id).await.unwrap();
        assert_eq!(bytes, b"live", "live resources are untouched");

        assert_eq!(be.purge_deleted_resources(now()).await.unwrap(), 0);
        let mut revived = Resource::new("revived", "text/plain", "r.txt", 3);
        revived.id = dead_id;
        be.create_resource(revived, b"new".to_vec()).await.unwrap();
        let (_, bytes) = be.read_resource(dead_id).await.unwrap();
        assert_eq!(bytes, b"new");
    }
```

**What it does** — A pre-tombstone cutoff purges nothing; a later cutoff
frees exactly the dead payload while the tombstone metadata survives and live
resources are untouched; purge is idempotent and the id can be recreated.

### fn fresh_store_is_stamped_current_version

**Identification** — tokio test; marker
`// md:mod tests > fn fresh_store_is_stamped_current_version`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn fresh_store_is_stamped_current_version
    #[tokio::test]
    async fn fresh_store_is_stamped_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let be = FsBackend::new(dir.path()).await.unwrap();
        let stamp = tokio::fs::read_to_string(be.format_version_path())
            .await
            .unwrap();
        assert_eq!(
            stamp.trim().parse::<u32>().unwrap(),
            FsBackend::FORMAT_VERSION,
            "a brand-new store starts stamped at the current format version"
        );
    }
```

**What it does** — A brand-new store starts stamped `FORMAT_VERSION`.

### fn migrates_a_legacy_stamp_and_preserves_data

**Identification** — tokio test; marker
`// md:mod tests > fn migrates_a_legacy_stamp_and_preserves_data`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn migrates_a_legacy_stamp_and_preserves_data
    #[tokio::test]
    async fn migrates_a_legacy_stamp_and_preserves_data() {
        let dir = tempfile::tempdir().unwrap();

        let note_id = {
            let be = FsBackend::new(dir.path()).await.unwrap();
            let note = be.create_note(Note::new("legacy", "kept")).await.unwrap();
            tokio::fs::write(be.format_version_path(), "1")
                .await
                .unwrap();
            note.id
        };

        let be = FsBackend::new(dir.path()).await.unwrap();
        let stamp = tokio::fs::read_to_string(be.format_version_path())
            .await
            .unwrap();
        assert_eq!(
            stamp.trim().parse::<u32>().unwrap(),
            FsBackend::FORMAT_VERSION
        );
        assert_eq!(be.read_note(note_id).await.unwrap().body, "kept");
    }
```

**What it does** — A store rolled back to stamp 1 reopens through the ladder
to the current stamp with data intact.

### fn refuses_to_open_a_newer_format

**Identification** — tokio test; marker
`// md:mod tests > fn refuses_to_open_a_newer_format`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn refuses_to_open_a_newer_format
    #[tokio::test]
    async fn refuses_to_open_a_newer_format() {
        let dir = tempfile::tempdir().unwrap();
        {
            let be = FsBackend::new(dir.path()).await.unwrap();
            let future = (FsBackend::FORMAT_VERSION + 1).to_string();
            tokio::fs::write(be.format_version_path(), future)
                .await
                .unwrap();
        }
        let err = match FsBackend::new(dir.path()).await {
            Ok(_) => panic!("opening a newer on-disk format must be refused"),
            Err(e) => e,
        };
        assert!(
            matches!(err, StorageError::InvalidState(ref m) if m.contains("newer than this build")),
            "got: {err:?}"
        );
    }
```

**What it does** — A stamp of `FORMAT_VERSION + 1` is refused with the
"newer than this build" `InvalidState`.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `FsBackend` — defined here (EXTRACTED; the filesystem backend root)
- the repository-trait implementations (implements×6) and the log/merge/pagination helpers (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs` — error types (EXTRACTED: references×75)
- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: calls×1, references×37)
- `keeplin-core/src/storage/backend.rs` — the trait family (EXTRACTED: implements×6, references×12)
- `keeplin-core/src/storage/note_log.rs` — `merge`/`resolve`/`compact_own_log`/`VersionVector` (EXTRACTED: references×2)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/main.rs` — `build_storage` default mode (INFERRED)
- `keeplin-core/src/{history,ordering,linking,interop}.rs` tests — the cheapest real backend (EXTRACTED)
- `keeplin-core/tests/fs_backend.rs`, `tests/migrate.rs`, `tests/encryption.rs` (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `NoteMeta` | `// md:NoteMeta` |
| 3 | `NoteMetaEntry` | `// md:NoteMetaEntry` |
| 4 | `impl NoteMetaEntry` (container) | `// md:impl NoteMetaEntry` |
| 5 | `fn from_note` | `// md:impl NoteMetaEntry > fn from_note` |
| 6 | `NoteMetaIndex` | `// md:NoteMetaIndex` |
| 7 | `impl NoteMetaIndex` (container) | `// md:impl NoteMetaIndex` |
| 8 | `fn apply` | `// md:impl NoteMetaIndex > fn apply` |
| 9 | `NoteTagState` | `// md:NoteTagState` |
| 10 | `LogEntry` | `// md:LogEntry` |
| 11 | `fn default_entity_type` | `// md:fn default_entity_type` |
| 12 | `EpochHeader` | `// md:EpochHeader` |
| 13 | `fn parse_epoch_header` | `// md:fn parse_epoch_header` |
| 14 | `fn fs_tombstone_value` | `// md:fn fs_tombstone_value` |
| 15 | `fn fs_assoc_value` | `// md:fn fs_assoc_value` |
| 16 | `fn snapshot_entry_from_sidecar` | `// md:fn snapshot_entry_from_sidecar` |
| 17 | `fn snapshot_entry_from_value` | `// md:fn snapshot_entry_from_value` |
| 18 | `fn fs_assoc_from_data` | `// md:fn fs_assoc_from_data` |
| 19 | `fn fs_tombstone_from_data` | `// md:fn fs_tombstone_from_data` |
| 20 | `fn log_entry_to_change` | `// md:fn log_entry_to_change` |
| 21 | `fn atomic_write` | `// md:fn atomic_write` |
| 22 | `SyncState` | `// md:SyncState` |
| 23 | `FsBackend` | `// md:FsBackend` |
| 24 | `impl FsBackend` (container) | `// md:impl FsBackend` |
| 25 | `fn new` | `// md:impl FsBackend > fn new` |
| 26 | `fn sweep_orphan_tmp_files` | `// md:impl FsBackend > fn sweep_orphan_tmp_files` |
| 27 | `fn scan_sync_conflicts` | `// md:impl FsBackend > fn scan_sync_conflicts` |
| 28 | `fn sweep_tmp_in_dir` | `// md:impl FsBackend > fn sweep_tmp_in_dir` |
| 29 | `fn format_version_path` | `// md:impl FsBackend > fn format_version_path` |
| 30 | `fn ensure_format_version` | `// md:impl FsBackend > fn ensure_format_version` |
| 31 | `fn apply_format_migration` | `// md:impl FsBackend > fn apply_format_migration` |
| 32 | `fn note_dir` | `// md:impl FsBackend > fn note_dir` |
| 33 | `fn note_md_path` | `// md:impl FsBackend > fn note_md_path` |
| 34 | `fn note_meta_path` | `// md:impl FsBackend > fn note_meta_path` |
| 35 | `fn note_log_path` | `// md:impl FsBackend > fn note_log_path` |
| 36 | `fn device_log_path` | `// md:impl FsBackend > fn device_log_path` |
| 37 | `fn notebook_path` | `// md:impl FsBackend > fn notebook_path` |
| 38 | `fn tag_path` | `// md:impl FsBackend > fn tag_path` |
| 39 | `fn note_tag_dir` | `// md:impl FsBackend > fn note_tag_dir` |
| 40 | `fn note_tag_path` | `// md:impl FsBackend > fn note_tag_path` |
| 41 | `fn resource_dir` | `// md:impl FsBackend > fn resource_dir` |
| 42 | `fn resource_meta_path` | `// md:impl FsBackend > fn resource_meta_path` |
| 43 | `fn resource_data_path` | `// md:impl FsBackend > fn resource_data_path` |
| 44 | `fn read_or_create_device_id` | `// md:impl FsBackend > fn read_or_create_device_id` |
| 45 | `fn append_log` | `// md:impl FsBackend > fn append_log` |
| 46 | `fn maybe_compact_global_log_locked` | `// md:impl FsBackend > fn maybe_compact_global_log_locked` |
| 47 | `fn own_log_entry_count` | `// md:impl FsBackend > fn own_log_entry_count` |
| 48 | `fn read_own_epoch` | `// md:impl FsBackend > fn read_own_epoch` |
| 49 | `fn compact_global_log_locked` | `// md:impl FsBackend > fn compact_global_log_locked` |
| 50 | `fn build_global_snapshot` | `// md:impl FsBackend > fn build_global_snapshot` |
| 51 | `fn log_offset_path` | `// md:impl FsBackend > fn log_offset_path` |
| 52 | `fn read_log_offset` | `// md:impl FsBackend > fn read_log_offset` |
| 53 | `fn write_log_offset` | `// md:impl FsBackend > fn write_log_offset` |
| 54 | `fn read_log_header` | `// md:impl FsBackend > fn read_log_header` |
| 55 | `fn read_other_logs_since` | `// md:impl FsBackend > fn read_other_logs_since` |
| 56 | `fn read_new_entries` | `// md:impl FsBackend > fn read_new_entries` |
| 57 | `fn write_sidecar` | `// md:impl FsBackend > fn write_sidecar` |
| 58 | `fn read_sidecar` | `// md:impl FsBackend > fn read_sidecar` |
| 59 | `fn sidecar_vv` | `// md:impl FsBackend > fn sidecar_vv` |
| 60 | `fn next_sidecar_vv` | `// md:impl FsBackend > fn next_sidecar_vv` |
| 61 | `fn sidecar_incoming_wins` | `// md:impl FsBackend > fn sidecar_incoming_wins` |
| 62 | `fn read_assoc_state` | `// md:impl FsBackend > fn read_assoc_state` |
| 63 | `fn next_assoc_vv` | `// md:impl FsBackend > fn next_assoc_vv` |
| 64 | `fn assoc_incoming_wins` | `// md:impl FsBackend > fn assoc_incoming_wins` |
| 65 | `fn write_assoc_state` | `// md:impl FsBackend > fn write_assoc_state` |
| 66 | `fn read_resource_meta` | `// md:impl FsBackend > fn read_resource_meta` |
| 67 | `fn next_resource_vv` | `// md:impl FsBackend > fn next_resource_vv` |
| 68 | `fn resource_incoming_wins` | `// md:impl FsBackend > fn resource_incoming_wins` |
| 69 | `fn note_vv` | `// md:impl FsBackend > fn note_vv` |
| 70 | `fn read_note_logs` | `// md:impl FsBackend > fn read_note_logs` |
| 71 | `fn merge_note` | `// md:impl FsBackend > fn merge_note` |
| 72 | `fn materialize` | `// md:impl FsBackend > fn materialize` |
| 73 | `fn persist_note_projection` | `// md:impl FsBackend > fn persist_note_projection` |
| 74 | `fn read_note_projection` | `// md:impl FsBackend > fn read_note_projection` |
| 75 | `fn with_note_index` | `// md:impl FsBackend > fn with_note_index` |
| 76 | `fn build_note_index` | `// md:impl FsBackend > fn build_note_index` |
| 77 | `fn materialize_page` | `// md:impl FsBackend > fn materialize_page` |
| 78 | `fn append_note_op` | `// md:impl FsBackend > fn append_note_op` |
| 79 | `fn collect_advanced_notes` | `// md:impl FsBackend > fn collect_advanced_notes` |
| 80 | `KeyedItem` | `// md:KeyedItem` |
| 81 | `impl PartialEq for KeyedItem` | `// md:impl PartialEq for KeyedItem` |
| 82 | `impl Eq for KeyedItem` | `// md:impl Eq for KeyedItem` |
| 83 | `impl PartialOrd for KeyedItem` | `// md:impl PartialOrd for KeyedItem` |
| 84 | `impl Ord for KeyedItem` | `// md:impl Ord for KeyedItem` |
| 85 | `PageCollector` | `// md:PageCollector` |
| 86 | `impl PageCollector` (container) | `// md:impl PageCollector` |
| 87 | `fn new` | `// md:impl PageCollector > fn new` |
| 88 | `fn push` | `// md:impl PageCollector > fn push` |
| 89 | `fn into_page` | `// md:impl PageCollector > fn into_page` |
| 90 | `fn paginate` | `// md:fn paginate` |
| 91 | `impl NoteRepository for FsBackend` (container) | `// md:impl NoteRepository for FsBackend` |
| 92 | `fn create_note` | `// md:impl NoteRepository for FsBackend > fn create_note` |
| 93 | `fn read_note` | `// md:impl NoteRepository for FsBackend > fn read_note` |
| 94 | `fn update_note` | `// md:impl NoteRepository for FsBackend > fn update_note` |
| 95 | `fn delete_note` | `// md:impl NoteRepository for FsBackend > fn delete_note` |
| 96 | `fn list_notes` | `// md:impl NoteRepository for FsBackend > fn list_notes` |
| 97 | `fn list_notes_in_notebook` | `// md:impl NoteRepository for FsBackend > fn list_notes_in_notebook` |
| 98 | `fn list_starred_notes` | `// md:impl NoteRepository for FsBackend > fn list_starred_notes` |
| 99 | `fn notebook_sort_profile` | `// md:impl NoteRepository for FsBackend > fn notebook_sort_profile` |
| 100 | `impl NotebookRepository for FsBackend` (container) | `// md:impl NotebookRepository for FsBackend` |
| 101 | `fn create_notebook` | `// md:impl NotebookRepository for FsBackend > fn create_notebook` |
| 102 | `fn read_notebook` | `// md:impl NotebookRepository for FsBackend > fn read_notebook` |
| 103 | `fn update_notebook` | `// md:impl NotebookRepository for FsBackend > fn update_notebook` |
| 104 | `fn delete_notebook` | `// md:impl NotebookRepository for FsBackend > fn delete_notebook` |
| 105 | `fn list_notebooks` | `// md:impl NotebookRepository for FsBackend > fn list_notebooks` |
| 106 | `impl TagRepository for FsBackend` (container) | `// md:impl TagRepository for FsBackend` |
| 107 | `fn create_tag` | `// md:impl TagRepository for FsBackend > fn create_tag` |
| 108 | `fn read_tag` | `// md:impl TagRepository for FsBackend > fn read_tag` |
| 109 | `fn update_tag` | `// md:impl TagRepository for FsBackend > fn update_tag` |
| 110 | `fn delete_tag` | `// md:impl TagRepository for FsBackend > fn delete_tag` |
| 111 | `fn list_tags` | `// md:impl TagRepository for FsBackend > fn list_tags` |
| 112 | `fn add_note_tag` | `// md:impl TagRepository for FsBackend > fn add_note_tag` |
| 113 | `fn remove_note_tag` | `// md:impl TagRepository for FsBackend > fn remove_note_tag` |
| 114 | `fn list_note_tags` | `// md:impl TagRepository for FsBackend > fn list_note_tags` |
| 115 | `impl ResourceRepository for FsBackend` (container) | `// md:impl ResourceRepository for FsBackend` |
| 116 | `fn create_resource` | `// md:impl ResourceRepository for FsBackend > fn create_resource` |
| 117 | `fn read_resource` | `// md:impl ResourceRepository for FsBackend > fn read_resource` |
| 118 | `fn delete_resource` | `// md:impl ResourceRepository for FsBackend > fn delete_resource` |
| 119 | `fn list_resources` | `// md:impl ResourceRepository for FsBackend > fn list_resources` |
| 120 | `fn purge_deleted_resources` | `// md:impl ResourceRepository for FsBackend > fn purge_deleted_resources` |
| 121 | `impl SyncBackend for FsBackend` (container) | `// md:impl SyncBackend for FsBackend` |
| 122 | `fn get_changes_since` | `// md:impl SyncBackend for FsBackend > fn get_changes_since` |
| 123 | `fn apply_change` | `// md:impl SyncBackend for FsBackend > fn apply_change` |
| 124 | `fn get_last_sync_time` | `// md:impl SyncBackend for FsBackend > fn get_last_sync_time` |
| 125 | `fn update_sync_time` | `// md:impl SyncBackend for FsBackend > fn update_sync_time` |
| 126 | `fn send_changes` | `// md:impl SyncBackend for FsBackend > fn send_changes` |
| 127 | `fn receive_changes` | `// md:impl SyncBackend for FsBackend > fn receive_changes` |
| 128 | `fn get_device_id` | `// md:impl SyncBackend for FsBackend > fn get_device_id` |
| 129 | `fn prune_change_journal` | `// md:impl SyncBackend for FsBackend > fn prune_change_journal` |
| 130 | `impl FsBackend (global history)` (container) | `// md:impl FsBackend (global history)` |
| 131 | `fn read_all_global_entries` | `// md:impl FsBackend (global history) > fn read_all_global_entries` |
| 132 | `impl HistoryRepository for FsBackend` (container) | `// md:impl HistoryRepository for FsBackend` |
| 133 | `fn note_history` | `// md:impl HistoryRepository for FsBackend > fn note_history` |
| 134 | `fn notebook_history` | `// md:impl HistoryRepository for FsBackend > fn notebook_history` |
| 135 | `fn sort_and_cap` | `// md:fn sort_and_cap` |
| 136 | `mod tests` (container) | `// md:mod tests` |
| 137 | `fn concurrent_same_note_updates_keep_every_log_entry` | `// md:mod tests > fn concurrent_same_note_updates_keep_every_log_entry` |
| 138 | `fn read_does_not_rewrite_projection` | `// md:mod tests > fn read_does_not_rewrite_projection` |
| 139 | `fn list_notes_pages_match_full_walk` | `// md:mod tests > fn list_notes_pages_match_full_walk` |
| 140 | `fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones` | `// md:mod tests > fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones` |
| 141 | `fn failed_atomic_write_cleans_up_its_temp_file` | `// md:mod tests > fn failed_atomic_write_cleans_up_its_temp_file` |
| 142 | `fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it` | `// md:mod tests > fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it` |
| 143 | `fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair` | `// md:mod tests > fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair` |
| 144 | `fn detects_syncthing_conflict_copies_without_removing_them` | `// md:mod tests > fn detects_syncthing_conflict_copies_without_removing_them` |
| 145 | `fn purge_reclaims_old_tombstoned_payloads_only` | `// md:mod tests > fn purge_reclaims_old_tombstoned_payloads_only` |
| 146 | `fn fresh_store_is_stamped_current_version` | `// md:mod tests > fn fresh_store_is_stamped_current_version` |
| 147 | `fn migrates_a_legacy_stamp_and_preserves_data` | `// md:mod tests > fn migrates_a_legacy_stamp_and_preserves_data` |
| 148 | `fn refuses_to_open_a_newer_format` | `// md:mod tests > fn refuses_to_open_a_newer_format` |