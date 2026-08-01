# `storage/fs/resources.rs` — content-addressed attachment storage

Self-contained companion for `keeplin-core/src/storage/fs/resources.rs`. It documents every source block in source order with complete code embedded for every leaf.

**How to navigate**: each source marker `// md:<Header> > …` maps to the matching header chain below.

---

## Overview

**Identification** — file-level module declarations and imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::path::PathBuf;

use async_trait::async_trait;
use blake2::{Blake2s256, Digest};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{Resource, SYSTEM_RESOURCE_NOTE_ID};
use crate::storage::note_log::{self, resolve, VersionVector, Winner};
use crate::storage::{ResourceRepository, SortableRfc3339};

use super::convert::fs_tombstone_value;
use super::pagination::paginate;
use super::FsBackend;
```

**What it does** — Owns content-addressed attachment storage. This is a structural relocation from the former monolithic filesystem module; storage behavior, on-disk format version 8, serialization shapes, conflict resolution, and public `storage::fs::FsBackend` API are unchanged.

**Dependencies** — every binding above is either a crate symbol used directly by the blocks below or a sibling item exposed as `pub(super)`; expects: those symbols to preserve the signatures and behavior the relocated bodies already relied on, with compile-time failure on drift.

**Used by** — sibling modules under `storage/fs/` and callers of `crate::storage::fs::FsBackend`.

**Repeated context** — `FsBackend::FORMAT_VERSION` remains 8. Rust makes `mod.rs` fields visible to descendant modules; sibling-defined helpers used across files carry only `pub(super)`.

---

## fn content_hash

**Identification** — private free function `fn content_hash(data: &[u8]) -> String`;
marker `// md:fn content_hash`.

**Code** — complete and verbatim:

```rust
// md:fn content_hash
pub(super) fn content_hash(data: &[u8]) -> String {
    let digest = Blake2s256::digest(data);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
```

**What it does** — The physical storage key for an attachment: the BLAKE2s-256
digest of its bytes, lowercase-hex encoded (64 chars). Content-addressed and
deterministic — identical bytes always yield the same string, distinct bytes
(barring a cryptographic collision) always differ — which is what makes
`{hash}.knrs` naming both stable across devices and self-deduplicating within a
note. Pure; total; never fails.

**Dependencies** —
- `Blake2s256::digest` / `blake2::Digest` — one-shot hash of the whole slice;
  expects a stable 256-bit output per input. If the algorithm or its output
  encoding ever changes, every previously written `{hash}.knrs` name stops
  matching freshly computed hashes — a silent read miss, not a compile error.

**Used by** — `create_resource` (names the blob it writes), `read_resource` (via
the stored `blob_hash`, not recomputed), `apply_change` `ResourceCreate` (names a
replicated blob), and the layout/dedup tests.

**Repeated context** — the hash is fs-local storage metadata only; it never
appears in the wire `Resource` or any `Change`.

---

## StoredResource

**Identification** — private serde struct; marker `// md:StoredResource`.

**Code** — complete and verbatim:

```rust
// md:StoredResource
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StoredResource {
    #[serde(flatten)]
    pub(super) resource: Resource,
    #[serde(default)]
    pub(super) blob_hash: String,
}
```

**What it does** — The on-disk shape of `{id}.meta.ndjson`: the wire `Resource`
flattened at the top level plus a fs-local `blob_hash` that maps the resource's
`id` to the `{hash}.knrs` blob holding its bytes. `#[serde(flatten)]` keeps the
JSON object shape identical to a bare `Resource` with one extra key, so anything
that deserialises the sidecar as a plain `Resource` (e.g.
`snapshot_entry_from_sidecar::<Resource>`) still works and silently ignores
`blob_hash`; `#[serde(default)]` lets a sidecar with no hash (a metadata-only
create, or a fabricated tombstone) decode with an empty string. Because the wire
`Resource` is embedded rather than wrapped, the shared type never changes shape.

**Used by** — every read/write of a resource sidecar: `read_resource_sidecar`,
`create_resource`, `delete_resource`, `list_resources`,
`list_resources_for_note`, `purge_deleted_resources`, `cascade_stamp_resources`,
`cascade_unstamp_resources`, and both resource arms of `apply_change`.

**Repeated context** — sidecars are single-object NDJSON written by
`write_sidecar`; the encryption-at-rest rule does not touch id-plaintext metadata.

---

## impl FsBackend

**Identification** — the first inherent impl; marker `// md:impl FsBackend`.
Constructor, sweeps, format versioning, path helpers, log machinery,
sidecar/association/resource versioning, the note merge pipeline. Contains the
constants `FORMAT_VERSION = 8`, `NOTE_LOG_COMPACT_THRESHOLD = 256`,
`GLOBAL_LOG_COMPACT_THRESHOLD = 512`, `GLOBAL_LOG_SOFT_BYTES = 64 KiB`
(documented with the methods that use them).

**Code** — container: members documented as sub-blocks below: fn note_resources_dir, fn resource_meta_path, fn resource_blob_path, fn all_note_ids, fn note_resource_ids, fn locate_resource_note, fn read_resource_sidecar, fn read_resource_meta, fn next_resource_vv, fn resource_incoming_wins, fn cascade_stamp_resources, fn cascade_unstamp_resources.

---

### fn note_resources_dir

**Identification** — marker `// md:impl FsBackend > fn note_resources_dir`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_resources_dir
    pub(super) fn note_resources_dir(&self, note_id: Uuid) -> PathBuf {
        self.note_dir(note_id).join("resources")
    }
```

**What it does** — `notes/{note_id}/resources` — the folder holding one note's
attachment blobs and their meta sidecars. Replaces the old global
`resources/{id}/` pool (issue #127).

**Dependencies** —
- `note_dir` — the note's own directory; expects `notes/{note_id}`.

**Used by** — `resource_meta_path`, `resource_blob_path`, `note_resource_ids`,
and every resource create/apply path (via `create_dir_all`).

---

### fn resource_meta_path

**Identification** — marker `// md:impl FsBackend > fn resource_meta_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn resource_meta_path
    pub(super) fn resource_meta_path(&self, note_id: Uuid, id: Uuid) -> PathBuf {
        self.note_resources_dir(note_id)
            .join(format!("{id}.meta.ndjson"))
    }
```

**What it does** — `notes/{note_id}/resources/{id}.meta.ndjson` — the
`StoredResource` sidecar, indexed by the resource `id` so it never collides with
a `{hash}.knrs` blob and is skipped by the `.knrs` blob sweep.

**Dependencies** —
- `note_resources_dir` — the containing folder; expects `notes/{note_id}/resources`.

**Used by** — every resource read/write path and `locate_resource_note`.

---

### fn resource_blob_path

**Identification** — marker `// md:impl FsBackend > fn resource_blob_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn resource_blob_path
    pub(super) fn resource_blob_path(&self, note_id: Uuid, hash: &str) -> PathBuf {
        self.note_resources_dir(note_id)
            .join(format!("{hash}.knrs"))
    }
```

**What it does** — `notes/{note_id}/resources/{hash}.knrs` — the attachment bytes
in their original format, named by `content_hash`. The `.knrs` file **is** the
original renamed, not a container; two live resources in one note with identical
content share a single blob.

**Dependencies** —
- `note_resources_dir` — the containing folder; expects `notes/{note_id}/resources`.

**Used by** — `create_resource`, `read_resource`, `purge_deleted_resources`, and
`apply_change` `ResourceCreate`.

---

### fn all_note_ids

**Identification** — marker `// md:impl FsBackend > fn all_note_ids`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn all_note_ids
    pub(super) async fn all_note_ids(&self) -> Result<Vec<Uuid>, StorageError> {
        let mut ids = Vec::new();
        let mut rd = match tokio::fs::read_dir(self.root.join("notes")).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            if let Ok(id) = Uuid::parse_str(&entry.file_name().to_string_lossy()) {
                ids.push(id);
            }
        }
        Ok(ids)
    }
```

**What it does** — Every note-directory UUID under `notes/`. A missing `notes/`
dir yields an empty vector (no error). Non-UUID entries are skipped. It backs the
whole-store resource walks now that attachments are scattered across note folders
instead of a single pool.

**Dependencies** —
- `tokio::fs::read_dir` — lists `notes/`; expects `NotFound` to mean "no notes
  yet", every other error to propagate.

**Used by** — `list_resources`, `purge_deleted_resources`,
`build_global_snapshot`, `locate_resource_note`.

---

### fn note_resource_ids

**Identification** — marker `// md:impl FsBackend > fn note_resource_ids`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn note_resource_ids
    pub(super) async fn note_resource_ids(&self, note_id: Uuid) -> Result<Vec<Uuid>, StorageError> {
        let mut ids = Vec::new();
        let mut rd = match tokio::fs::read_dir(self.note_resources_dir(note_id)).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".meta.ndjson") else {
                continue;
            };
            if let Ok(id) = Uuid::parse_str(stem) {
                ids.push(id);
            }
        }
        Ok(ids)
    }
```

**What it does** — The resource UUIDs of one note, read from the
`{id}.meta.ndjson` sidecars in its `resources/` folder. The `.knrs` blobs are
skipped (they do not end in `.meta.ndjson`), so a resource is enumerated exactly
once regardless of how many blobs it shares. A note with no attachments folder
yields an empty vector.

**Dependencies** —
- `note_resources_dir` — the folder to scan; expects `notes/{note_id}/resources`.
- `tokio::fs::read_dir` — lists it; expects `NotFound` to mean "no attachments".

**Used by** — `list_resources`, `list_resources_for_note`,
`purge_deleted_resources`, `build_global_snapshot`, `cascade_stamp_resources`,
`cascade_unstamp_resources`.

---

### fn locate_resource_note

**Identification** — marker `// md:impl FsBackend > fn locate_resource_note`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn locate_resource_note
    pub(super) async fn locate_resource_note(
        &self,
        id: Uuid,
    ) -> Result<Option<Uuid>, StorageError> {
        for note_id in self.all_note_ids().await? {
            if self.resource_meta_path(note_id, id).exists() {
                return Ok(Some(note_id));
            }
        }
        Ok(None)
    }
```

**What it does** — Resolves a bare resource `id` to its owning `note_id` by
scanning note folders for a `{id}.meta.ndjson` sidecar. This is the price of a
per-note layout: the `ResourceRepository` trait keys reads/deletes by `id` alone,
but the storage path needs the `note_id`. Returns the first match (`id`s are
unique, so at most one exists in practice); `None` if no sidecar carries that id.

**Dependencies** —
- `all_note_ids` — the folders to search; expects every note dir enumerated.
- `resource_meta_path` — the candidate path; expects `.exists()` to be a cheap
  stat, not a read.

**Used by** — `read_resource_sidecar` (hence `read_resource`, `delete_resource`,
`read_resource_meta`, `resource_incoming_wins`, `next_resource_vv`, and the
resource arms of `apply_change`).

---

### fn read_resource_sidecar

**Identification** — marker `// md:impl FsBackend > fn read_resource_sidecar`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_resource_sidecar
    pub(super) async fn read_resource_sidecar(
        &self,
        id: Uuid,
    ) -> Result<Option<(Uuid, StoredResource)>, StorageError> {
        let Some(note_id) = self.locate_resource_note(id).await? else {
            return Ok(None);
        };
        let stored: StoredResource = self
            .read_sidecar(&self.resource_meta_path(note_id, id), id)
            .await?;
        Ok(Some((note_id, stored)))
    }
```

**What it does** — Locates a resource by `id` and reads its `StoredResource`
sidecar, returning both the owning `note_id` and the parsed record (which carries
the `blob_hash` needed to reach the bytes). `None` when no sidecar exists;
`CorruptedData` when one exists but will not parse.

**Dependencies** —
- `locate_resource_note` — finds the owning note; expects `None` for "no such
  resource".
- `read_sidecar` — parses the sidecar as `StoredResource`; expects the file to
  exist (guaranteed by the locate step) and to decode.

**Used by** — `read_resource_meta`, `read_resource`, `delete_resource`, and both
resource arms of `apply_change`.

---

### fn read_resource_meta

**Identification** — marker `// md:impl FsBackend > fn read_resource_meta`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_resource_meta
    pub(super) async fn read_resource_meta(
        &self,
        id: Uuid,
    ) -> Result<Option<Resource>, StorageError> {
        Ok(self
            .read_resource_sidecar(id)
            .await?
            .map(|(_, stored)| stored.resource))
    }
```

**What it does** — The wire `Resource` for an id, `None` when no sidecar exists.
Delegates the locate-and-read to `read_resource_sidecar` and drops both the
owning `note_id` and the fs-local `blob_hash`, keeping the same
`Option<Resource>` contract its version-vector callers expect.

---

### fn next_resource_vv

**Identification** — marker `// md:impl FsBackend > fn next_resource_vv`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn next_resource_vv
    pub(super) async fn next_resource_vv(&self, id: Uuid) -> Result<VersionVector, StorageError> {
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

---

### fn resource_incoming_wins

**Identification** — marker
`// md:impl FsBackend > fn resource_incoming_wins`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn resource_incoming_wins
    pub(super) async fn resource_incoming_wins(
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

---

### fn cascade_stamp_resources

**Identification** — `async fn cascade_stamp_resources(&self, note_id: Uuid, deleted_at:
DateTime<Utc>) -> Result<(), StorageError>`; marker
`// md:impl FsBackend > fn cascade_stamp_resources`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn cascade_stamp_resources
    pub(super) async fn cascade_stamp_resources(
        &self,
        note_id: Uuid,
        deleted_at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        for id in self.note_resource_ids(note_id).await? {
            let meta_path = self.resource_meta_path(note_id, id);
            let mut stored: StoredResource = match self.read_sidecar(&meta_path, id).await {
                Ok(s) => s,
                Err(StorageError::NotFound(_)) => continue,
                Err(e) => return Err(e),
            };
            if stored.resource.deleted_at.is_none() {
                stored.resource.deleted_at = Some(deleted_at);
                self.write_sidecar(&meta_path, &stored).await?;
            }
        }
        Ok(())
    }
```

**What it does** — The soft-delete cascade (issue #125, D3): when a note is tombstoned, every
**live** attachment in that note is stamped `deleted_at = <the note's tombstone ts>`. With the
per-note layout it only walks that note's own `resources/` folder (`note_resource_ids`), so the
`note_id == …` predicate is now implicit — every sidecar there belongs to the note. For each live
one it rewrites only the `deleted_at` field of the `StoredResource` (preserving `blob_hash`) —
deliberately **without** bumping `vv`/`last_writer` (so replicas don't diverge on version
vectors) and **without** `append_log` (so the cascade never echoes into the relay broadcast).
Convergence comes from every replica applying the same cascade when it applies the `NoteDelete`.
A note with no attachments folder is a no-op.

**Dependencies** —
- `note_resource_ids` — the note's attachment ids; expects a missing folder to yield an empty list.
- `resource_meta_path`, `read_sidecar`, `write_sidecar` — per-resource sidecar IO; expect
  `read_sidecar` to surface `NotFound` for a resource with no meta (skipped, not fatal).
- `StoredResource.resource.deleted_at` — the live predicate and the only field rewritten.

**Used by** — `delete_note` (local delete) and `apply_change(NoteDelete)` (sync).

**Repeated context** — the cascade is derived state, never journaled: no `vv`/`last_writer`
bump, no `append_log`.

---

### fn cascade_unstamp_resources

**Identification** — `async fn cascade_unstamp_resources(&self, note_id: Uuid, deleted_at:
DateTime<Utc>) -> Result<(), StorageError>`; marker
`// md:impl FsBackend > fn cascade_unstamp_resources`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn cascade_unstamp_resources
    pub(super) async fn cascade_unstamp_resources(
        &self,
        note_id: Uuid,
        deleted_at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        for id in self.note_resource_ids(note_id).await? {
            let meta_path = self.resource_meta_path(note_id, id);
            let mut stored: StoredResource = match self.read_sidecar(&meta_path, id).await {
                Ok(s) => s,
                Err(StorageError::NotFound(_)) => continue,
                Err(e) => return Err(e),
            };
            if stored.resource.deleted_at == Some(deleted_at) {
                stored.resource.deleted_at = None;
                self.write_sidecar(&meta_path, &stored).await?;
            }
        }
        Ok(())
    }
```

**What it does** — The restore side of the cascade: when a tombstoned note is revived, only the
attachments the note **actually dragged down** are recovered — those in the note's folder whose
`deleted_at` equals the note's old tombstone ts (`== Some(deleted_at)`). An attachment deleted
directly on its own carries a different `deleted_at` and survives the restore. Like the stamp, it
walks only the note's own `resources/` folder and rewrites only `deleted_at` of the
`StoredResource` (preserving `blob_hash`), without a `vv`/`last_writer` bump or `append_log`.

**Dependencies** —
- `note_resource_ids` — the note's attachment ids; expects a missing folder to yield an empty list.
- `resource_meta_path`, `read_sidecar`, `write_sidecar` — same sidecar IO as the stamp.
- `StoredResource.resource.deleted_at == Some(deleted_at)` — the exact-timestamp match; expects
  the caller to pass the note's prior tombstone ts (from `merge_note` before the revival).

**Used by** — `update_note` (local revive) and `apply_change(NoteCreate|NoteUpdate)` (sync).

**Repeated context** — derived state, never journaled (see `cascade_stamp_resources`).

---

## impl ResourceRepository for FsBackend

**Identification** — marker `// md:impl ResourceRepository for FsBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_resource, fn read_resource, fn delete_resource, fn list_resources, fn list_resources_for_note, fn purge_deleted_resources.

**What it does** — attachments as `{hash}.knrs` blobs + `{id}.meta.ndjson`
sidecars under `notes/{note_id}/resources/` (issue #127).

---

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
        let hash = content_hash(&data);
        tokio::fs::create_dir_all(self.note_resources_dir(resource.note_id)).await?;
        resource.vv = self.next_resource_vv(resource.id).await?;
        resource.last_writer = self.device_id.clone();
        tokio::fs::write(self.resource_blob_path(resource.note_id, &hash), &data).await?;
        let stored = StoredResource {
            resource: resource.clone(),
            blob_hash: hash,
        };
        self.write_sidecar(
            &self.resource_meta_path(resource.note_id, resource.id),
            &stored,
        )
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

**What it does** — Hash the bytes, ensure the note's `resources/` folder, stamp
vv/writer, then write the **blob first, metadata last**: `read_resource` finds a
resource through its sidecar, so the `StoredResource` write is the commit
marker — a crash between the two leaves an orphan `{hash}.knrs` (harmless;
overwritten identically on retry, or reclaimed as unreferenced) rather than a
sidecar pointing at missing bytes. The sidecar records `blob_hash` so the blob
is locatable; the `"create"` log entry carries the wire `Resource` only (no hash,
no bytes). Identical content in the same note reuses the same `{hash}.knrs`.

---

### fn read_resource

**Identification** — marker
`// md:impl ResourceRepository for FsBackend > fn read_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for FsBackend > fn read_resource
    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let Some((note_id, stored)) = self.read_resource_sidecar(id).await? else {
            return Err(StorageError::NotFound(id.to_string()));
        };
        if stored.resource.deleted_at.is_some() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let data = tokio::fs::read(self.resource_blob_path(note_id, &stored.blob_hash)).await?;
        Ok((stored.resource, data))
    }
```

**What it does** — `NotFound` when no sidecar exists or when it is tombstoned
(the tombstone is kept for sync); else the wire `Resource` plus the bytes read
from the `{hash}.knrs` blob its sidecar's `blob_hash` names.

---

### fn delete_resource

**Identification** — marker
`// md:impl ResourceRepository for FsBackend > fn delete_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for FsBackend > fn delete_resource
    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        let Some((note_id, mut stored)) = self.read_resource_sidecar(id).await? else {
            return Err(StorageError::NotFound(id.to_string()));
        };
        let ts = now();
        stored.resource.deleted_at = Some(ts);
        note_log::increment(&mut stored.resource.vv, &self.device_id);
        stored.resource.last_writer = self.device_id.clone();
        self.write_sidecar(&self.resource_meta_path(note_id, id), &stored)
            .await?;
        self.append_log(
            "resource",
            id,
            "delete",
            fs_tombstone_value(ts, &stored.resource.vv, &stored.resource.last_writer),
        )
        .await?;
        tracing::info!(%id, "Resource deleted");
        Ok(())
    }
```

**What it does** — Soft delete: locate the sidecar, stamp its tombstone + bump vv
(the `blob_hash` is preserved), leaving the `{hash}.knrs` bytes in place for later
`purge_deleted_resources`; append a `"delete"` entry. `NotFound` if no sidecar
carries that id.

---

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
        for note_id in self.all_note_ids().await? {
            for id in self.note_resource_ids(note_id).await? {
                let meta_path = self.resource_meta_path(note_id, id);
                match self.read_sidecar::<StoredResource>(&meta_path, id).await {
                    Ok(s) if s.resource.deleted_at.is_none() => resources.push(s.resource),
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

**What it does** — Walk every note's `resources/` folder, keep the live decodable
`StoredResource` sidecars' wire `Resource`, sort, `paginate` (metadata only, no
`{hash}.knrs` bytes).

---

### fn list_resources_for_note

**Identification** — marker
`// md:impl ResourceRepository for FsBackend > fn list_resources_for_note`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for FsBackend > fn list_resources_for_note
    async fn list_resources_for_note(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let mut resources = Vec::new();
        for id in self.note_resource_ids(note_id).await? {
            let meta_path = self.resource_meta_path(note_id, id);
            match self.read_sidecar::<StoredResource>(&meta_path, id).await {
                Ok(s) if s.resource.deleted_at.is_none() => resources.push(s.resource),
                Ok(_) => {}
                Err(e) => tracing::warn!("Could not load resource {id}: {e}"),
            }
        }
        resources.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(paginate(resources, limit, page_token.as_deref(), |r| {
            (r.created_at.to_sortable_rfc3339(), r.id)
        }))
    }
```

**What it does** — Native override of the trait's `list_resources_for_note` (issue #125). With
the per-note layout the note filter is the directory itself: it scans only
`notes/{note_id}/resources/` (`note_resource_ids`), so there is no cross-note walk and no
`r.note_id == note_id` test to apply — every sidecar there belongs to the note. A user-note
query never touches `SYSTEM_RESOURCE_NOTE_ID`'s folder, so system resources stay out of per-note
listings. Live sidecars are sorted by `(created_at, id)` and paginated.

**Dependencies** —
- `note_resource_ids` — the note's attachment ids; expects a missing folder to yield an empty list.
- `resource_meta_path`, `read_sidecar`, `paginate`, `super::effective_page_size` — the same
  machinery as `list_resources`; expect the `(sortable-created_at, id)` cursor.

**Used by** — the daemon's `list_resources` RPC / REST handler when a `note_id` filter is
present; tests.

---

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
        for note_id in self.all_note_ids().await? {
            let mut stored_list = Vec::new();
            for id in self.note_resource_ids(note_id).await? {
                match self
                    .read_sidecar::<StoredResource>(&self.resource_meta_path(note_id, id), id)
                    .await
                {
                    Ok(s) => stored_list.push(s),
                    Err(StorageError::NotFound(_)) => {}
                    Err(e) => {
                        tracing::warn!(
                            "Skipping resource {id} during purge (unreadable meta): {e}"
                        );
                    }
                }
            }
            let live_hashes: std::collections::HashSet<&str> = stored_list
                .iter()
                .filter(|s| s.resource.deleted_at.is_none())
                .map(|s| s.blob_hash.as_str())
                .collect();
            for stored in &stored_list {
                let Some(deleted_at) = stored.resource.deleted_at else {
                    continue;
                };
                if deleted_at >= older_than {
                    continue;
                }
                if stored.blob_hash.is_empty() || live_hashes.contains(stored.blob_hash.as_str()) {
                    continue;
                }
                match tokio::fs::remove_file(self.resource_blob_path(note_id, &stored.blob_hash))
                    .await
                {
                    Ok(()) => purged += 1,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        if purged > 0 {
            tracing::info!(purged, "Reclaimed payloads of soft-deleted resources");
        }
        Ok(purged)
    }
```

**What it does** — Reclaims `{hash}.knrs` blobs, one note at a time. Per note it
loads every `StoredResource`, builds the set of hashes still referenced by a
**live** resource, then for each tombstone older than the cutoff removes its blob
**only if** no live sibling shares that hash (content-dedup reference counting)
and the hash is non-empty. Live resources, not-yet-old tombstones, crashed-create
orphans and unreadable metas conservatively keep their bytes. Removing a shared
blob when a live reference exists is skipped, so the surviving attachment still
reads. Each blob removal replicates as a Syncthing deletion — safe: every peer
converges on the same tombstones, and a late concurrent revive rewrites the file.
Tombstone sidecars always survive; the count returned is blobs physically freed.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2; CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the same ignored `graphify-out/` layout locally.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `content-addressed attachment storage` — defined or implemented in this focused filesystem module (INFERRED)

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
| 2 | `fn content_hash` | `// md:fn content_hash` |
| 3 | `StoredResource` | `// md:StoredResource` |
| 4 | `impl FsBackend` (container) | `// md:impl FsBackend` |
| 5 | `fn note_resources_dir` | `// md:impl FsBackend > fn note_resources_dir` |
| 6 | `fn resource_meta_path` | `// md:impl FsBackend > fn resource_meta_path` |
| 7 | `fn resource_blob_path` | `// md:impl FsBackend > fn resource_blob_path` |
| 8 | `fn all_note_ids` | `// md:impl FsBackend > fn all_note_ids` |
| 9 | `fn note_resource_ids` | `// md:impl FsBackend > fn note_resource_ids` |
| 10 | `fn locate_resource_note` | `// md:impl FsBackend > fn locate_resource_note` |
| 11 | `fn read_resource_sidecar` | `// md:impl FsBackend > fn read_resource_sidecar` |
| 12 | `fn read_resource_meta` | `// md:impl FsBackend > fn read_resource_meta` |
| 13 | `fn next_resource_vv` | `// md:impl FsBackend > fn next_resource_vv` |
| 14 | `fn resource_incoming_wins` | `// md:impl FsBackend > fn resource_incoming_wins` |
| 15 | `fn cascade_stamp_resources` | `// md:impl FsBackend > fn cascade_stamp_resources` |
| 16 | `fn cascade_unstamp_resources` | `// md:impl FsBackend > fn cascade_unstamp_resources` |
| 17 | `impl ResourceRepository for FsBackend` (container) | `// md:impl ResourceRepository for FsBackend` |
| 18 | `fn create_resource` | `// md:impl ResourceRepository for FsBackend > fn create_resource` |
| 19 | `fn read_resource` | `// md:impl ResourceRepository for FsBackend > fn read_resource` |
| 20 | `fn delete_resource` | `// md:impl ResourceRepository for FsBackend > fn delete_resource` |
| 21 | `fn list_resources` | `// md:impl ResourceRepository for FsBackend > fn list_resources` |
| 22 | `fn list_resources_for_note` | `// md:impl ResourceRepository for FsBackend > fn list_resources_for_note` |
| 23 | `fn purge_deleted_resources` | `// md:impl ResourceRepository for FsBackend > fn purge_deleted_resources` |
