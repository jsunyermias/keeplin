# `storage/db/notes.rs` — NoteRepository over LibSQL

Self-contained companion for `keeplin-core/src/storage/db/notes.rs`. It documents **every
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

**Identification** — file-level block: the imports the `NoteRepository` implementation needs. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview

use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{now, Note},
};

use crate::storage::{NoteRepository, SortableRfc3339};

use super::convert::{
    bookmarks_to_json, build_page, links_to_json, parse_cursor, tombstone_data, vv_to_json,
};
use super::DbBackend;
```

**What it does** — The `NoteRepository` implementation for `DbBackend`.

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

## impl NoteRepository for DbBackend

**Identification** — marker `// md:impl NoteRepository for DbBackend`; each
method carries `// md:impl NoteRepository for DbBackend > fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_note, fn read_note, fn update_note, fn delete_note, fn list_notes, fn note_backlinks, fn list_notes_in_notebook, fn list_starred_notes, fn notebook_sort_profile.

**What it does** — the note surface. Common write pattern: exclusive lock →
`begin` → stamp `vv = next_local_vv` + `last_writer = device_id` → primary
write (+ `refresh_note_links` for notes) → `record_change` with the full
snapshot → `commit` (or `rollback` on any error).

---

### fn create_note

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn create_note`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for DbBackend > fn create_note
    async fn create_note(&self, mut note: Note) -> Result<Note, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        note.vv = self.next_local_vv("notes", &note.id.to_string()).await?;
        note.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            self.conn
                .execute(
                    "INSERT INTO notes
                     (id, title, body, notebook_id, is_todo, todo_due, todo_completed, created_at, updated_at, deleted_at, alias, bookmarks, links, vv, last_writer, is_pinned, is_starred, sort_key)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
                    libsql::params![
                        note.id.to_string(),
                        note.title.clone(),
                        note.body.clone(),
                        note.notebook_id.to_string(),
                        note.is_todo as i64,
                        note.todo_due.map(|d| d.to_sortable_rfc3339()),
                        note.todo_completed.map(|d| d.to_sortable_rfc3339()),
                        note.created_at.to_sortable_rfc3339(),
                        note.updated_at.to_sortable_rfc3339(),
                        note.deleted_at.map(|d| d.to_sortable_rfc3339()),
                        note.alias.clone(),
                        bookmarks_to_json(&note.bookmarks),
                        links_to_json(&note.links),
                        vv_to_json(&note.vv),
                        note.last_writer.clone(),
                        note.is_pinned as i64,
                        note.is_starred as i64,
                        note.sort_key as i64,
                    ],
                )
                .await?;
            self.refresh_note_links(&note).await?;
            let data = serde_json::to_value(&note).ok().map(|v| v.to_string());
            self.record_change("note", &note.id.to_string(), "create", data).await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %note.id, "Note created");
                Ok(note)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }
```

**What it does** — Plain `INSERT` (an existing id errors — fresh-destination
contract), links projection refresh, `"create"` journal row with the full
snapshot; returns the stamped note.

---

### fn read_note

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn read_note`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for DbBackend > fn read_note
    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,body,notebook_id,is_todo,todo_due,todo_completed,
                        created_at,updated_at,deleted_at,alias,bookmarks,links,vv,last_writer,
                        is_pinned,is_starred,sort_key
                 FROM notes WHERE id = ?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Self::row_to_note(&row),
            None => Err(StorageError::NotFound(id.to_string())),
        }
    }
```

**What it does** — Single-row SELECT (18 columns); `NotFound` when absent.
Note: tombstoned rows **are** returned (needed for resolution and revival);
user-facing layers re-check `deleted_at`.

---

### fn update_note

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn update_note`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for DbBackend > fn update_note
    async fn update_note(&self, mut note: Note) -> Result<Note, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        note.vv = self.next_local_vv("notes", &note.id.to_string()).await?;
        note.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let prior_deleted = {
                let mut rows = self
                    .conn
                    .query(
                        "SELECT deleted_at FROM notes WHERE id = ?1",
                        [note.id.to_string()],
                    )
                    .await?;
                match rows.next().await? {
                    Some(row) => row.get::<Option<String>>(0)?,
                    None => None,
                }
            };
            let affected = self
                .conn
                .execute(
                    "UPDATE notes SET
                     title=?2, body=?3, notebook_id=?4, is_todo=?5, todo_due=?6,
                     todo_completed=?7, updated_at=?8, deleted_at=?9,
                     alias=?10, bookmarks=?11, links=?12, vv=?13, last_writer=?14,
                     is_pinned=?15, is_starred=?16, sort_key=?17
                     WHERE id = ?1",
                    libsql::params![
                        note.id.to_string(),
                        note.title.clone(),
                        note.body.clone(),
                        note.notebook_id.to_string(),
                        note.is_todo as i64,
                        note.todo_due.map(|d| d.to_sortable_rfc3339()),
                        note.todo_completed.map(|d| d.to_sortable_rfc3339()),
                        note.updated_at.to_sortable_rfc3339(),
                        note.deleted_at.map(|d| d.to_sortable_rfc3339()),
                        note.alias.clone(),
                        bookmarks_to_json(&note.bookmarks),
                        links_to_json(&note.links),
                        vv_to_json(&note.vv),
                        note.last_writer.clone(),
                        note.is_pinned as i64,
                        note.is_starred as i64,
                        note.sort_key as i64,
                    ],
                )
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(note.id.to_string()));
            }
            if note.deleted_at.is_none() {
                if let Some(old_ts) = prior_deleted {
                    self.conn
                        .execute(
                            "UPDATE resources SET deleted_at = NULL WHERE note_id = ?1 AND deleted_at = ?2",
                            libsql::params![note.id.to_string(), old_ts],
                        )
                        .await?;
                }
            }
            self.refresh_note_links(&note).await?;
            let data = serde_json::to_value(&note).ok().map(|v| v.to_string());
            self.record_change("note", &note.id.to_string(), "update", data)
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %note.id, "Note updated");
                Ok(note)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }
```

**What it does** — `UPDATE … WHERE id` (0 rows → `NotFound`), links refresh,
`"update"` journal row. **Restore cascade (issue #125):** `prior_deleted` is read
before the update; if this update makes the note live again (`note.deleted_at` is
`None`) and it was previously tombstoned, the resources the note dragged down —
those whose `deleted_at` equals the note's old tombstone ts — are un-stamped
(`deleted_at = NULL`). A resource deleted directly on its own has a different ts and
is left deleted.

---

### fn delete_note

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn delete_note`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for DbBackend > fn delete_note
    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let vv = self.next_local_vv("notes", &id.to_string()).await?;
        let writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let ts = now();
            let affected = self
                .conn
                .execute(
                    "UPDATE notes SET deleted_at = ?2, updated_at = ?2, vv = ?3, last_writer = ?4 WHERE id = ?1",
                    libsql::params![id.to_string(), ts.to_sortable_rfc3339(), vv_to_json(&vv), writer.clone()],
                )
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(id.to_string()));
            }
            self.conn
                .execute(
                    "UPDATE resources SET deleted_at = ?2 WHERE note_id = ?1 AND deleted_at IS NULL",
                    libsql::params![id.to_string(), ts.to_sortable_rfc3339()],
                )
                .await?;
            self.record_change("note", &id.to_string(), "delete", Some(tombstone_data(ts, &vv, &writer)))
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(%id, "Note deleted");
                Ok(())
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }
```

**What it does** — Soft delete: `deleted_at = updated_at = now`, bumped vv,
`"delete"` journal row with `tombstone_data`. **Delete cascade (issue #125):** in the
same transaction, every live resource with this `note_id` is stamped `deleted_at = ts`
(the note's tombstone ts), so attachments follow their note into the tombstone.

---

### fn list_notes

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn list_notes`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for DbBackend > fn list_notes
    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,body,notebook_id,is_todo,todo_due,todo_completed,
                        created_at,updated_at,deleted_at,alias,bookmarks,links,vv,last_writer,
                        is_pinned,is_starred,sort_key
                 FROM notes
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
        let mut notes = Vec::new();
        while let Some(row) = rows.next().await? {
            notes.push(Self::row_to_note(&row)?);
        }
        Ok(build_page(notes, limit as usize, |n| {
            format!("{}|{}", n.created_at.to_sortable_rfc3339(), n.id)
        }))
    }
```

**What it does** — Live notes in `(created_at, id)` order with the
`"<ts>|<id>"` keyset cursor, `LIMIT limit + 1` + `build_page`.

---

### fn note_backlinks

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn note_backlinks`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for DbBackend > fn note_backlinks
    async fn note_backlinks(
        &self,
        target_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT n.id,n.title,n.body,n.notebook_id,n.is_todo,n.todo_due,n.todo_completed,
                        n.created_at,n.updated_at,n.deleted_at,n.alias,n.bookmarks,n.links,n.vv,n.last_writer,
                        n.is_pinned,n.is_starred,n.sort_key
                 FROM note_links nl
                 JOIN notes n ON n.id = nl.source_note_id
                 WHERE nl.target_note_id = ?1 AND n.deleted_at IS NULL
                   AND (
                     ?2 = '' OR n.created_at > ?3
                     OR (n.created_at = ?3 AND n.id > ?4)
                   )
                 ORDER BY n.created_at ASC, n.id ASC
                 LIMIT ?5",
                libsql::params![
                    target_id.to_string(),
                    cursor_ts.clone(),
                    cursor_ts,
                    cursor_id,
                    limit + 1
                ],
            )
            .await?;
        let mut notes = Vec::new();
        while let Some(row) = rows.next().await? {
            notes.push(Self::row_to_note(&row)?);
        }
        Ok(build_page(notes, limit as usize, |n| {
            format!("{}|{}", n.created_at.to_sortable_rfc3339(), n.id)
        }))
    }
```

**What it does** — The indexed override of the trait default: `note_links`
joined back to live notes (`idx_note_links_target` makes the WHERE an index
seek), keyset cursor on `(created_at, id)`.

---

### fn list_notes_in_notebook

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn list_notes_in_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for DbBackend > fn list_notes_in_notebook
    async fn list_notes_in_notebook(
        &self,
        notebook_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_key, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,body,notebook_id,is_todo,todo_due,todo_completed,
                        created_at,updated_at,deleted_at,alias,bookmarks,links,vv,last_writer,
                        is_pinned,is_starred,sort_key
                 FROM notes
                 WHERE notebook_id = ?1 AND deleted_at IS NULL
                   AND (
                     ?2 = ''
                     OR (CASE WHEN sort_key = 0 THEN 1000 ELSE sort_key END) > CAST(?2 AS INTEGER)
                     OR ((CASE WHEN sort_key = 0 THEN 1000 ELSE sort_key END) = CAST(?2 AS INTEGER)
                         AND id > ?3)
                   )
                 ORDER BY (CASE WHEN sort_key = 0 THEN 1000 ELSE sort_key END) ASC, id ASC
                 LIMIT ?4",
                libsql::params![notebook_id.to_string(), cursor_key, cursor_id, limit + 1],
            )
            .await?;
        let mut notes = Vec::new();
        while let Some(row) = rows.next().await? {
            notes.push(Self::row_to_note(&row)?);
        }
        Ok(build_page(notes, limit as usize, |n| {
            format!("{}|{}", n.effective_sort_key(), n.id)
        }))
    }
```

**What it does** — One notebook's live notes ordered by the **effective** sort
key — the SQL `CASE WHEN sort_key = 0 THEN 1000 …` mirrors
`Note::effective_sort_key`, and the cursor carries the effective key compared
numerically (`CAST`).

---

### fn list_starred_notes

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn list_starred_notes`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for DbBackend > fn list_starred_notes
    async fn list_starred_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,body,notebook_id,is_todo,todo_due,todo_completed,
                        created_at,updated_at,deleted_at,alias,bookmarks,links,vv,last_writer,
                        is_pinned,is_starred,sort_key
                 FROM notes
                 WHERE is_starred = 1 AND deleted_at IS NULL
                   AND (
                     ?1 = '' OR created_at > ?2
                     OR (created_at = ?2 AND id > ?3)
                   )
                 ORDER BY created_at ASC, id ASC
                 LIMIT ?4",
                libsql::params![cursor_ts.clone(), cursor_ts, cursor_id, limit + 1],
            )
            .await?;
        let mut notes = Vec::new();
        while let Some(row) = rows.next().await? {
            notes.push(Self::row_to_note(&row)?);
        }
        Ok(build_page(notes, limit as usize, |n| {
            format!("{}|{}", n.created_at.to_sortable_rfc3339(), n.id)
        }))
    }
```

**What it does** — `is_starred = 1` live notes, `(created_at, id)` keyset.

---

### fn notebook_sort_profile

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn notebook_sort_profile`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteRepository for DbBackend > fn notebook_sort_profile
    async fn notebook_sort_profile(
        &self,
        notebook_id: Uuid,
    ) -> Result<super::NotebookSortProfile, StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT sort_key FROM notes WHERE notebook_id = ?1 AND deleted_at IS NULL",
                [notebook_id.to_string()],
            )
            .await?;
        let mut keys = Vec::new();
        while let Some(row) = rows.next().await? {
            let raw = row.get::<i64>(0)?.max(0) as u32;
            keys.push(if raw == 0 {
                Note::DEFAULT_SORT_KEY
            } else {
                raw
            });
        }
        Ok(super::NotebookSortProfile::from_effective_keys(keys))
    }
```

**What it does** — Keys-only scan (`idx_notes_notebook_sort`) mapped through
the 0→`DEFAULT_SORT_KEY` sentinel into
`NotebookSortProfile::from_effective_keys`.

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
| 2 | impl NoteRepository for DbBackend | `// md:impl NoteRepository for DbBackend` |
| 3 | fn create_note | `// md:impl NoteRepository for DbBackend > fn create_note` |
| 4 | fn read_note | `// md:impl NoteRepository for DbBackend > fn read_note` |
| 5 | fn update_note | `// md:impl NoteRepository for DbBackend > fn update_note` |
| 6 | fn delete_note | `// md:impl NoteRepository for DbBackend > fn delete_note` |
| 7 | fn list_notes | `// md:impl NoteRepository for DbBackend > fn list_notes` |
| 8 | fn note_backlinks | `// md:impl NoteRepository for DbBackend > fn note_backlinks` |
| 9 | fn list_notes_in_notebook | `// md:impl NoteRepository for DbBackend > fn list_notes_in_notebook` |
| 10 | fn list_starred_notes | `// md:impl NoteRepository for DbBackend > fn list_starred_notes` |
| 11 | fn notebook_sort_profile | `// md:impl NoteRepository for DbBackend > fn notebook_sort_profile` |
