// md:Overview

use crate::{error::StorageError, models::SYSTEM_RESOURCE_NOTE_ID};

use super::DbBackend;

// md:impl DbBackend (migrations)
impl DbBackend {
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

    // md:impl DbBackend (migrations) > fn schema_version
    async fn schema_version(conn: &libsql::Connection) -> Result<u32, StorageError> {
        let mut rows = conn.query("PRAGMA user_version", ()).await?;
        match rows.next().await? {
            Some(row) => Ok(row.get::<i64>(0)?.max(0) as u32),
            None => Ok(0),
        }
    }

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

    // md:impl DbBackend (migrations) > fn migrate_v3_tag_system
    async fn migrate_v3_tag_system(conn: &libsql::Connection) -> Result<(), StorageError> {
        Self::add_column_if_missing(conn, "tags", "system INTEGER NOT NULL DEFAULT 0").await?;
        Ok(())
    }

    // md:impl DbBackend (migrations) > fn migrate_v4_resource_media
    async fn migrate_v4_resource_media(conn: &libsql::Connection) -> Result<(), StorageError> {
        Self::add_column_if_missing(conn, "resources", "duration_ms INTEGER").await?;
        Self::add_column_if_missing(conn, "resources", "width INTEGER").await?;
        Self::add_column_if_missing(conn, "resources", "height INTEGER").await?;
        Ok(())
    }

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
}

// md:mod migration_tests
#[cfg(test)]
mod migration_tests {
    // md:mod migration_tests > imports
    use super::*;
    use crate::models::{Note, Resource, Tag};
    use crate::storage::{HistoryRepository, NoteRepository, ResourceRepository, TagRepository};
    use uuid::Uuid;

    // md:mod migration_tests > fn raw_conn
    async fn raw_conn(path: &std::path::Path) -> libsql::Connection {
        let db = libsql::Builder::new_local(path).build().await.unwrap();
        db.connect().unwrap()
    }

    // md:mod migration_tests > fn user_version
    async fn user_version(conn: &libsql::Connection) -> u32 {
        DbBackend::schema_version(conn).await.unwrap()
    }

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
}
