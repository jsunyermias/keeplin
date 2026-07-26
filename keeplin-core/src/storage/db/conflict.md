# `storage/db/conflict.rs` — version-vector conflict resolution

Self-contained companion for `keeplin-core/src/storage/db/conflict.rs`. It documents **every
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

**Identification** — file-level block: the imports the conflict-resolution helpers need. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview

use chrono::{DateTime, Utc};

use crate::error::StorageError;

use crate::storage::note_log::{self, resolve, VersionVector, Winner};
use crate::storage::SortableRfc3339;

use super::convert::{json_to_vv, vv_to_json};
use super::DbBackend;
```

**What it does** — Last-writer-wins conflict resolution over version vectors, for entities, note/tag associations and resources. These helpers decide whether an incoming change supersedes what is stored, and mint the next local version vector.

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

## impl DbBackend (conflict resolution)

**Identification** — the first inherent impl; marker `// md:impl DbBackend (conflict resolution)`.
Version-vector reads, last-writer-wins comparisons and the next-vector helpers, for entities, note/tag associations and resources.

**Code** — container: members documented as sub-blocks below: fn current_meta, fn incoming_wins, fn next_local_vv, fn row_is_live, fn assoc_meta, fn next_assoc_vv, fn assoc_incoming_wins, fn upsert_assoc, fn resource_meta, fn next_resource_vv, fn resource_incoming_wins.

---

### fn current_meta

**Identification** — marker `// md:impl DbBackend (conflict resolution) > fn current_meta`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (conflict resolution) > fn current_meta
    async fn current_meta(
        &self,
        table: &str,
        id: &str,
    ) -> Result<Option<(VersionVector, DateTime<Utc>, String)>, StorageError> {
        let mut rows = self
            .conn
            .query(
                &format!("SELECT vv, updated_at, last_writer FROM {table} WHERE id = ?1"),
                [id.to_owned()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Ok(Some((
                json_to_vv(&row.get::<String>(0)?),
                Self::parse_required_dt(row.get::<String>(1)?)?,
                row.get::<String>(2)?,
            ))),
            None => Ok(None),
        }
    }
```

**What it does** — Reads `(vv, updated_at, last_writer)` of a row in
`notes`/`notebooks`/`tags` (hard-coded table literals — interpolation is safe),
or `None` when absent. Feeds `resolve`.

**Used by** — `incoming_wins`, `next_local_vv`.

---

### fn incoming_wins

**Identification** — marker `// md:impl DbBackend (conflict resolution) > fn incoming_wins`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (conflict resolution) > fn incoming_wins
    pub(super) async fn incoming_wins(
        &self,
        table: &str,
        id: &str,
        incoming_vv: &VersionVector,
        incoming_updated: DateTime<Utc>,
        incoming_writer: &str,
    ) -> Result<bool, StorageError> {
        match self.current_meta(table, id).await? {
            None => Ok(true),
            Some((local_vv, local_updated, local_writer)) => Ok(matches!(
                resolve(
                    &local_vv,
                    local_updated,
                    &local_writer,
                    incoming_vv,
                    incoming_updated,
                    incoming_writer,
                ),
                Winner::Incoming
            )),
        }
    }
```

**What it does** — Whether an incoming remote write replaces the local row:
`true` with no local row, else `resolve(local, incoming) == Winner::Incoming`.
Replaces the old bare-`updated_at` LWW so concurrent edits converge
deterministically.

**Used by** — `apply_change` (notes/notebooks/tags).

---

### fn next_local_vv

**Identification** — marker `// md:impl DbBackend (conflict resolution) > fn next_local_vv`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (conflict resolution) > fn next_local_vv
    pub(super) async fn next_local_vv(
        &self,
        table: &str,
        id: &str,
    ) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .current_meta(table, id)
            .await?
            .map(|(vv, _, _)| vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }
```

**What it does** — The vector for a **local** write: current stored vector (or
empty) with this device's component incremented; the caller stamps the entity
and sets `last_writer = device_id`.

**Used by** — every local create/update/delete.

---

### fn row_is_live

**Identification** — marker `// md:impl DbBackend (conflict resolution) > fn row_is_live`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (conflict resolution) > fn row_is_live
    pub(super) async fn row_is_live(&self, table: &str, id: &str) -> Result<bool, StorageError> {
        let mut rows = self
            .conn
            .query(
                &format!("SELECT deleted_at FROM {table} WHERE id = ?1"),
                [id.to_owned()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Ok(row.get::<Option<String>>(0)?.is_none()),
            None => Ok(false),
        }
    }
```

**What it does** — Whether a row exists with `deleted_at IS NULL`
(`notes`/`tags` literals only). Used to refuse dangling associations.

**Used by** — `add_note_tag`.

---

### fn assoc_meta

**Identification** — marker `// md:impl DbBackend (conflict resolution) > fn assoc_meta`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (conflict resolution) > fn assoc_meta
    async fn assoc_meta(
        &self,
        note_id: &str,
        tag_id: &str,
    ) -> Result<Option<(VersionVector, DateTime<Utc>, String)>, StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT vv, updated_at, last_writer FROM note_tags WHERE note_id=?1 AND tag_id=?2",
                [note_id.to_owned(), tag_id.to_owned()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => {
                let updated_at = match row.get::<Option<String>>(1)? {
                    Some(s) => Self::parse_required_dt(s)?,
                    None => DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default(),
                };
                Ok(Some((
                    json_to_vv(&row.get::<String>(0)?),
                    updated_at,
                    row.get::<String>(2)?,
                )))
            }
            None => Ok(None),
        }
    }
```

**What it does** — Version metadata of a note↔tag association; a pre-version
row (NULL `updated_at`) is reported at the epoch so any real write dominates.

**Used by** — `next_assoc_vv`, `assoc_incoming_wins`.

---

### fn next_assoc_vv

**Identification** — marker `// md:impl DbBackend (conflict resolution) > fn next_assoc_vv`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (conflict resolution) > fn next_assoc_vv
    pub(super) async fn next_assoc_vv(
        &self,
        note_id: &str,
        tag_id: &str,
    ) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .assoc_meta(note_id, tag_id)
            .await?
            .map(|(vv, _, _)| vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }
```

**What it does** — Local association write vector (current + increment).

**Used by** — `add_note_tag`, `remove_note_tag`.

---

### fn assoc_incoming_wins

**Identification** — marker `// md:impl DbBackend (conflict resolution) > fn assoc_incoming_wins`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (conflict resolution) > fn assoc_incoming_wins
    pub(super) async fn assoc_incoming_wins(
        &self,
        note_id: &str,
        tag_id: &str,
        incoming_vv: &VersionVector,
        incoming_updated: DateTime<Utc>,
        incoming_writer: &str,
    ) -> Result<bool, StorageError> {
        match self.assoc_meta(note_id, tag_id).await? {
            None => Ok(true),
            Some((lvv, lupd, lwriter)) => Ok(matches!(
                resolve(
                    &lvv,
                    lupd,
                    &lwriter,
                    incoming_vv,
                    incoming_updated,
                    incoming_writer
                ),
                Winner::Incoming
            )),
        }
    }
```

**What it does** — `resolve` for association writes; `true` when the pair has
no row.

**Used by** — `apply_change` (NoteTagAdd/Remove).

---

### fn upsert_assoc

**Identification** — marker `// md:impl DbBackend (conflict resolution) > fn upsert_assoc`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (conflict resolution) > fn upsert_assoc
    pub(super) async fn upsert_assoc(
        &self,
        note_id: &str,
        tag_id: &str,
        updated_at: DateTime<Utc>,
        deleted_at: Option<DateTime<Utc>>,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO note_tags (note_id,tag_id,updated_at,deleted_at,vv,last_writer)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                libsql::params![
                    note_id.to_owned(),
                    tag_id.to_owned(),
                    updated_at.to_sortable_rfc3339(),
                    deleted_at.map(|d| d.to_sortable_rfc3339()),
                    vv_to_json(vv),
                    last_writer.to_owned(),
                ],
            )
            .await?;
        Ok(())
    }
```

**What it does** — `INSERT OR REPLACE` of the association's versioned state:
`deleted_at = NULL` for an add (present), `Some(ts)` for a remove (tombstone).

**Used by** — `add_note_tag`, `remove_note_tag`, `apply_change`.

---

### fn resource_meta

**Identification** — marker `// md:impl DbBackend (conflict resolution) > fn resource_meta`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (conflict resolution) > fn resource_meta
    async fn resource_meta(
        &self,
        id: &str,
    ) -> Result<Option<(VersionVector, DateTime<Utc>, String)>, StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT vv, created_at, deleted_at, last_writer FROM resources WHERE id=?1",
                [id.to_owned()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => {
                let created_at = Self::parse_required_dt(row.get::<String>(1)?)?;
                let deleted_at = Self::parse_optional_dt(row.get::<Option<String>>(2)?)?;
                Ok(Some((
                    json_to_vv(&row.get::<String>(0)?),
                    deleted_at.unwrap_or(created_at),
                    row.get::<String>(3)?,
                )))
            }
            None => Ok(None),
        }
    }
```

**What it does** — Resource version metadata; resources have no `updated_at`,
so the tiebreak timestamp is `deleted_at` when tombstoned else `created_at`.

**Used by** — `next_resource_vv`, `resource_incoming_wins`.

---

### fn next_resource_vv

**Identification** — marker `// md:impl DbBackend (conflict resolution) > fn next_resource_vv`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (conflict resolution) > fn next_resource_vv
    pub(super) async fn next_resource_vv(&self, id: &str) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .resource_meta(id)
            .await?
            .map(|(vv, _, _)| vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }
```

**What it does** — Local resource write vector (current + increment).

**Used by** — `create_resource`, `delete_resource`.

---

### fn resource_incoming_wins

**Identification** — marker `// md:impl DbBackend (conflict resolution) > fn resource_incoming_wins`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (conflict resolution) > fn resource_incoming_wins
    pub(super) async fn resource_incoming_wins(
        &self,
        id: &str,
        incoming_vv: &VersionVector,
        incoming_ts: DateTime<Utc>,
        incoming_writer: &str,
    ) -> Result<bool, StorageError> {
        match self.resource_meta(id).await? {
            None => Ok(true),
            Some((lvv, lts, lwriter)) => Ok(matches!(
                resolve(
                    &lvv,
                    lts,
                    &lwriter,
                    incoming_vv,
                    incoming_ts,
                    incoming_writer
                ),
                Winner::Incoming
            )),
        }
    }
```

**What it does** — `resolve` for resource changes; `true` with no local row.

**Used by** — `apply_change` (ResourceCreate/Delete).

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
| 2 | impl DbBackend (conflict resolution) | `// md:impl DbBackend (conflict resolution)` |
| 3 | fn current_meta | `// md:impl DbBackend (conflict resolution) > fn current_meta` |
| 4 | fn incoming_wins | `// md:impl DbBackend (conflict resolution) > fn incoming_wins` |
| 5 | fn next_local_vv | `// md:impl DbBackend (conflict resolution) > fn next_local_vv` |
| 6 | fn row_is_live | `// md:impl DbBackend (conflict resolution) > fn row_is_live` |
| 7 | fn assoc_meta | `// md:impl DbBackend (conflict resolution) > fn assoc_meta` |
| 8 | fn next_assoc_vv | `// md:impl DbBackend (conflict resolution) > fn next_assoc_vv` |
| 9 | fn assoc_incoming_wins | `// md:impl DbBackend (conflict resolution) > fn assoc_incoming_wins` |
| 10 | fn upsert_assoc | `// md:impl DbBackend (conflict resolution) > fn upsert_assoc` |
| 11 | fn resource_meta | `// md:impl DbBackend (conflict resolution) > fn resource_meta` |
| 12 | fn next_resource_vv | `// md:impl DbBackend (conflict resolution) > fn next_resource_vv` |
| 13 | fn resource_incoming_wins | `// md:impl DbBackend (conflict resolution) > fn resource_incoming_wins` |
