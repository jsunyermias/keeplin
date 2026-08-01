# `storage/fs/sync.rs` — passive filesystem SyncBackend

Self-contained companion for `keeplin-core/src/storage/fs/sync.rs`. It documents every source block in source order with complete code embedded for every leaf.

**How to navigate**: each source marker `// md:<Header> > …` maps to the matching header chain below.

---

## Overview

**Identification** — file-level module declarations and imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{Change, Notebook, Resource, Tag, SYSTEM_RESOURCE_NOTE_ID};
use crate::storage::note_log::VersionVector;
use crate::storage::SyncBackend;

use super::convert::log_entry_to_change;
use super::resources::{content_hash, StoredResource};
use super::tags::NoteTagState;
use super::FsBackend;
```

**What it does** — Owns passive filesystem SyncBackend. This is a structural relocation from the former monolithic filesystem module; storage behavior, on-disk format version 8, serialization shapes, conflict resolution, and public `storage::fs::FsBackend` API are unchanged.

**Dependencies** — every binding above is either a crate symbol used directly by the blocks below or a sibling item exposed as `pub(super)`; expects: those symbols to preserve the signatures and behavior the relocated bodies already relied on, with compile-time failure on drift.

**Used by** — sibling modules under `storage/fs/` and callers of `crate::storage::fs::FsBackend`.

**Repeated context** — `FsBackend::FORMAT_VERSION` remains 8. Rust makes `mod.rs` fields visible to descendant modules; sibling-defined helpers used across files carry only `pub(super)`.

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

**What it does** — The contents of `.keeplin/sync_state.ndjson`: `last_sync`,
the watermark `get_changes_since` filters against.

**Used by** — `get_last_sync_time`, `update_sync_time`.

---

## impl SyncBackend for FsBackend

**Identification** — marker `// md:impl SyncBackend for FsBackend`; per-method
markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn get_changes_since, fn apply_change, fn get_last_sync_time, fn update_sync_time, fn send_changes, fn receive_changes, fn get_device_id, fn prune_change_journal.

**What it does** — the passive-replication sync surface.

---

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

---

### fn apply_change

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn apply_change`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncBackend for FsBackend > fn apply_change
    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        match change {
            Change::NoteCreate { note } | Change::NoteUpdate { note } => {
                let prior_deleted = self.merge_note(note.id).await?.and_then(|n| n.deleted_at);
                let materialized = self.materialize(note.id).await?;
                if materialized.is_some_and(|n| n.deleted_at.is_none()) {
                    if let Some(old_ts) = prior_deleted {
                        self.cascade_unstamp_resources(note.id, old_ts).await?;
                    }
                }
                tracing::debug!(id = %note.id, "Materialized remote note change");
            }
            Change::NoteDelete { id, deleted_at, .. } => {
                self.materialize(id).await?;
                self.cascade_stamp_resources(id, deleted_at).await?;
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
                            system: false,
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
                    tokio::fs::create_dir_all(self.note_resources_dir(resource.note_id)).await?;
                    let blob_hash = match &data {
                        Some(bytes) => {
                            let hash = content_hash(bytes);
                            tokio::fs::write(
                                self.resource_blob_path(resource.note_id, &hash),
                                bytes,
                            )
                            .await?;
                            hash
                        }
                        None => self
                            .read_resource_sidecar(resource.id)
                            .await?
                            .map(|(_, s)| s.blob_hash)
                            .unwrap_or_default(),
                    };
                    let stored = StoredResource {
                        resource: resource.clone(),
                        blob_hash,
                    };
                    self.write_sidecar(
                        &self.resource_meta_path(resource.note_id, resource.id),
                        &stored,
                    )
                    .await?;
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
                    let (note_id, mut stored) = match self.read_resource_sidecar(id).await? {
                        Some(found) => found,
                        None => (
                            SYSTEM_RESOURCE_NOTE_ID,
                            StoredResource {
                                resource: Resource {
                                    id,
                                    note_id: SYSTEM_RESOURCE_NOTE_ID,
                                    title: String::new(),
                                    mime_type: String::new(),
                                    file_name: String::new(),
                                    size: 0,
                                    duration_ms: None,
                                    dimensions: None,
                                    created_at: deleted_at,
                                    deleted_at: None,
                                    vv: VersionVector::new(),
                                    last_writer: String::new(),
                                },
                                blob_hash: String::new(),
                            },
                        ),
                    };
                    stored.resource.deleted_at = Some(deleted_at);
                    stored.resource.vv = vv;
                    stored.resource.last_writer = last_writer;
                    tokio::fs::create_dir_all(self.note_resources_dir(note_id)).await?;
                    self.write_sidecar(&self.resource_meta_path(note_id, id), &stored)
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
- **ResourceCreate** — gate; ensure the note's `resources/` folder, then write
  the `{hash}.knrs` blob only when the change carries bytes (`data = Some` from a
  DbBackend peer; `None` from an FsBackend peer whose blob Syncthing replicates
  independently — the sidecar keeps whatever `blob_hash` a prior bytes-bearing
  create already recorded), then write the `StoredResource` sidecar.
- **ResourceDelete** — gate; tombstone the existing sidecar or — unknown
  locally — write a minimal tombstone under `SYSTEM_RESOURCE_NOTE_ID` with an
  empty `blob_hash` so a later stale create loses in `resolve` (issue #71); the
  blob is retained.

---

### fn get_last_sync_time

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn get_last_sync_time`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncBackend for FsBackend > fn get_last_sync_time
    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
        let path = self.root.join(".keeplin").join("sync_state.ndjson");
        match self.read_sidecar::<SyncState>(&path, Uuid::nil()).await {
            Ok(state) => Ok(state.last_sync),
            Err(StorageError::NotFound(_)) => {
                Ok(DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default())
            }
            Err(e) => Err(e),
        }
    }
```

**What it does** — `.keeplin/sync_state.ndjson`, epoch when absent.

---

### fn update_sync_time

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn update_sync_time`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncBackend for FsBackend > fn update_sync_time
    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        let state = SyncState { last_sync: ts };
        let path = self.root.join(".keeplin").join("sync_state.ndjson");
        self.write_sidecar(&path, &state).await
    }
```

**What it does** — Atomic sidecar write of the watermark.

---

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

---

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

---

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

---

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

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2; CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the same ignored `graphify-out/` layout locally.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `passive filesystem SyncBackend` — defined or implemented in this focused filesystem module (INFERRED)

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
| 2 | `SyncState` | `// md:SyncState` |
| 3 | `impl SyncBackend for FsBackend` (container) | `// md:impl SyncBackend for FsBackend` |
| 4 | `fn get_changes_since` | `// md:impl SyncBackend for FsBackend > fn get_changes_since` |
| 5 | `fn apply_change` | `// md:impl SyncBackend for FsBackend > fn apply_change` |
| 6 | `fn get_last_sync_time` | `// md:impl SyncBackend for FsBackend > fn get_last_sync_time` |
| 7 | `fn update_sync_time` | `// md:impl SyncBackend for FsBackend > fn update_sync_time` |
| 8 | `fn send_changes` | `// md:impl SyncBackend for FsBackend > fn send_changes` |
| 9 | `fn receive_changes` | `// md:impl SyncBackend for FsBackend > fn receive_changes` |
| 10 | `fn get_device_id` | `// md:impl SyncBackend for FsBackend > fn get_device_id` |
| 11 | `fn prune_change_journal` | `// md:impl SyncBackend for FsBackend > fn prune_change_journal` |
