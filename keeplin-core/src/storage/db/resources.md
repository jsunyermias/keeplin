# `storage/db/resources.rs` — ResourceRepository over LibSQL

Self-contained companion for `keeplin-core/src/storage/db/resources.rs`. It documents **every
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

**Identification** — file-level block: the imports the `ResourceRepository` implementation needs. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{now, Resource},
};

use crate::storage::{ResourceRepository, SortableRfc3339};

use super::convert::{build_page, parse_cursor, tombstone_data, vv_to_json};
use super::DbBackend;
```

**What it does** — The `ResourceRepository` implementation for `DbBackend`.

**Dependencies** — every binding above is either a crate this block's siblings call directly or a
path relocated from the pre-split `storage/db.rs`; expects: the symbols to keep the
signatures the block bodies below already rely on, since a changed signature fails to
compile rather than degrading silently.

**Used by** — the sibling modules of this directory module, and `crate::storage::db` through
`mod.rs`.

**Repeated context** — the directory module keeps `DbBackend`'s fields private in `mod.rs`;
Rust makes them visible to every descendant module, so siblings read them without any
widening. Items defined in one sibling and used by another carry `pub(super)`.

---

## impl ResourceRepository for DbBackend

**Identification** — marker `// md:impl ResourceRepository for DbBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_resource, fn read_resource, fn delete_resource, fn list_resources, fn purge_deleted_resources.

**What it does** — resources with BLOB payloads.

---

### fn create_resource

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn create_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for DbBackend > fn create_resource
    async fn create_resource(
        &self,
        mut resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        resource.vv = self.next_resource_vv(&resource.id.to_string()).await?;
        resource.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let data_b64 = STANDARD.encode(&data);
            self.conn
                .execute(
                    "INSERT INTO resources (id,title,mime_type,file_name,size,data,created_at,deleted_at,vv,last_writer,duration_ms,width,height,note_id)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                    libsql::params![
                        resource.id.to_string(),
                        resource.title.clone(),
                        resource.mime_type.clone(),
                        resource.file_name.clone(),
                        resource.size as i64,
                        data,
                        resource.created_at.to_sortable_rfc3339(),
                        resource.deleted_at.map(|d| d.to_sortable_rfc3339()),
                        vv_to_json(&resource.vv),
                        resource.last_writer.clone(),
                        resource.duration_ms.map(|d| d as i64),
                        resource.dimensions.map(|(w, _)| w as i64),
                        resource.dimensions.map(|(_, h)| h as i64),
                        resource.note_id.to_string(),
                    ],
                )
                .await?;
            let change_data = serde_json::to_value(&resource).ok().map(|mut v| {
                v["_data_b64"] = serde_json::Value::String(data_b64);
                v.to_string()
            });
            self.record_change("resource", &resource.id.to_string(), "create", change_data)
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %resource.id, "Resource created");
                Ok(resource)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }
```

**What it does** — Stamped `INSERT` storing the BLOB, plus a `"create"` journal
row whose JSON carries `_data_b64` (the Base64 payload) so peers receiving the
change via the relay reconstruct the full resource without a separate download.

---

### fn read_resource

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn read_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for DbBackend > fn read_resource
    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,mime_type,file_name,size,created_at,deleted_at,vv,last_writer,duration_ms,width,height,note_id,data
                 FROM resources WHERE id=?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            None => Err(StorageError::NotFound(id.to_string())),
            Some(row) => {
                let resource = Self::row_to_resource(&row)?;
                if resource.deleted_at.is_some() {
                    return Err(StorageError::NotFound(id.to_string()));
                }
                let blob: Vec<u8> = row.get(13)?;
                Ok((resource, blob))
            }
        }
    }
```

**What it does** — Metadata + BLOB; a tombstoned resource reads as `NotFound`
(before touching data).

---

### fn delete_resource

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn delete_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for DbBackend > fn delete_resource
    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let vv = self.next_resource_vv(&id.to_string()).await?;
        let writer = self.device_id.clone();
        let ts = now();
        let r: Result<(), StorageError> = async {
            let affected = self
                .conn
                .execute(
                    "UPDATE resources SET deleted_at=?2, vv=?3, last_writer=?4 WHERE id=?1",
                    libsql::params![
                        id.to_string(),
                        ts.to_sortable_rfc3339(),
                        vv_to_json(&vv),
                        writer.clone()
                    ],
                )
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(id.to_string()));
            }
            self.record_change(
                "resource",
                &id.to_string(),
                "delete",
                Some(tombstone_data(ts, &vv, &writer)),
            )
            .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(%id, "Resource deleted");
                Ok(())
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }
```

**What it does** — Soft delete: tombstone + bumped vector; the payload is
retained (reclaim is `purge_deleted_resources`); tombstone journal row.

---

### fn list_resources

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn list_resources`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for DbBackend > fn list_resources
    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,mime_type,file_name,size,created_at,deleted_at,vv,last_writer,duration_ms,width,height,note_id
                 FROM resources
                 WHERE deleted_at IS NULL
                   AND (?1 = '' OR created_at > ?2 OR (created_at = ?2 AND id > ?3))
                 ORDER BY created_at ASC, id ASC
                 LIMIT ?4",
                libsql::params![cursor_ts.clone(), cursor_ts, cursor_id, limit + 1],
            )
            .await?;
        let mut resources = Vec::new();
        while let Some(row) = rows.next().await? {
            resources.push(Self::row_to_resource(&row)?);
        }
        Ok(build_page(resources, limit as usize, |r| {
            format!("{}|{}", r.created_at.to_sortable_rfc3339(), r.id)
        }))
    }
```

**What it does** — Live metadata (no BLOBs), `(created_at, id)` keyset. The `SELECT` now carries
`note_id` (column 12) so `row_to_resource` fills `Resource.note_id`.

---

### fn list_resources_for_note

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn list_resources_for_note`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for DbBackend > fn list_resources_for_note
    async fn list_resources_for_note(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,mime_type,file_name,size,created_at,deleted_at,vv,last_writer,duration_ms,width,height,note_id
                 FROM resources
                 WHERE note_id = ?5 AND deleted_at IS NULL
                   AND (?1 = '' OR created_at > ?2 OR (created_at = ?2 AND id > ?3))
                 ORDER BY created_at ASC, id ASC
                 LIMIT ?4",
                libsql::params![cursor_ts.clone(), cursor_ts, cursor_id, limit + 1, note_id.to_string()],
            )
            .await?;
        let mut resources = Vec::new();
        while let Some(row) = rows.next().await? {
            resources.push(Self::row_to_resource(&row)?);
        }
        Ok(build_page(resources, limit as usize, |r| {
            format!("{}|{}", r.created_at.to_sortable_rfc3339(), r.id)
        }))
    }
```

**What it does** — Native override of the trait's `list_resources_for_note` (issue #125): the
live attachments of one note, `(created_at, id)` keyset, filtered in SQL by `note_id = ?5`
(backed by the `idx_resources_note` index from migration v5) rather than exhausting
`list_resources` and filtering in memory. A user-note query never matches
`SYSTEM_RESOURCE_NOTE_ID`, so system resources stay out of per-note listings.

**Dependencies** —
- `parse_cursor`, `build_page`, `super::effective_page_size` — same keyset/pagination machinery
  as `list_resources`; expect the `(created_at, id)` cursor format.
- `Self::row_to_resource` — row → `Resource` incl. `note_id`; expects the `SELECT` column order.
- the `idx_resources_note` index — expects `(note_id, created_at, id)` so the filter+order is
  index-served.

**Used by** — the daemon's `list_resources` RPC / REST handler when a `note_id` filter is
present; tests.

---

### fn purge_deleted_resources

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn purge_deleted_resources`.

**Code** — complete and verbatim:

```rust
    // md:impl ResourceRepository for DbBackend > fn purge_deleted_resources
    async fn purge_deleted_resources(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, StorageError> {
        let _write_guard = self.lock.write().await;
        let purged = self
            .conn
            .execute(
                "UPDATE resources SET data = NULL
                 WHERE deleted_at IS NOT NULL AND deleted_at < ?1 AND data IS NOT NULL",
                libsql::params![older_than.to_sortable_rfc3339()],
            )
            .await?;
        if purged > 0 {
            tracing::info!(purged, "Reclaimed payloads of soft-deleted resources");
        }
        Ok(purged)
    }
```

**What it does** — `UPDATE … SET data = NULL` for tombstones older than the
cutoff — frees the dead bytes but keeps the tombstone row (`deleted_at`, vv,
`last_writer`) so the deletion keeps converging; `size` remains as a record.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `DbBackend` — extended here with the blocks below (INFERRED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/db/mod.rs` — owns `DbBackend` and its fields (INFERRED)
- `keeplin-core/src/storage/db/convert.rs` — shared encoding helpers (INFERRED)
- `keeplin-core/src/storage/backend.rs` — the repository traits and shared types (INFERRED)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/storage/db/mod.rs` — declares this submodule (INFERRED)

**Invariants** (the rules this file must keep true — restated here even if stated
elsewhere)

- The split is a relocation: `storage::db::DbBackend` stays the public path, so no caller outside this directory module changes.

---

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | impl ResourceRepository for DbBackend | `// md:impl ResourceRepository for DbBackend` |
| 3 | fn create_resource | `// md:impl ResourceRepository for DbBackend > fn create_resource` |
| 4 | fn read_resource | `// md:impl ResourceRepository for DbBackend > fn read_resource` |
| 5 | fn delete_resource | `// md:impl ResourceRepository for DbBackend > fn delete_resource` |
| 6 | fn list_resources | `// md:impl ResourceRepository for DbBackend > fn list_resources` |
| 7 | fn list_resources_for_note | `// md:impl ResourceRepository for DbBackend > fn list_resources_for_note` |
| 8 | fn purge_deleted_resources | `// md:impl ResourceRepository for DbBackend > fn purge_deleted_resources` |
