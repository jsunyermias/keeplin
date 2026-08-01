# `storage/fs/notes.rs` — per-device note logs, projections, indexing, and NoteRepository

Self-contained companion for `keeplin-core/src/storage/fs/notes.rs`. It documents every source block in source order with complete code embedded for every leaf.

---

## Overview

**Identification** — file-level module declarations and imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{now, Change, Note};
use crate::storage::note_log::{self, NoteLogEntry, NoteOp, VersionVector};
use crate::storage::{NoteRepository, NotebookSortProfile, SortableRfc3339};

use super::io::atomic_write;
use super::pagination::PageCollector;
use super::FsBackend;
```

**What it does** — Owns per-device note logs, projections, indexing, and `NoteRepository`. This is a structural relocation from the former monolithic filesystem module; storage behavior, format version 8, serialization, conflict resolution, and public API are unchanged.

**Dependencies** — every binding above is used below or exposes a sibling item as `pub(super)`; expects: pre-split signatures and behavior.

**Used by** — sibling `storage/fs/` modules and existing `FsBackend` callers.

**Repeated context** — `FsBackend::FORMAT_VERSION` remains 8.

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
`notes/{id}/meta.ndjson`: the merged note (body blanked — it lives in `note.md`)
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

---

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
pub(super) struct NoteMetaIndex {
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

---

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

## fn parse_note_log

**Identification** — `fn parse_note_log(bytes: &[u8]) -> Result<Vec<NoteLogEntry>, StorageError>`;
marker `// md:fn parse_note_log`.

**Code** — complete and verbatim:

```rust
// md:fn parse_note_log
fn parse_note_log(bytes: &[u8]) -> Result<Vec<NoteLogEntry>, StorageError> {
    let mut out = Vec::new();
    for line in bytes.split(|b| *b == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let entry: NoteLogEntry = serde_json::from_slice(line)
            .map_err(|e| StorageError::CorruptedData(format!("ndjson decode: {e}")))?;
        out.push(entry);
    }
    Ok(out)
}
```

**What it does** — Parses a per-device note log from NDJSON: splits on `\n`,
skips blank/whitespace-only lines (so a trailing newline is tolerated), and
decodes each remaining line into a `NoteLogEntry`. The first malformed line
aborts with `CorruptedData` (all-or-nothing, matching the previous single-blob
decode). This is the inverse of `write_note_log`.

**Dependencies** —
- `serde_json::from_slice` — decodes one line into a `NoteLogEntry`; expects
  each non-blank line to be a complete JSON object — a truncated last line
  surfaces as `CorruptedData`, not a partial entry.

**Used by** — `read_note_logs` (all logs of a note, per-file error is logged and
skipped there) and `append_note_op` (the own-device log, where a decode error
propagates).

**Repeated context** — Per-device logs are single-writer, so a well-formed log
is only ever produced by this device's `write_note_log`; corruption means an
external truncation.

---

## impl FsBackend

**Identification** — the first inherent impl; marker `// md:impl FsBackend`.
Constructor, sweeps, format versioning, path helpers, log machinery,
sidecar/association/resource versioning, the note merge pipeline. Contains the
constants `FORMAT_VERSION = 8`, `NOTE_LOG_COMPACT_THRESHOLD = 256`,
`GLOBAL_LOG_COMPACT_THRESHOLD = 512`, `GLOBAL_LOG_SOFT_BYTES = 64 KiB`
(documented with the methods that use them).

**Code** — container: members documented as sub-blocks below: fn note_dir, fn note_md_path, fn note_meta_path, fn note_log_path, fn write_note_log, fn note_vv, fn read_note_logs, fn merge_note, fn materialize, fn persist_note_projection, fn read_note_projection, fn with_note_index, fn build_note_index, fn materialize_page, fn append_note_op, fn collect_advanced_notes.

---

### fn note_dir

**Identification** — marker `// md:impl FsBackend > fn note_dir`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_dir
    pub(super) fn note_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("notes").join(id.to_string())
    }
```

**What it does** — `{root}/notes/{id}`.

---

### fn note_md_path

**Identification** — marker `// md:impl FsBackend > fn note_md_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_md_path
    pub(super) fn note_md_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("note.md")
    }
```

**What it does** — `…/note.md` (human-readable when unencrypted).

---

### fn note_meta_path

**Identification** — marker `// md:impl FsBackend > fn note_meta_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_meta_path
    pub(super) fn note_meta_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("meta.ndjson")
    }
```

**What it does** — `…/meta.ndjson` (cache, not source of truth).

---

### fn note_log_path

**Identification** — marker `// md:impl FsBackend > fn note_log_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_log_path
    pub(super) fn note_log_path(&self, id: Uuid, device_id: &str) -> PathBuf {
        self.note_dir(id).join(format!("log.{device_id}.ndjson"))
    }
```

**What it does** — `…/log.{device_id}.ndjson` (single-writer op log).

---

### fn write_note_log

**Identification** — marker `// md:impl FsBackend > fn write_note_log`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn write_note_log
    pub(super) async fn write_note_log(
        &self,
        path: &Path,
        entries: &[NoteLogEntry],
    ) -> Result<(), StorageError> {
        let mut bytes = Vec::new();
        for entry in entries {
            let line = serde_json::to_vec(entry)
                .map_err(|e| StorageError::InvalidState(format!("ndjson encode: {e}")))?;
            bytes.extend_from_slice(&line);
            bytes.push(b'\n');
        }
        atomic_write(path, &bytes).await
    }
```

**What it does** — Serialises a per-device note log as NDJSON: one
`NoteLogEntry` JSON object per line, `\n`-terminated, then one atomic replace of
the whole file (the log is rewritten wholesale on every append/compaction, so
this is not append-in-place). An empty slice writes an empty file. Encode
failure → `InvalidState`.

**Dependencies** —
- `serde_json::to_vec` — encodes one entry per line; expects `NoteLogEntry` to
  be `Serialize` (it derives it) — a non-serialisable op would surface as
  `InvalidState`, not a panic.
- `atomic_write` — write-temp-then-rename; expects it to replace the file
  atomically so a crash never leaves a half-written log.

**Used by** — `append_note_op` (the only writer of the own-device log).

**Repeated context** — Filesystem state is NDJSON, not MessagePack; the atomic
write-temp-then-rename pattern is unchanged by the format.

---

### fn note_vv

**Identification** — marker `// md:impl FsBackend > fn note_vv`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_vv
    pub(super) async fn note_vv(&self, id: Uuid) -> Result<VersionVector, StorageError> {
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

---

### fn read_note_logs

**Identification** — marker `// md:impl FsBackend > fn read_note_logs`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_note_logs
    pub(super) async fn read_note_logs(
        &self,
        id: Uuid,
    ) -> Result<Vec<Vec<NoteLogEntry>>, StorageError> {
        let dir = self.note_dir(id);
        let mut logs = Vec::new();
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(logs),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("log.") && name.ends_with(".ndjson") {
                let bytes = tokio::fs::read(entry.path()).await?;
                match parse_note_log(&bytes) {
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

**What it does** — Reads every `log.*.ndjson` for a note. A missing directory
yields empty; an unreadable individual log is **excluded from the merge and
reported at error level** (that device's entire history is missing — a
silent-data-loss risk, not routine). The file is left in place (it belongs to
another device; a local rename would replicate back to its writer), so a
restored copy re-enters the merge on the next read.

---

### fn merge_note

**Identification** — marker `// md:impl FsBackend > fn merge_note`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn merge_note
    pub(super) async fn merge_note(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
        let logs = self.read_note_logs(id).await?;
        Ok(note_log::merge(&logs).note)
    }
```

**What it does** — Merge without touching disk. Reads use this so a read never
rewrites projections (no write amplification) and never consumes a peer change
the next sync should report. `None` when the note has no entries.

---

### fn materialize

**Identification** — marker `// md:impl FsBackend > fn materialize`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn materialize
    pub(super) async fn materialize(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
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

**What it does** — Merge + refresh the `note.md`/`meta.ndjson` projection
(used by write and sync paths, never reads); a resolved concurrent conflict is
logged.

---

### fn persist_note_projection

**Identification** — marker
`// md:impl FsBackend > fn persist_note_projection`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn persist_note_projection
    pub(super) async fn persist_note_projection(
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
+ vector to `meta.ndjson` (both atomic). **The single choke point every note
write passes through**, so it also keeps the `NoteMetaIndex` current: on-disk
first, then the index entry (only if built — an unbuilt index misses nothing,
its eventual build reads the fresh projection). A crash between the two
leaves the index no staler than the projection.

---

### fn read_note_projection

**Identification** — marker
`// md:impl FsBackend > fn read_note_projection`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_note_projection
    pub(super) async fn read_note_projection(
        &self,
        id: Uuid,
    ) -> Result<Option<Note>, StorageError> {
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

---

### fn with_note_index

**Identification** — marker `// md:impl FsBackend > fn with_note_index`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn with_note_index
    pub(super) async fn with_note_index<R>(
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

---

### fn build_note_index

**Identification** — marker `// md:impl FsBackend > fn build_note_index`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn build_note_index
    pub(super) async fn build_note_index(&self) -> Result<NoteMetaIndex, StorageError> {
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

---

### fn materialize_page

**Identification** — marker `// md:impl FsBackend > fn materialize_page`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn materialize_page
    pub(super) async fn materialize_page(&self, ids: Vec<Uuid>) -> Result<Vec<Note>, StorageError> {
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

---

### fn append_note_op

**Identification** — marker `// md:impl FsBackend > fn append_note_op`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn append_note_op
    pub(super) async fn append_note_op(&self, id: Uuid, op: NoteOp) -> Result<Note, StorageError> {
        let _write_guard = self.note_write_lock.lock().await;
        tokio::fs::create_dir_all(self.note_dir(id)).await?;
        let mut vv = note_log::merge(&self.read_note_logs(id).await?).vv;
        note_log::increment(&mut vv, &self.device_id);
        let log_path = self.note_log_path(id, &self.device_id);
        let mut log: Vec<NoteLogEntry> = if log_path.exists() {
            let bytes = tokio::fs::read(&log_path).await?;
            parse_note_log(&bytes)?
        } else {
            Vec::new()
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
        self.write_note_log(&log_path, &log).await?;
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

---

### fn collect_advanced_notes

**Identification** — marker
`// md:impl FsBackend > fn collect_advanced_notes`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn collect_advanced_notes
    pub(super) async fn collect_advanced_notes(&self) -> Result<Vec<Change>, StorageError> {
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

## impl NoteRepository for FsBackend

**Identification** — marker `// md:impl NoteRepository for FsBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_note, fn read_note, fn update_note, fn delete_note, fn list_notes, fn list_notes_in_notebook, fn list_starred_notes, fn notebook_sort_profile.

**What it does** — the note surface over the log pipeline.

---

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

---

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

---

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
        let id = note.id;
        let prior_deleted = self.merge_note(id).await?.and_then(|n| n.deleted_at);
        let merged = self.append_note_op(id, NoteOp::Upsert(note)).await?;
        if merged.deleted_at.is_none() {
            if let Some(old_ts) = prior_deleted {
                self.cascade_unstamp_resources(id, old_ts).await?;
            }
        }
        tracing::info!(id = %merged.id, "Note updated");
        Ok(merged)
    }
```

**What it does** — `NotFound` when the note has no logs at all, else
`append_note_op(Upsert)`. **Restore cascade (issue #125):** the note's `deleted_at`
is read (via `merge_note`) before the upsert; if the upsert makes it live again and it
was previously tombstoned, the resources it dragged down (whose `deleted_at` equals the
old tombstone ts) are un-stamped via `cascade_unstamp_resources`.

---

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
        let ts = now();
        self.append_note_op(id, NoteOp::Tombstone { deleted_at: ts })
            .await?;
        self.cascade_stamp_resources(id, ts).await?;
        tracing::info!(%id, "Note deleted");
        Ok(())
    }
```

**What it does** — `NotFound` without logs, else
`append_note_op(Tombstone { deleted_at: ts })`. **Delete cascade (issue #125):** after
the tombstone is appended, `cascade_stamp_resources(id, ts)` stamps every live resource
with this `note_id` at the same tombstone ts, so attachments follow their note down.

---

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
        let limit = crate::storage::effective_page_size(page_size) as usize;
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

---

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
        let limit = crate::storage::effective_page_size(page_size) as usize;
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

---

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
        let limit = crate::storage::effective_page_size(page_size) as usize;
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

---

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

## Graph context

Repo-tooling metadata, not a code block.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- note log and repository implementation — defined here (INFERRED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/fs/mod.rs` and sibling `storage/fs/` modules — shared backend state and helpers (INFERRED)
- `keeplin-core/src/storage/backend.rs`, `models.rs`, `error.rs`, and `storage/note_log.rs` — unchanged storage contracts (INFERRED)

**Direct dependents** (files whose symbols reference this one)

- sibling `storage/fs/` modules and existing callers (INFERRED)

**Invariants** (the rules this file must keep true)

- This split does not change the on-disk format; `FsBackend::FORMAT_VERSION` remains 8.
- The public backend path remains `crate::storage::fs::FsBackend`.
- Note merge, projection, and pagination behavior remains unchanged.

---

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
| 9 | `fn parse_note_log` | `// md:fn parse_note_log` |
| 10 | `impl FsBackend` (container) | `// md:impl FsBackend` |
| 11 | `fn note_dir` | `// md:impl FsBackend > fn note_dir` |
| 12 | `fn note_md_path` | `// md:impl FsBackend > fn note_md_path` |
| 13 | `fn note_meta_path` | `// md:impl FsBackend > fn note_meta_path` |
| 14 | `fn note_log_path` | `// md:impl FsBackend > fn note_log_path` |
| 15 | `fn write_note_log` | `// md:impl FsBackend > fn write_note_log` |
| 16 | `fn note_vv` | `// md:impl FsBackend > fn note_vv` |
| 17 | `fn read_note_logs` | `// md:impl FsBackend > fn read_note_logs` |
| 18 | `fn merge_note` | `// md:impl FsBackend > fn merge_note` |
| 19 | `fn materialize` | `// md:impl FsBackend > fn materialize` |
| 20 | `fn persist_note_projection` | `// md:impl FsBackend > fn persist_note_projection` |
| 21 | `fn read_note_projection` | `// md:impl FsBackend > fn read_note_projection` |
| 22 | `fn with_note_index` | `// md:impl FsBackend > fn with_note_index` |
| 23 | `fn build_note_index` | `// md:impl FsBackend > fn build_note_index` |
| 24 | `fn materialize_page` | `// md:impl FsBackend > fn materialize_page` |
| 25 | `fn append_note_op` | `// md:impl FsBackend > fn append_note_op` |
| 26 | `fn collect_advanced_notes` | `// md:impl FsBackend > fn collect_advanced_notes` |
| 27 | `impl NoteRepository for FsBackend` (container) | `// md:impl NoteRepository for FsBackend` |
| 28 | `fn create_note` | `// md:impl NoteRepository for FsBackend > fn create_note` |
| 29 | `fn read_note` | `// md:impl NoteRepository for FsBackend > fn read_note` |
| 30 | `fn update_note` | `// md:impl NoteRepository for FsBackend > fn update_note` |
| 31 | `fn delete_note` | `// md:impl NoteRepository for FsBackend > fn delete_note` |
| 32 | `fn list_notes` | `// md:impl NoteRepository for FsBackend > fn list_notes` |
| 33 | `fn list_notes_in_notebook` | `// md:impl NoteRepository for FsBackend > fn list_notes_in_notebook` |
| 34 | `fn list_starred_notes` | `// md:impl NoteRepository for FsBackend > fn list_starred_notes` |
| 35 | `fn notebook_sort_profile` | `// md:impl NoteRepository for FsBackend > fn notebook_sort_profile` |
