# `storage/fs/notebooks.rs` — NotebookRepository filesystem sidecars

Self-contained companion for `keeplin-core/src/storage/fs/notebooks.rs`. It documents every source block in source order with complete code embedded for every leaf.

**How to navigate**: each source marker `// md:<Header> > …` maps to the matching header chain below.

---

## Overview

**Identification** — file-level module declarations and imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::path::PathBuf;

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{now, Notebook};
use crate::storage::{NotebookRepository, SortableRfc3339};

use super::convert::fs_tombstone_value;
use super::pagination::paginate;
use super::FsBackend;
```

**What it does** — Owns NotebookRepository filesystem sidecars. This is a structural relocation from the former monolithic filesystem module; storage behavior, on-disk format version 8, serialization shapes, conflict resolution, and public `storage::fs::FsBackend` API are unchanged.

**Dependencies** — every binding above is either a crate symbol used directly by the blocks below or a sibling item exposed as `pub(super)`; expects: those symbols to preserve the signatures and behavior the relocated bodies already relied on, with compile-time failure on drift.

**Used by** — sibling modules under `storage/fs/` and callers of `crate::storage::fs::FsBackend`.

**Repeated context** — `FsBackend::FORMAT_VERSION` remains 8. Rust makes `mod.rs` fields visible to descendant modules; sibling-defined helpers used across files carry only `pub(super)`.

---

## impl FsBackend

**Identification** — the first inherent impl; marker `// md:impl FsBackend`.
Constructor, sweeps, format versioning, path helpers, log machinery,
sidecar/association/resource versioning, the note merge pipeline. Contains the
constants `FORMAT_VERSION = 8`, `NOTE_LOG_COMPACT_THRESHOLD = 256`,
`GLOBAL_LOG_COMPACT_THRESHOLD = 512`, `GLOBAL_LOG_SOFT_BYTES = 64 KiB`
(documented with the methods that use them).

**Code** — container: members documented as sub-blocks below: fn notebook_path.

---

### fn notebook_path

**Identification** — marker `// md:impl FsBackend > fn notebook_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn notebook_path
    pub(super) fn notebook_path(&self, id: Uuid) -> PathBuf {
        self.root.join("notebooks").join(format!("{id}.ndjson"))
    }
```

**What it does** — `{root}/notebooks/{id}.ndjson`.

---

## impl NotebookRepository for FsBackend

**Identification** — marker `// md:impl NotebookRepository for FsBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_notebook, fn read_notebook, fn update_notebook, fn delete_notebook, fn list_notebooks.

**What it does** — sidecar CRUD + global-log journaling.

---

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

---

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

---

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

---

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

---

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
            if let Some(stem) = fname.strip_suffix(".ndjson") {
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

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2; CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the same ignored `graphify-out/` layout locally.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `NotebookRepository filesystem sidecars` — defined or implemented in this focused filesystem module (INFERRED)

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
| 2 | `impl FsBackend` (container) | `// md:impl FsBackend` |
| 3 | `fn notebook_path` | `// md:impl FsBackend > fn notebook_path` |
| 4 | `impl NotebookRepository for FsBackend` (container) | `// md:impl NotebookRepository for FsBackend` |
| 5 | `fn create_notebook` | `// md:impl NotebookRepository for FsBackend > fn create_notebook` |
| 6 | `fn read_notebook` | `// md:impl NotebookRepository for FsBackend > fn read_notebook` |
| 7 | `fn update_notebook` | `// md:impl NotebookRepository for FsBackend > fn update_notebook` |
| 8 | `fn delete_notebook` | `// md:impl NotebookRepository for FsBackend > fn delete_notebook` |
| 9 | `fn list_notebooks` | `// md:impl NotebookRepository for FsBackend > fn list_notebooks` |
