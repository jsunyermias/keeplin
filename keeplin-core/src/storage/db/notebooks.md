# `storage/db/notebooks.rs` — NotebookRepository over LibSQL

Self-contained companion for `keeplin-core/src/storage/db/notebooks.rs`. It documents **every
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

**Identification** — file-level block: the imports the `NotebookRepository` implementation needs. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview

use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{now, Notebook},
};

use crate::storage::{NotebookRepository, SortableRfc3339};

use super::convert::{build_page, parse_cursor, tombstone_data, vv_to_json};
use super::DbBackend;
```

**What it does** — The `NotebookRepository` implementation for `DbBackend`.

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

## impl NotebookRepository for DbBackend

**Identification** — marker `// md:impl NotebookRepository for DbBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_notebook, fn read_notebook, fn update_notebook, fn delete_notebook, fn list_notebooks.

**What it does** — the notebook CRUD, same transactional write pattern.

---

### fn create_notebook

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn create_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl NotebookRepository for DbBackend > fn create_notebook
    async fn create_notebook(&self, mut notebook: Notebook) -> Result<Notebook, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        notebook.vv = self
            .next_local_vv("notebooks", &notebook.id.to_string())
            .await?;
        notebook.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            self.conn
                .execute(
                    "INSERT INTO notebooks (id,title,created_at,updated_at,deleted_at,alias,vv,last_writer)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    libsql::params![
                        notebook.id.to_string(),
                        notebook.title.clone(),
                        notebook.created_at.to_sortable_rfc3339(),
                        notebook.updated_at.to_sortable_rfc3339(),
                        notebook.deleted_at.map(|d| d.to_sortable_rfc3339()),
                        notebook.alias.clone(),
                        vv_to_json(&notebook.vv),
                        notebook.last_writer.clone(),
                    ],
                )
                .await?;
            let data = serde_json::to_value(&notebook).ok().map(|v| v.to_string());
            self.record_change("notebook", &notebook.id.to_string(), "create", data)
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %notebook.id, "Notebook created");
                Ok(notebook)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }
```

**What it does** — Stamped `INSERT` + `"create"` journal row.

---

### fn read_notebook

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn read_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl NotebookRepository for DbBackend > fn read_notebook
    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at,alias,vv,last_writer
                 FROM notebooks WHERE id = ?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Self::row_to_notebook(&row),
            None => Err(StorageError::NotFound(id.to_string())),
        }
    }
```

**What it does** — Single-row SELECT; `NotFound` when absent.

---

### fn update_notebook

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn update_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl NotebookRepository for DbBackend > fn update_notebook
    async fn update_notebook(&self, mut notebook: Notebook) -> Result<Notebook, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        notebook.vv = self
            .next_local_vv("notebooks", &notebook.id.to_string())
            .await?;
        notebook.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let affected = self
                .conn
                .execute(
                    "UPDATE notebooks SET title=?2,updated_at=?3,deleted_at=?4,alias=?5,vv=?6,last_writer=?7 WHERE id=?1",
                    libsql::params![
                        notebook.id.to_string(),
                        notebook.title.clone(),
                        notebook.updated_at.to_sortable_rfc3339(),
                        notebook.deleted_at.map(|d| d.to_sortable_rfc3339()),
                        notebook.alias.clone(),
                        vv_to_json(&notebook.vv),
                        notebook.last_writer.clone(),
                    ],
                )
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(notebook.id.to_string()));
            }
            let data = serde_json::to_value(&notebook).ok().map(|v| v.to_string());
            self.record_change("notebook", &notebook.id.to_string(), "update", data)
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %notebook.id, "Notebook updated");
                Ok(notebook)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }
```

**What it does** — Stamped `UPDATE` (0 rows → `NotFound`) + `"update"` row.

---

### fn delete_notebook

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn delete_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl NotebookRepository for DbBackend > fn delete_notebook
    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let vv = self.next_local_vv("notebooks", &id.to_string()).await?;
        let writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let ts = now();
            let affected = self
                .conn
                .execute(
                    "UPDATE notebooks SET deleted_at=?2, updated_at=?2, vv=?3, last_writer=?4 WHERE id=?1",
                    libsql::params![id.to_string(), ts.to_sortable_rfc3339(), vv_to_json(&vv), writer.clone()],
                )
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(id.to_string()));
            }
            self.record_change("notebook", &id.to_string(), "delete", Some(tombstone_data(ts, &vv, &writer)))
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(%id, "Notebook deleted");
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

### fn list_notebooks

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn list_notebooks`.

**Code** — complete and verbatim:

```rust
    // md:impl NotebookRepository for DbBackend > fn list_notebooks
    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at,alias,vv,last_writer
                 FROM notebooks
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
        let mut notebooks = Vec::new();
        while let Some(row) = rows.next().await? {
            notebooks.push(Self::row_to_notebook(&row)?);
        }
        Ok(build_page(notebooks, limit as usize, |nb| {
            format!("{}|{}", nb.created_at.to_sortable_rfc3339(), nb.id)
        }))
    }
```

**What it does** — Live notebooks, `(created_at, id)` keyset.

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
| 2 | impl NotebookRepository for DbBackend | `// md:impl NotebookRepository for DbBackend` |
| 3 | fn create_notebook | `// md:impl NotebookRepository for DbBackend > fn create_notebook` |
| 4 | fn read_notebook | `// md:impl NotebookRepository for DbBackend > fn read_notebook` |
| 5 | fn update_notebook | `// md:impl NotebookRepository for DbBackend > fn update_notebook` |
| 6 | fn delete_notebook | `// md:impl NotebookRepository for DbBackend > fn delete_notebook` |
| 7 | fn list_notebooks | `// md:impl NotebookRepository for DbBackend > fn list_notebooks` |
