# `storage/db/migrations.rs` — schema versioning and forward migrations

Self-contained companion for `keeplin-core/src/storage/db/migrations.rs`. It documents **every
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

**Identification** — file-level block: the imports this migration module needs. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview

use crate::{error::StorageError, models::SYSTEM_RESOURCE_NOTE_ID};

use super::DbBackend;
```

**What it does** — Schema versioning for the LibSQL database: the `user_version` ladder, the baseline schema and each forward migration step. Migrations are forward-only and are applied in order on open; a database stamped newer than this build is refused rather than downgraded. `mod migration_tests` lives here because it tests exactly this ladder.

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

## impl DbBackend (migrations)

**Identification** — the first inherent impl; marker `// md:impl DbBackend (migrations)`.
The schema-version ladder and every forward migration step.

**Code** — container: members documented as sub-blocks below: fn run_migrations, fn schema_version, fn apply_migration, fn migrate_v1_baseline, fn add_column_if_missing, fn migrate_v2_ordering, fn migrate_v3_tag_system, fn migrate_v4_resource_media, fn migrate_v5_resource_note_id.

---

### fn run_migrations

**Identification** — `async fn run_migrations(conn) -> Result<(), StorageError>`;
marker `// md:impl DbBackend (migrations) > fn run_migrations`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (migrations) > fn run_migrations
    pub(super) async fn run_migrations(conn: &libsql::Connection) -> Result<(), StorageError> {
        let current = Self::schema_version(conn).await?;
        if current > Self::SCHEMA_VERSION {
            return Err(StorageError::InvalidState(format!(
                "database schema version {current} is newer than this build supports \
                 (max {}); upgrade keeplin to open it",
                Self::SCHEMA_VERSION
            )));
        }
        for version in (current + 1)..=Self::SCHEMA_VERSION {
            conn.execute("BEGIN IMMEDIATE", ()).await?;
            let stepped = async {
                Self::apply_migration(conn, version).await?;
                conn.execute(&format!("PRAGMA user_version = {version}"), ())
                    .await?;
                Ok::<(), StorageError>(())
            }
            .await;
            match stepped {
                Ok(()) => {
                    conn.execute("COMMIT", ()).await?;
                    tracing::info!(version, "Applied database schema migration");
                }
                Err(e) => {
                    conn.execute("ROLLBACK", ()).await.ok();
                    return Err(e);
                }
            }
        }
        Ok(())
    }
```

**What it does** — Brings the schema up to `SCHEMA_VERSION`, recording progress
in `PRAGMA user_version` so each step runs exactly once across restarts. An
up-to-date database does no schema work; each outstanding step runs in its own
`BEGIN IMMEDIATE` transaction with the version stamp set **inside** it (SQLite
DDL is transactional; the stamp rolls back with a failed step, so a crash
mid-migration retries cleanly). A database whose `user_version` is **newer**
than this build is rejected (`InvalidState`) so a downgrade cannot corrupt a
schema it doesn't understand. `PRAGMA user_version = {n}` takes no bound
parameters; `n` is our own const, never caller input.

**Dependencies** — `schema_version`, `apply_migration`.

**Used by** — `new`. **Repeated context** — none.

---

### fn schema_version

**Identification** — marker `// md:impl DbBackend (migrations) > fn schema_version`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (migrations) > fn schema_version
    async fn schema_version(conn: &libsql::Connection) -> Result<u32, StorageError> {
        let mut rows = conn.query("PRAGMA user_version", ()).await?;
        match rows.next().await? {
            Some(row) => Ok(row.get::<i64>(0)?.max(0) as u32),
            None => Ok(0),
        }
    }
```

**What it does** — Reads `PRAGMA user_version` (`0` for a pre-framework or
never-stamped database).

**Used by** — `run_migrations`; migration tests.

---

### fn apply_migration

**Identification** — marker `// md:impl DbBackend (migrations) > fn apply_migration`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (migrations) > fn apply_migration
    async fn apply_migration(conn: &libsql::Connection, version: u32) -> Result<(), StorageError> {
        match version {
            1 => Self::migrate_v1_baseline(conn).await,
            2 => Self::migrate_v2_ordering(conn).await,
            3 => Self::migrate_v3_tag_system(conn).await,
            4 => Self::migrate_v4_resource_media(conn).await,
            5 => Self::migrate_v5_resource_note_id(conn).await,
            other => Err(StorageError::InvalidState(format!(
                "no migration defined for schema version {other}"
            ))),
        }
    }
```

**What it does** — Dispatches the step that advances **to** `version`: 1 →
`migrate_v1_baseline`, 2 → `migrate_v2_ordering`, 3 → `migrate_v3_tag_system`,
4 → `migrate_v4_resource_media`, anything else → `InvalidState`. The caller wraps
it in a transaction and bumps the stamp. `SCHEMA_VERSION` is now `5`.

**Used by** — `run_migrations`.

---

### fn migrate_v1_baseline

**Identification** — marker `// md:impl DbBackend (migrations) > fn migrate_v1_baseline`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (migrations) > fn migrate_v1_baseline
    async fn migrate_v1_baseline(conn: &libsql::Connection) -> Result<(), StorageError> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS notes (
                id              TEXT PRIMARY KEY,
                title           TEXT NOT NULL,
                body            TEXT NOT NULL DEFAULT '',
                notebook_id     TEXT,
                is_todo         INTEGER NOT NULL DEFAULT 0,
                todo_due        TEXT,
                todo_completed  TEXT,
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL,
                deleted_at      TEXT,
                alias           TEXT,
                bookmarks       TEXT NOT NULL DEFAULT '[]',
                links           TEXT NOT NULL DEFAULT '[]',
                vv              TEXT NOT NULL DEFAULT '{}',
                last_writer     TEXT NOT NULL DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS notebooks (
                id          TEXT PRIMARY KEY,
                title       TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL,
                deleted_at  TEXT,
                alias       TEXT,
                vv          TEXT NOT NULL DEFAULT '{}',
                last_writer TEXT NOT NULL DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS tags (
                id          TEXT PRIMARY KEY,
                title       TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL,
                deleted_at  TEXT,
                vv          TEXT NOT NULL DEFAULT '{}',
                last_writer TEXT NOT NULL DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS note_tags (
                note_id     TEXT NOT NULL,
                tag_id      TEXT NOT NULL,
                updated_at  TEXT,
                deleted_at  TEXT,
                vv          TEXT NOT NULL DEFAULT '{}',
                last_writer TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (note_id, tag_id)
            );

            -- Projection of each note's resolved outgoing links, maintained on every note
            -- write, so backlinks (who links to a given note) is an indexed lookup rather
            -- than a full scan. Only links with a resolved `target_note_id` are recorded;
            -- the target UUID is plaintext (like `notebook_id`), so the index also works
            -- under at-rest encryption.
            CREATE TABLE IF NOT EXISTS note_links (
                source_note_id TEXT NOT NULL,
                target_note_id TEXT NOT NULL,
                PRIMARY KEY (source_note_id, target_note_id)
            );

            CREATE TABLE IF NOT EXISTS resources (
                id          TEXT PRIMARY KEY,
                title       TEXT NOT NULL,
                mime_type   TEXT NOT NULL,
                file_name   TEXT NOT NULL,
                size        INTEGER NOT NULL,
                data        BLOB,
                created_at  TEXT NOT NULL,
                deleted_at  TEXT,
                vv          TEXT NOT NULL DEFAULT '{}',
                last_writer TEXT NOT NULL DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS sync_state (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS device (
                id TEXT PRIMARY KEY
            );

            -- Append-only change journal that records every mutation in insertion order.
            -- The `id` column is an auto-incrementing integer that serves as a
            -- tie-breaker when two changes share the same `changed_at` timestamp.
            -- The `data` column stores the full entity JSON for create/update operations
            -- and is NULL for delete operations. For resource creates, the JSON also
            -- contains a `_data_b64` key with the Base64-encoded binary payload so
            -- remote peers can reconstruct the complete resource from the journal alone.
            CREATE TABLE IF NOT EXISTS entity_changes (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                entity_type TEXT     NOT NULL,
                entity_id   TEXT     NOT NULL,
                operation   TEXT     NOT NULL,
                changed_at  TEXT     NOT NULL,
                data        TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_notes_updated_at        ON notes(updated_at);
            CREATE INDEX IF NOT EXISTS idx_notes_notebook_id       ON notes(notebook_id);
            CREATE INDEX IF NOT EXISTS idx_notes_is_todo           ON notes(is_todo) WHERE is_todo = 1;
            CREATE INDEX IF NOT EXISTS idx_note_tags_note_id       ON note_tags(note_id);
            CREATE INDEX IF NOT EXISTS idx_note_tags_tag_id        ON note_tags(tag_id);
            CREATE INDEX IF NOT EXISTS idx_resources_created_at    ON resources(created_at);
            CREATE INDEX IF NOT EXISTS idx_note_links_target       ON note_links(target_note_id);
            CREATE INDEX IF NOT EXISTS idx_entity_changes_changed_at ON entity_changes(changed_at);
            ",
        )
        .await?;

        Self::add_column_if_missing(conn, "notes", "alias TEXT").await?;
        Self::add_column_if_missing(conn, "notes", "bookmarks TEXT NOT NULL DEFAULT '[]'").await?;
        Self::add_column_if_missing(conn, "notes", "links TEXT NOT NULL DEFAULT '[]'").await?;
        Self::add_column_if_missing(conn, "notebooks", "alias TEXT").await?;

        for table in ["notes", "notebooks", "tags"] {
            Self::add_column_if_missing(conn, table, "vv TEXT NOT NULL DEFAULT '{}'").await?;
            Self::add_column_if_missing(conn, table, "last_writer TEXT NOT NULL DEFAULT ''")
                .await?;
        }
        Self::add_column_if_missing(conn, "note_tags", "updated_at TEXT").await?;
        Self::add_column_if_missing(conn, "note_tags", "deleted_at TEXT").await?;
        Self::add_column_if_missing(conn, "note_tags", "vv TEXT NOT NULL DEFAULT '{}'").await?;
        Self::add_column_if_missing(conn, "note_tags", "last_writer TEXT NOT NULL DEFAULT ''")
            .await?;
        Self::add_column_if_missing(conn, "resources", "deleted_at TEXT").await?;
        Self::add_column_if_missing(conn, "resources", "vv TEXT NOT NULL DEFAULT '{}'").await?;
        Self::add_column_if_missing(conn, "resources", "last_writer TEXT NOT NULL DEFAULT ''")
            .await?;

        Ok(())
    }
```

**What it does** — The baseline schema in one step: tables `notes`, `notebooks`,
`tags`, `note_tags` (versioned associations), `note_links` (projection of each
note's resolved outgoing links so backlinks are an indexed lookup; target UUIDs
are plaintext so the index works under at-rest encryption), `resources`
(metadata + BLOB), `sync_state`, `device`, `entity_changes` (append-only
journal; auto-increment `id` breaks `changed_at` ties; `data` holds full entity
JSON for create/update, `NULL` for deletes, and `_data_b64` for resource
payloads) plus the supporting indexes. All `IF NOT EXISTS`, followed by
`add_column_if_missing` guards for every column added since v0
(alias/bookmarks/links, `vv`/`last_writer` on all entities, versioned
`note_tags` columns, resource soft-delete columns) — so a pre-framework
database, which already has them, is carried onto the ladder at `1` unchanged.
**Deliberately no `UNIQUE` index for aliases**: under at-rest encryption the
stored alias is per-write ciphertext (fresh nonce, never compares equal), and a
hard constraint would make `apply_change` error on a sync-introduced duplicate
instead of tolerating it — `LinkingBackend` enforces uniqueness on plaintext at
the application layer.

**Used by** — `apply_migration`.

---

### fn add_column_if_missing

**Identification** — marker `// md:impl DbBackend (migrations) > fn add_column_if_missing`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (migrations) > fn add_column_if_missing
    async fn add_column_if_missing(
        conn: &libsql::Connection,
        table: &str,
        column_def: &str,
    ) -> Result<(), StorageError> {
        let sql = format!("ALTER TABLE {table} ADD COLUMN {column_def}");
        match conn.execute(&sql, ()).await {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("duplicate column name") => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
```

**What it does** — `ALTER TABLE … ADD COLUMN …`, treating "duplicate column
name" as success.

**Used by** — the migration steps.

---

### fn migrate_v2_ordering

**Identification** — marker `// md:impl DbBackend (migrations) > fn migrate_v2_ordering`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (migrations) > fn migrate_v2_ordering
    async fn migrate_v2_ordering(conn: &libsql::Connection) -> Result<(), StorageError> {
        Self::add_column_if_missing(conn, "notes", "is_pinned INTEGER NOT NULL DEFAULT 0").await?;
        Self::add_column_if_missing(conn, "notes", "is_starred INTEGER NOT NULL DEFAULT 0").await?;
        Self::add_column_if_missing(conn, "notes", "sort_key INTEGER NOT NULL DEFAULT 0").await?;
        conn.execute_batch(
            "
            UPDATE notes SET notebook_id = '00000000-0000-0000-0000-000000000000'
             WHERE notebook_id IS NULL;

            CREATE INDEX IF NOT EXISTS idx_notes_notebook_sort
                ON notes (notebook_id, sort_key, id);
            ",
        )
        .await?;
        Ok(())
    }
```

**What it does** — Migration v2: `is_pinned`/`is_starred`/`sort_key` columns
(defaults keep old rows valid — `sort_key 0` is the never-positioned sentinel);
existing `NULL notebook_id` rows are moved to the Inbox (nil UUID) so queries
never see NULL again; and the `(notebook_id, sort_key, id)` index behind
`list_notes_in_notebook`.

**Used by** — `apply_migration`.

---

### fn migrate_v3_tag_system

**Identification** — marker `// md:impl DbBackend (migrations) > fn migrate_v3_tag_system`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (migrations) > fn migrate_v3_tag_system
    async fn migrate_v3_tag_system(conn: &libsql::Connection) -> Result<(), StorageError> {
        Self::add_column_if_missing(conn, "tags", "system INTEGER NOT NULL DEFAULT 0").await?;
        Ok(())
    }
```

**What it does** — Migration v3: adds the `system` column to `tags`
(`INTEGER NOT NULL DEFAULT 0`, SQLite has no native bool). The default keeps every
existing tag valid as a non-system (`false`) tag. `add_column_if_missing` swallows the
"duplicate column name" error, so re-running on a database that already has the column is
a no-op.

**Dependencies** —
- `add_column_if_missing` — issues the `ALTER TABLE tags ADD COLUMN`; expects it to treat
  "duplicate column name" as success so the step is idempotent.

**Used by** — `apply_migration` (arm `3`).

---

### fn migrate_v4_resource_media

**Identification** — marker `// md:impl DbBackend (migrations) > fn migrate_v4_resource_media`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (migrations) > fn migrate_v4_resource_media
    async fn migrate_v4_resource_media(conn: &libsql::Connection) -> Result<(), StorageError> {
        Self::add_column_if_missing(conn, "resources", "duration_ms INTEGER").await?;
        Self::add_column_if_missing(conn, "resources", "width INTEGER").await?;
        Self::add_column_if_missing(conn, "resources", "height INTEGER").await?;
        Ok(())
    }
```

**What it does** — Migration v4: adds three nullable columns to `resources` —
`duration_ms`, `width`, `height` (all plain `INTEGER`, no `NOT NULL`). They back
`Resource.duration_ms` and `Resource.dimensions` (`(width, height)`). Nullable means a
pre-existing or non-media attachment simply has `NULL` in all three, which `row_to_resource`
reads back as `None`. `width`/`height` are written and read as a pair (both-or-neither).
`add_column_if_missing` swallows "duplicate column name", so re-running is a no-op.

**Dependencies** —
- `add_column_if_missing` — issues each `ALTER TABLE resources ADD COLUMN`; expects it to
  treat "duplicate column name" as success so the step is idempotent.

**Used by** — `apply_migration` (arm `4`).

---

### fn migrate_v5_resource_note_id

**Identification** — marker `// md:impl DbBackend (migrations) > fn migrate_v5_resource_note_id`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (migrations) > fn migrate_v5_resource_note_id
    async fn migrate_v5_resource_note_id(conn: &libsql::Connection) -> Result<(), StorageError> {
        let sentinel = SYSTEM_RESOURCE_NOTE_ID.to_string();
        Self::add_column_if_missing(
            conn,
            "resources",
            &format!("note_id TEXT NOT NULL DEFAULT '{sentinel}'"),
        )
        .await?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_resources_note ON resources(note_id, created_at, id)",
            (),
        )
        .await?;
        Ok(())
    }
```

**What it does** — Migration v5 (issue #125): adds `note_id TEXT NOT NULL DEFAULT '<sentinel>'`
to `resources` plus an index `(note_id, created_at, id)` that backs the native
`list_resources_for_note`. The default is `SYSTEM_RESOURCE_NOTE_ID`
(`00000000-0000-0000-0000-000000000001`): existing rows and non-media/system attachments get a
valid non-nil owner, and `row_to_resource` reads it back into `Resource.note_id`. `note_id` is
stored as `TEXT` like every other UUID in this SQLite schema (parsed by `parse_uuid`). Both
statements are idempotent (`add_column_if_missing` swallows "duplicate column name";
`CREATE INDEX IF NOT EXISTS`), so re-running is a no-op.

**Dependencies** —
- `add_column_if_missing` — issues the `ALTER TABLE`; expects "duplicate column name" treated
  as success for idempotency.
- `SYSTEM_RESOURCE_NOTE_ID` — the column default; expects the reserved non-nil sentinel so
  per-note queries never collide with it.

**Used by** — `apply_migration` (arm `5`).

---

## mod migration_tests

**Identification** — `#[cfg(test)] mod migration_tests`; marker
`// md:mod migration_tests`. An imports block, two helpers and six tests.

**Code** — container: members documented as sub-blocks below: imports, fn raw_conn, fn user_version, fn note_history_reads_this_devices_versions_newest_first, fn fresh_database_is_stamped_current_and_reopen_is_a_noop, fn tag_system_flag_round_trips, fn resource_media_metadata_round_trips, fn migrates_a_pre_framework_database_without_losing_data, fn refuses_to_open_a_newer_schema.

**What it does** — Pins the migration framework and the journal-derived
history.

The explicit `imports` leaf below preserves the test-module dependency preamble
verbatim.

---

### imports

**Identification** — test-module dependencies; marker `// md:mod migration_tests > imports`.

**Code** — complete and verbatim:

```rust
    // md:mod migration_tests > imports
    use super::*;
    use crate::models::{Note, Resource, Tag};
    use crate::storage::{HistoryRepository, NoteRepository, ResourceRepository, TagRepository};
    use uuid::Uuid;
```

**What it does** — Brings the parent module API and test-only dependencies into
scope.

**Dependencies** —

- `super::*` — the items under test from the parent module; expects: the parent keeps them at module scope; a rename or a move into a submodule breaks these tests at compile time, which is the intended early signal.

**Used by** — every block of `mod migration_tests` in this file: `fn raw_conn`, `fn user_version`, `fn note_history_reads_this_devices_versions_newest_first`, `fn fresh_database_is_stamped_current_and_reopen_is_a_noop`, `fn tag_system_flag_round_trips`, `fn resource_media_metadata_round_trips`, `fn migrates_a_pre_framework_database_without_losing_data`, `fn refuses_to_open_a_newer_schema`. Nothing outside the module can use it: the preamble is private to `mod migration_tests`.

**Repeated context** — This preamble is a leaf block, not scaffolding: only the `mod` declaration, its attributes and its braces are exempt from coverage, so these `use` lines carry their own marker and are verified verbatim against the source (template v2.5.0, RULE 6). Changing an import here without updating this fence fails `scripts/check-docs.sh`.

---

### fn raw_conn

**Identification** — helper; marker `// md:mod migration_tests > fn raw_conn`.

**Code** — complete and verbatim:

```rust
    // md:mod migration_tests > fn raw_conn
    async fn raw_conn(path: &std::path::Path) -> libsql::Connection {
        let db = libsql::Builder::new_local(path).build().await.unwrap();
        db.connect().unwrap()
    }
```

**What it does** — A raw libsql connection bypassing `DbBackend::new`, so a
test can plant a pre-framework schema.

---

### fn user_version

**Identification** — helper; marker
`// md:mod migration_tests > fn user_version`.

**Code** — complete and verbatim:

```rust
    // md:mod migration_tests > fn user_version
    async fn user_version(conn: &libsql::Connection) -> u32 {
        DbBackend::schema_version(conn).await.unwrap()
    }
```

**What it does** — Reads the stamp via `schema_version`.

---

### fn note_history_reads_this_devices_versions_newest_first

**Identification** — tokio test; marker
`// md:mod migration_tests > fn note_history_reads_this_devices_versions_newest_first`.

**Code** — complete and verbatim:

```rust
    // md:mod migration_tests > fn note_history_reads_this_devices_versions_newest_first
    #[tokio::test]
    async fn note_history_reads_this_devices_versions_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hist.db");
        let be = DbBackend::new(&path, "", "").await.unwrap();

        let n = be.create_note(Note::new("t", "v1")).await.unwrap();
        let mut e = n.clone();
        e.body = "v2".into();
        be.update_note(e).await.unwrap();

        let hist = be.note_history(n.id, 0).await.unwrap();
        assert_eq!(hist.len(), 2, "create + update");
        assert_eq!(hist[0].entity.as_ref().unwrap().body, "v2", "newest first");
        assert_eq!(hist[1].entity.as_ref().unwrap().body, "v1");

        be.delete_note(n.id).await.unwrap();
        let hist = be.note_history(n.id, 0).await.unwrap();
        assert_eq!(hist.len(), 3);
        assert!(hist[0].entity.is_none(), "newest version is the tombstone");

        assert_eq!(be.note_history(n.id, 1).await.unwrap().len(), 1);
    }
```

**What it does** — create + update → two versions newest-first; a delete adds
a tombstone version (`entity: None`) on top; `limit = 1` caps the reply.

---

### fn fresh_database_is_stamped_current_and_reopen_is_a_noop

**Identification** — tokio test; marker
`// md:mod migration_tests > fn fresh_database_is_stamped_current_and_reopen_is_a_noop`.

**Code** — complete and verbatim:

```rust
    // md:mod migration_tests > fn fresh_database_is_stamped_current_and_reopen_is_a_noop
    #[tokio::test]
    async fn fresh_database_is_stamped_current_and_reopen_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fresh.db");

        let be = DbBackend::new(&path, "", "").await.unwrap();
        assert_eq!(
            user_version(&be.conn).await,
            DbBackend::SCHEMA_VERSION,
            "a fresh database is stamped at the current schema version"
        );
        let note = be.create_note(Note::new("t", "b")).await.unwrap();
        drop(be);

        let reopened = DbBackend::new(&path, "", "").await.unwrap();
        assert_eq!(
            user_version(&reopened.conn).await,
            DbBackend::SCHEMA_VERSION
        );
        assert_eq!(reopened.read_note(note.id).await.unwrap().title, "t");
    }
```

**What it does** — A fresh database is stamped `SCHEMA_VERSION`, a note
round-trips, and reopening runs no migrations while preserving data.

---

### fn tag_system_flag_round_trips

**Identification** — tokio test; marker
`// md:mod migration_tests > fn tag_system_flag_round_trips`.

**Code** — complete and verbatim:

```rust
    // md:mod migration_tests > fn tag_system_flag_round_trips
    #[tokio::test]
    async fn tag_system_flag_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tag_system.db");
        let be = DbBackend::new(&path, "", "").await.unwrap();

        let mut t = Tag::new("internal");
        t.system = true;
        let created = be.create_tag(t).await.unwrap();
        assert!(
            created.system,
            "create_tag keeps the system flag it was given"
        );
        assert!(
            be.read_tag(created.id).await.unwrap().system,
            "system round-trips through the tags.system column"
        );

        let plain = be.create_tag(Tag::new("plain")).await.unwrap();
        assert!(!plain.system, "Tag::new defaults system to false");

        let mut upd = be.read_tag(plain.id).await.unwrap();
        upd.system = true;
        assert!(be.update_tag(upd).await.unwrap().system);
        assert!(
            be.read_tag(plain.id).await.unwrap().system,
            "update_tag persists a flipped system flag"
        );

        let (tags, _) = be.list_tags(100, None).await.unwrap();
        assert_eq!(
            tags.iter().filter(|t| t.system).count(),
            2,
            "list_tags surfaces the system flag for every row"
        );
    }
```

**What it does** — Exercises the `tags.system` column end-to-end on `DbBackend`: a tag
created with `system = true` reads back true; `Tag::new` defaults to `false`; `update_tag`
persists a flipped flag; and `list_tags` surfaces `system` on every row. Guards the column
order that `row_to_tag` depends on (any `SELECT` dropping the trailing `system` column would
fail here).

**Dependencies** —
- `DbBackend::{create_tag, read_tag, update_tag, list_tags}` — the CRUD surface under test;
  expects each to read/write the `system` column consistently.
- `Tag::new` — expects it to default `system` to `false`.

**Used by** — test binary only.

---

### fn resource_media_metadata_round_trips

**Identification** — tokio test; marker
`// md:mod migration_tests > fn resource_media_metadata_round_trips`.

**Code** — complete and verbatim:

```rust
    // md:mod migration_tests > fn resource_media_metadata_round_trips
    #[tokio::test]
    async fn resource_media_metadata_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("resource_media.db");
        let be = DbBackend::new(&path, "", "").await.unwrap();

        let mut r = Resource::new(SYSTEM_RESOURCE_NOTE_ID, "clip", "video/mp4", "clip.mp4", 10);
        r.duration_ms = Some(4200);
        r.dimensions = Some((1920, 1080));
        let created = be.create_resource(r, vec![1, 2, 3]).await.unwrap();
        assert_eq!(created.duration_ms, Some(4200));
        assert_eq!(created.dimensions, Some((1920, 1080)));

        let (read, blob) = be.read_resource(created.id).await.unwrap();
        assert_eq!(
            read.duration_ms,
            Some(4200),
            "duration survives create+read"
        );
        assert_eq!(read.dimensions, Some((1920, 1080)), "dimensions survive");
        assert_eq!(
            blob,
            vec![1, 2, 3],
            "blob still read from its shifted column"
        );

        let plain = be
            .create_resource(
                Resource::new(SYSTEM_RESOURCE_NOTE_ID, "doc", "text/plain", "d.txt", 3),
                vec![9],
            )
            .await
            .unwrap();
        assert_eq!(plain.duration_ms, None, "non-media attachment stays None");
        assert_eq!(plain.dimensions, None);

        let (listed, _) = be.list_resources(100, None).await.unwrap();
        let clip = listed.iter().find(|x| x.id == created.id).unwrap();
        assert_eq!(clip.duration_ms, Some(4200));
        assert_eq!(clip.dimensions, Some((1920, 1080)));
        let doc = listed.iter().find(|x| x.id == plain.id).unwrap();
        assert!(doc.duration_ms.is_none() && doc.dimensions.is_none());
    }
```

**What it does** — Exercises the media columns end-to-end on `DbBackend`: a resource created
with `duration_ms`/`dimensions` reads back with both preserved (and the blob still read from
its now-shifted column `12`); a non-media attachment keeps both `None`; and `list_resources`
surfaces both. Guards the column order `row_to_resource` and every resource `SELECT` depend on.

**Dependencies** —
- `DbBackend::{create_resource, read_resource, list_resources}` — the surface under test;
  expects each to read/write `duration_ms`/`width`/`height` in the agreed column order.
- `Resource::new` — expects it to default `duration_ms`/`dimensions` to `None`.

**Used by** — test binary only.

---

### fn migrates_a_pre_framework_database_without_losing_data

**Identification** — tokio test; marker
`// md:mod migration_tests > fn migrates_a_pre_framework_database_without_losing_data`.

**Code** — complete and verbatim:

```rust
    // md:mod migration_tests > fn migrates_a_pre_framework_database_without_losing_data
    #[tokio::test]
    async fn migrates_a_pre_framework_database_without_losing_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");

        {
            let conn = raw_conn(&path).await;
            conn.execute_batch(
                "CREATE TABLE notes (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    body TEXT NOT NULL DEFAULT '',
                    notebook_id TEXT,
                    is_todo INTEGER NOT NULL DEFAULT 0,
                    todo_due TEXT,
                    todo_completed TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    deleted_at TEXT
                );",
            )
            .await
            .unwrap();
            conn.execute(
                "INSERT INTO notes (id,title,body,created_at,updated_at)
                 VALUES ('11111111-1111-4111-8111-111111111111','legacy','kept',
                         '2020-01-01T00:00:00+00:00','2020-01-01T00:00:00+00:00')",
                (),
            )
            .await
            .unwrap();
            assert_eq!(user_version(&conn).await, 0, "unstamped legacy database");
        }

        let be = DbBackend::new(&path, "", "").await.unwrap();
        assert_eq!(user_version(&be.conn).await, DbBackend::SCHEMA_VERSION);

        let id: Uuid = "11111111-1111-4111-8111-111111111111".parse().unwrap();
        let migrated = be.read_note(id).await.unwrap();
        assert_eq!(migrated.title, "legacy");
        assert_eq!(migrated.body, "kept");
        assert!(migrated.vv.is_empty());
        assert_eq!(migrated.notebook_id, Uuid::nil());
        assert_eq!(migrated.sort_key, 0);
        assert!(!migrated.is_pinned);
        assert!(!migrated.is_starred);
        let (inbox, _) = be
            .list_notes_in_notebook(Uuid::nil(), 0, None)
            .await
            .unwrap();
        assert!(
            inbox.iter().any(|n| n.id == id),
            "the migrated note lists under the Inbox"
        );

        be.create_note(Note::new("after", "migration"))
            .await
            .unwrap();
    }
```

**What it does** — Plants an old-shape unstamped `notes` table with a row;
opening through `DbBackend` migrates in place to the current stamp; the legacy
row survives (empty vv, NULL notebook moved to the Inbox, sentinel sort key,
listed under the Inbox) and new writes work.

---

### fn refuses_to_open_a_newer_schema

**Identification** — tokio test; marker
`// md:mod migration_tests > fn refuses_to_open_a_newer_schema`.

**Code** — complete and verbatim:

```rust
    // md:mod migration_tests > fn refuses_to_open_a_newer_schema
    #[tokio::test]
    async fn refuses_to_open_a_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.db");

        {
            let conn = raw_conn(&path).await;
            conn.execute(
                &format!("PRAGMA user_version = {}", DbBackend::SCHEMA_VERSION + 1),
                (),
            )
            .await
            .unwrap();
        }

        let err = match DbBackend::new(&path, "", "").await {
            Ok(_) => panic!("opening a newer schema must be refused"),
            Err(e) => e,
        };
        assert!(
            matches!(err, StorageError::InvalidState(ref m) if m.contains("newer than this build")),
            "a newer schema must be refused, got: {err:?}"
        );
    }
```

**What it does** — A database stamped `SCHEMA_VERSION + 1` is refused with the
"newer than this build" `InvalidState`.

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
- Migrations are forward-only and applied in order; a database stamped newer than this build is refused, never downgraded.

---

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | impl DbBackend (migrations) | `// md:impl DbBackend (migrations)` |
| 3 | fn run_migrations | `// md:impl DbBackend (migrations) > fn run_migrations` |
| 4 | fn schema_version | `// md:impl DbBackend (migrations) > fn schema_version` |
| 5 | fn apply_migration | `// md:impl DbBackend (migrations) > fn apply_migration` |
| 6 | fn migrate_v1_baseline | `// md:impl DbBackend (migrations) > fn migrate_v1_baseline` |
| 7 | fn add_column_if_missing | `// md:impl DbBackend (migrations) > fn add_column_if_missing` |
| 8 | fn migrate_v2_ordering | `// md:impl DbBackend (migrations) > fn migrate_v2_ordering` |
| 9 | fn migrate_v3_tag_system | `// md:impl DbBackend (migrations) > fn migrate_v3_tag_system` |
| 10 | fn migrate_v4_resource_media | `// md:impl DbBackend (migrations) > fn migrate_v4_resource_media` |
| 11 | fn migrate_v5_resource_note_id | `// md:impl DbBackend (migrations) > fn migrate_v5_resource_note_id` |
| 12 | mod migration_tests | `// md:mod migration_tests` |
| 13 | imports | `// md:mod migration_tests > imports` |
| 14 | fn raw_conn | `// md:mod migration_tests > fn raw_conn` |
| 15 | fn user_version | `// md:mod migration_tests > fn user_version` |
| 16 | fn note_history_reads_this_devices_versions_newest_first | `// md:mod migration_tests > fn note_history_reads_this_devices_versions_newest_first` |
| 17 | fn fresh_database_is_stamped_current_and_reopen_is_a_noop | `// md:mod migration_tests > fn fresh_database_is_stamped_current_and_reopen_is_a_noop` |
| 18 | fn tag_system_flag_round_trips | `// md:mod migration_tests > fn tag_system_flag_round_trips` |
| 19 | fn resource_media_metadata_round_trips | `// md:mod migration_tests > fn resource_media_metadata_round_trips` |
| 20 | fn migrates_a_pre_framework_database_without_losing_data | `// md:mod migration_tests > fn migrates_a_pre_framework_database_without_losing_data` |
| 21 | fn refuses_to_open_a_newer_schema | `// md:mod migration_tests > fn refuses_to_open_a_newer_schema` |
