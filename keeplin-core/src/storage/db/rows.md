# `storage/db/rows.rs` — libsql row -> domain model decoding

Self-contained companion for `keeplin-core/src/storage/db/rows.rs`. It documents **every
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

**Identification** — file-level block: the imports the row decoders need. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview

use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Change, Note, Notebook, Resource, Tag},
};

use super::convert::{
    assoc_from_data, json_to_bookmarks, json_to_links, json_to_vv, tombstone_from_data,
};
use super::DbBackend;
```

**What it does** — Decoding of `libsql::Row` values into domain models. Every `row_to_*` here is the single place a stored row becomes a `Note`, `Notebook`, `Tag`, `Resource` or `Change`, so column order and null handling are decided once.

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

## impl DbBackend (row mapping)

**Identification** — the first inherent impl; marker `// md:impl DbBackend (row mapping)`.
Constructor, migrations, journal/WS/row/versioning helpers.

**Code** — container: members documented as sub-blocks below: fn new, fn run_migrations, fn schema_version, fn apply_migration, fn migrate_v1_baseline, fn add_column_if_missing, fn get_or_create_device_id, fn record_change, fn refresh_note_links, fn connect_ws, fn row_to_note, fn parse_uuid, fn parse_required_dt, fn parse_optional_dt, fn row_to_notebook, fn row_to_tag, fn row_to_resource, fn row_to_change, fn begin, fn commit, fn rollback, fn ensure_ws, fn migrate_v2_ordering, fn current_meta, fn incoming_wins, fn next_local_vv, fn row_is_live, fn assoc_meta, fn next_assoc_vv, fn assoc_incoming_wins, fn upsert_assoc, fn resource_meta, fn next_resource_vv, fn resource_incoming_wins.

---

### fn row_to_note

**Identification** — marker `// md:impl DbBackend (row mapping) > fn row_to_note`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (row mapping) > fn row_to_note
    pub(super) fn row_to_note(row: &libsql::Row) -> Result<Note, StorageError> {
        let id = Self::parse_uuid(row.get::<String>(0)?)?;
        let title: String = row.get(1)?;
        let body: String = row.get(2)?;
        let notebook_id: Uuid = row
            .get::<Option<String>>(3)?
            .map(Self::parse_uuid)
            .transpose()?
            .unwrap_or_else(Uuid::nil);
        let is_todo: bool = row.get::<i64>(4)? != 0;
        let todo_due = Self::parse_optional_dt(row.get::<Option<String>>(5)?)?;
        let todo_completed = Self::parse_optional_dt(row.get::<Option<String>>(6)?)?;
        let created_at = Self::parse_required_dt(row.get::<String>(7)?)?;
        let updated_at = Self::parse_required_dt(row.get::<String>(8)?)?;
        let deleted_at = Self::parse_optional_dt(row.get::<Option<String>>(9)?)?;
        let alias: Option<String> = row.get(10)?;
        let bookmarks = json_to_bookmarks(&row.get::<String>(11)?);
        let links = json_to_links(&row.get::<String>(12)?);
        let vv = json_to_vv(&row.get::<String>(13)?);
        let last_writer: String = row.get(14)?;
        let is_pinned: bool = row.get::<i64>(15)? != 0;
        let is_starred: bool = row.get::<i64>(16)? != 0;
        let sort_key: u32 = row.get::<i64>(17)?.max(0) as u32;

        Ok(Note {
            id,
            title,
            body,
            notebook_id,
            is_todo,
            todo_due,
            todo_completed,
            created_at,
            updated_at,
            deleted_at,
            alias,
            bookmarks,
            links,
            vv,
            last_writer,
            is_pinned,
            is_starred,
            sort_key,
        })
    }
```

**What it does** — Maps an 18-column `notes` row (the fixed SELECT order used
everywhere) to a `Note`: NULL `notebook_id` → nil UUID (Inbox), JSON columns
via the lenient `json_to_*` parsers, `sort_key` clamped non-negative.

**Used by** — every note read/list.

---

### fn parse_uuid

**Identification** — marker `// md:impl DbBackend (row mapping) > fn parse_uuid`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (row mapping) > fn parse_uuid
    fn parse_uuid(s: String) -> Result<Uuid, StorageError> {
        s.parse()
            .map_err(|e: uuid::Error| StorageError::InvalidState(e.to_string()))
    }
```

**What it does** — `String → Uuid`, mapping failure to `InvalidState`
(corrupted row, server bug — not a caller error).

---

### fn parse_required_dt

**Identification** — marker `// md:impl DbBackend (row mapping) > fn parse_required_dt`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (row mapping) > fn parse_required_dt
    pub(super) fn parse_required_dt(s: String) -> Result<DateTime<Utc>, StorageError> {
        s.parse::<DateTime<Utc>>()
            .map_err(|e| StorageError::InvalidState(e.to_string()))
    }
```

**What it does** — `String → DateTime<Utc>`, failure → `InvalidState`.

---

### fn parse_optional_dt

**Identification** — marker `// md:impl DbBackend (row mapping) > fn parse_optional_dt`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (row mapping) > fn parse_optional_dt
    pub(super) fn parse_optional_dt(
        s: Option<String>,
    ) -> Result<Option<DateTime<Utc>>, StorageError> {
        match s {
            None => Ok(None),
            Some(v) => v
                .parse::<DateTime<Utc>>()
                .map(Some)
                .map_err(|e| StorageError::InvalidState(e.to_string())),
        }
    }
```

**What it does** — `Option<String> → Option<DateTime<Utc>>`, failure →
`InvalidState`.

---

### fn row_to_notebook

**Identification** — marker `// md:impl DbBackend (row mapping) > fn row_to_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (row mapping) > fn row_to_notebook
    pub(super) fn row_to_notebook(row: &libsql::Row) -> Result<Notebook, StorageError> {
        Ok(Notebook {
            id: Self::parse_uuid(row.get::<String>(0)?)?,
            title: row.get(1)?,
            created_at: Self::parse_required_dt(row.get::<String>(2)?)?,
            updated_at: Self::parse_required_dt(row.get::<String>(3)?)?,
            deleted_at: Self::parse_optional_dt(row.get::<Option<String>>(4)?)?,
            alias: row.get(5)?,
            vv: json_to_vv(&row.get::<String>(6)?),
            last_writer: row.get(7)?,
        })
    }
```

**What it does** — Maps the 8-column `notebooks` row shape.

---

### fn row_to_tag

**Identification** — marker `// md:impl DbBackend (row mapping) > fn row_to_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (row mapping) > fn row_to_tag
    pub(super) fn row_to_tag(row: &libsql::Row) -> Result<Tag, StorageError> {
        Ok(Tag {
            id: Self::parse_uuid(row.get::<String>(0)?)?,
            title: row.get(1)?,
            created_at: Self::parse_required_dt(row.get::<String>(2)?)?,
            updated_at: Self::parse_required_dt(row.get::<String>(3)?)?,
            deleted_at: Self::parse_optional_dt(row.get::<Option<String>>(4)?)?,
            vv: json_to_vv(&row.get::<String>(5)?),
            last_writer: row.get(6)?,
            system: row.get::<i64>(7)? != 0,
        })
    }
```

**What it does** — Maps the 8-column `tags` row shape. Column `7` is `system`, stored as
an SQLite integer and read back as a bool (`!= 0`). Every `SELECT` that feeds this mapper
(`read_tag`, `list_tags`) must therefore select the columns in this exact order, `system`
last.

---

### fn row_to_resource

**Identification** — marker `// md:impl DbBackend (row mapping) > fn row_to_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (row mapping) > fn row_to_resource
    pub(super) fn row_to_resource(row: &libsql::Row) -> Result<Resource, StorageError> {
        let width = row.get::<Option<i64>>(10)?;
        let height = row.get::<Option<i64>>(11)?;
        let dimensions = match (width, height) {
            (Some(w), Some(h)) => Some((w as u32, h as u32)),
            _ => None,
        };
        Ok(Resource {
            id: Self::parse_uuid(row.get::<String>(0)?)?,
            note_id: Self::parse_uuid(row.get::<String>(12)?)?,
            title: row.get(1)?,
            mime_type: row.get(2)?,
            file_name: row.get(3)?,
            size: row.get::<i64>(4)? as u64,
            duration_ms: row.get::<Option<i64>>(9)?.map(|v| v as u64),
            dimensions,
            created_at: Self::parse_required_dt(row.get::<String>(5)?)?,
            deleted_at: Self::parse_optional_dt(row.get::<Option<String>>(6)?)?,
            vv: json_to_vv(&row.get::<String>(7)?),
            last_writer: row.get(8)?,
        })
    }
```

**What it does** — Maps the 13-column metadata row shape (no `data` BLOB). Columns `9/10/11`
are `duration_ms`/`width`/`height`; column `12` is `note_id` (parsed as a UUID, like `id`);
`dimensions` is reconstructed as `Some((w, h))` only when
both are present (mixed → `None`, defensively). Every `SELECT` feeding this mapper
(`read_resource`, `list_resources`, `list_resources_for_note`, the resource-create
materialisation) must select the metadata columns in this order — `duration_ms, width, height,
note_id` after `last_writer`; only `read_resource` appends `data` afterwards, so its blob is
column `13`.

---

### fn row_to_change

**Identification** — marker `// md:impl DbBackend (row mapping) > fn row_to_change`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (row mapping) > fn row_to_change
    pub(super) fn row_to_change(
        entity_type: &str,
        entity_id_str: &str,
        operation: &str,
        changed_at: DateTime<Utc>,
        data: &serde_json::Value,
    ) -> Option<Change> {
        let id: Uuid = entity_id_str.parse().ok()?;
        match (entity_type, operation) {
            ("note", "create") => serde_json::from_value(data.clone())
                .ok()
                .map(|note| Change::NoteCreate { note }),
            ("note", "update") => serde_json::from_value(data.clone())
                .ok()
                .map(|note| Change::NoteUpdate { note }),
            ("note", "delete") => {
                let (deleted_at, vv, last_writer) = tombstone_from_data(data, changed_at);
                Some(Change::NoteDelete {
                    id,
                    deleted_at,
                    vv,
                    last_writer,
                })
            }
            ("notebook", "create") => serde_json::from_value(data.clone())
                .ok()
                .map(|notebook| Change::NotebookCreate { notebook }),
            ("notebook", "update") => serde_json::from_value(data.clone())
                .ok()
                .map(|notebook| Change::NotebookUpdate { notebook }),
            ("notebook", "delete") => {
                let (deleted_at, vv, last_writer) = tombstone_from_data(data, changed_at);
                Some(Change::NotebookDelete {
                    id,
                    deleted_at,
                    vv,
                    last_writer,
                })
            }
            ("tag", "create") => serde_json::from_value(data.clone())
                .ok()
                .map(|tag| Change::TagCreate { tag }),
            ("tag", "update") => serde_json::from_value(data.clone())
                .ok()
                .map(|tag| Change::TagUpdate { tag }),
            ("tag", "delete") => {
                let (deleted_at, vv, last_writer) = tombstone_from_data(data, changed_at);
                Some(Change::TagDelete {
                    id,
                    deleted_at,
                    vv,
                    last_writer,
                })
            }
            ("note_tag", "add") => {
                let tag_id: Uuid = data["tag_id"].as_str()?.parse().ok()?;
                let (updated_at, vv, last_writer) = assoc_from_data(data, changed_at);
                Some(Change::NoteTagAdd {
                    note_id: id,
                    tag_id,
                    updated_at,
                    vv,
                    last_writer,
                })
            }
            ("note_tag", "remove") => {
                let tag_id: Uuid = data["tag_id"].as_str()?.parse().ok()?;
                let (updated_at, vv, last_writer) = assoc_from_data(data, changed_at);
                Some(Change::NoteTagRemove {
                    note_id: id,
                    tag_id,
                    updated_at,
                    vv,
                    last_writer,
                })
            }
            ("resource", "create") => {
                let binary = data["_data_b64"]
                    .as_str()
                    .and_then(|b| STANDARD.decode(b).ok());
                serde_json::from_value(data.clone())
                    .ok()
                    .map(|resource| Change::ResourceCreate {
                        resource,
                        data: binary,
                    })
            }
            ("resource", "delete") => {
                let (deleted_at, vv, last_writer) = tombstone_from_data(data, changed_at);
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

**What it does** — Converts an `entity_changes` row into a typed `Change`:
create/update deserialise the full JSON; deletes reconstruct
`(deleted_at, vv, last_writer)` via `tombstone_from_data` (with `changed_at` as
fallback); note_tag add/remove via `assoc_from_data`; resource creates decode
`_data_b64` into `ResourceCreate.data`. Returns `None` for unknown
`(entity_type, operation)` pairs (a future build's rows) — callers log and skip
without aborting the sync.

**Used by** — `get_changes_since`.

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
| 2 | impl DbBackend (row mapping) | `// md:impl DbBackend (row mapping)` |
| 3 | fn row_to_note | `// md:impl DbBackend (row mapping) > fn row_to_note` |
| 4 | fn parse_uuid | `// md:impl DbBackend (row mapping) > fn parse_uuid` |
| 5 | fn parse_required_dt | `// md:impl DbBackend (row mapping) > fn parse_required_dt` |
| 6 | fn parse_optional_dt | `// md:impl DbBackend (row mapping) > fn parse_optional_dt` |
| 7 | fn row_to_notebook | `// md:impl DbBackend (row mapping) > fn row_to_notebook` |
| 8 | fn row_to_tag | `// md:impl DbBackend (row mapping) > fn row_to_tag` |
| 9 | fn row_to_resource | `// md:impl DbBackend (row mapping) > fn row_to_resource` |
| 10 | fn row_to_change | `// md:impl DbBackend (row mapping) > fn row_to_change` |
