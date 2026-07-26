# `storage/db/tags.rs` — TagRepository over LibSQL

Self-contained companion for `keeplin-core/src/storage/db/tags.rs`. It documents **every
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

**Identification** — file-level block: the imports the `TagRepository` implementation needs. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview

use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{now, NoteTag, Tag},
};

use crate::storage::{SortableRfc3339, TagRepository};

use super::convert::{assoc_data, build_page, parse_cursor, tombstone_data, vv_to_json};
use super::DbBackend;
```

**What it does** — The `TagRepository` implementation for `DbBackend`, covering tags and note/tag associations.

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

## impl TagRepository for DbBackend

**Identification** — marker `// md:impl TagRepository for DbBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_tag, fn read_tag, fn update_tag, fn delete_tag, fn list_tags, fn add_note_tag, fn remove_note_tag, fn list_note_tags.

**What it does** — tags + versioned associations.

---

### fn create_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn create_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for DbBackend > fn create_tag
    async fn create_tag(&self, mut tag: Tag) -> Result<Tag, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        tag.vv = self.next_local_vv("tags", &tag.id.to_string()).await?;
        tag.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            self.conn
                .execute(
                    "INSERT INTO tags (id,title,created_at,updated_at,deleted_at,vv,last_writer,system)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    libsql::params![
                        tag.id.to_string(),
                        tag.title.clone(),
                        tag.created_at.to_sortable_rfc3339(),
                        tag.updated_at.to_sortable_rfc3339(),
                        tag.deleted_at.map(|d| d.to_sortable_rfc3339()),
                        vv_to_json(&tag.vv),
                        tag.last_writer.clone(),
                        tag.system as i64,
                    ],
                )
                .await?;
            let data = serde_json::to_value(&tag).ok().map(|v| v.to_string());
            self.record_change("tag", &tag.id.to_string(), "create", data)
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %tag.id, "Tag created");
                Ok(tag)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }
```

**What it does** — Stamped `INSERT` + journal row. `system` is persisted as `?8`
(`tag.system as i64`); the journal `data` is the full serde JSON of the tag, so `system`
travels in the change feed too.

---

### fn read_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn read_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for DbBackend > fn read_tag
    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at,vv,last_writer,system
                 FROM tags WHERE id = ?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Self::row_to_tag(&row),
            None => Err(StorageError::NotFound(id.to_string())),
        }
    }
```

**What it does** — Single-row SELECT; `NotFound` when absent.

---

### fn update_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn update_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for DbBackend > fn update_tag
    async fn update_tag(&self, mut tag: Tag) -> Result<Tag, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        tag.vv = self.next_local_vv("tags", &tag.id.to_string()).await?;
        tag.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let affected = self
                .conn
                .execute(
                    "UPDATE tags SET title=?2,updated_at=?3,deleted_at=?4,vv=?5,last_writer=?6,system=?7 WHERE id=?1",
                    libsql::params![
                        tag.id.to_string(),
                        tag.title.clone(),
                        tag.updated_at.to_sortable_rfc3339(),
                        tag.deleted_at.map(|d| d.to_sortable_rfc3339()),
                        vv_to_json(&tag.vv),
                        tag.last_writer.clone(),
                        tag.system as i64,
                    ],
                )
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(tag.id.to_string()));
            }
            let data = serde_json::to_value(&tag).ok().map(|v| v.to_string());
            self.record_change("tag", &tag.id.to_string(), "update", data)
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %tag.id, "Tag updated");
                Ok(tag)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }
```

**What it does** — Stamped `UPDATE` (0 rows → `NotFound`) + journal row.

---

### fn delete_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn delete_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for DbBackend > fn delete_tag
    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let vv = self.next_local_vv("tags", &id.to_string()).await?;
        let writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let ts = now();
            let affected = self
                .conn
                .execute(
                    "UPDATE tags SET deleted_at=?2, updated_at=?2, vv=?3, last_writer=?4 WHERE id=?1",
                    libsql::params![id.to_string(), ts.to_sortable_rfc3339(), vv_to_json(&vv), writer.clone()],
                )
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(id.to_string()));
            }
            self.record_change("tag", &id.to_string(), "delete", Some(tombstone_data(ts, &vv, &writer)))
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(%id, "Tag deleted");
                Ok(())
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }
```

**What it does** — Soft delete + tombstone journal row.

---

### fn list_tags

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn list_tags`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for DbBackend > fn list_tags
    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at,vv,last_writer,system
                 FROM tags
                 WHERE deleted_at IS NULL
                   AND (
                     ?1 = '' OR created_at > ?2
                     OR (created_at = ?2 AND id > ?3)
                   )
                 ORDER BY created_at ASC, id ASC
                 LIMIT ?4",
                libsql::params![cursor_ts.clone(), cursor_ts, cursor_id, limit + 1],
            )
            .await?;
        let mut tags = Vec::new();
        while let Some(row) = rows.next().await? {
            tags.push(Self::row_to_tag(&row)?);
        }
        Ok(build_page(tags, limit as usize, |t| {
            format!("{}|{}", t.created_at.to_sortable_rfc3339(), t.id)
        }))
    }
```

**What it does** — Live tags, `(created_at, id)` keyset.

---

### fn add_note_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn add_note_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for DbBackend > fn add_note_tag
    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let note_id = note_tag.note_id.to_string();
        let tag_id = note_tag.tag_id.to_string();
        let vv = self.next_assoc_vv(&note_id, &tag_id).await?;
        let writer = self.device_id.clone();
        let ts = now();
        let r: Result<(), StorageError> = async {
            if !self.row_is_live("notes", &note_id).await? {
                return Err(StorageError::NotFound(note_id.clone()));
            }
            if !self.row_is_live("tags", &tag_id).await? {
                return Err(StorageError::NotFound(tag_id.clone()));
            }
            self.upsert_assoc(&note_id, &tag_id, ts, None, &vv, &writer)
                .await?;
            let data = assoc_data(note_tag.tag_id, ts, &vv, &writer);
            self.record_change("note_tag", &note_id, "add", Some(data))
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                Ok(())
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }
```

**What it does** — Verifies **both ends are live** (`row_is_live`; `NotFound`
otherwise — the API must not create dangling associations; `apply_change`
deliberately skips this because sync delivery order is not guaranteed), then
`upsert_assoc` with `deleted_at = NULL` (the present state, versioned so a
concurrent add-vs-remove converges) + an `"add"` journal row. Idempotent.

---

### fn remove_note_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn remove_note_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for DbBackend > fn remove_note_tag
    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let note_id_s = note_id.to_string();
        let tag_id_s = tag_id.to_string();
        let vv = self.next_assoc_vv(&note_id_s, &tag_id_s).await?;
        let writer = self.device_id.clone();
        let ts = now();
        let r: Result<(), StorageError> = async {
            self.upsert_assoc(&note_id_s, &tag_id_s, ts, Some(ts), &vv, &writer)
                .await?;
            let data = assoc_data(tag_id, ts, &vv, &writer);
            self.record_change("note_tag", &note_id_s, "remove", Some(data))
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                Ok(())
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }
```

**What it does** — `upsert_assoc` with a tombstone (kept so it can beat a
concurrent add) + a `"remove"` journal row. Idempotent.

---

### fn list_note_tags

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn list_note_tags`.

**Code** — complete and verbatim:

```rust
    // md:impl TagRepository for DbBackend > fn list_note_tags
    async fn list_note_tags(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT t.id,t.title,t.created_at,t.updated_at,t.deleted_at,t.vv,t.last_writer,t.system
                 FROM tags t
                 JOIN note_tags nt ON t.id = nt.tag_id
                 WHERE nt.note_id = ?1 AND nt.deleted_at IS NULL AND t.deleted_at IS NULL
                   AND (
                     ?2 = '' OR t.created_at > ?3
                     OR (t.created_at = ?3 AND t.id > ?4)
                   )
                 ORDER BY t.created_at ASC, t.id ASC
                 LIMIT ?5",
                libsql::params![
                    note_id.to_string(),
                    cursor_ts.clone(),
                    cursor_ts,
                    cursor_id,
                    limit + 1
                ],
            )
            .await?;
        let mut tags = Vec::new();
        while let Some(row) = rows.next().await? {
            tags.push(Self::row_to_tag(&row)?);
        }
        Ok(build_page(tags, limit as usize, |t| {
            format!("{}|{}", t.created_at.to_sortable_rfc3339(), t.id)
        }))
    }
```

**What it does** — Tags joined through live (`nt.deleted_at IS NULL`)
associations, `(created_at, id)` keyset.

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
| 2 | impl TagRepository for DbBackend | `// md:impl TagRepository for DbBackend` |
| 3 | fn create_tag | `// md:impl TagRepository for DbBackend > fn create_tag` |
| 4 | fn read_tag | `// md:impl TagRepository for DbBackend > fn read_tag` |
| 5 | fn update_tag | `// md:impl TagRepository for DbBackend > fn update_tag` |
| 6 | fn delete_tag | `// md:impl TagRepository for DbBackend > fn delete_tag` |
| 7 | fn list_tags | `// md:impl TagRepository for DbBackend > fn list_tags` |
| 8 | fn add_note_tag | `// md:impl TagRepository for DbBackend > fn add_note_tag` |
| 9 | fn remove_note_tag | `// md:impl TagRepository for DbBackend > fn remove_note_tag` |
| 10 | fn list_note_tags | `// md:impl TagRepository for DbBackend > fn list_note_tags` |
