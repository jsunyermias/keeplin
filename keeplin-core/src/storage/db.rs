// md:Overview
use std::sync::Arc;

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, RwLock};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::{
    error::StorageError,
    links::{Bookmark, NoteLink},
    models::{new_id, now, Change, Note, NoteTag, Notebook, Resource, Tag},
};

use super::backend::DEFAULT_HISTORY_LIMIT;
use super::note_log::{self, resolve, VersionVector, Winner};
use super::{
    EntityVersion, HistoryRepository, NoteRepository, NotebookRepository, ResourceRepository,
    SortableRfc3339, SyncBackend, TagRepository,
};

// md:WsStream
type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// md:DbBackend
pub struct DbBackend {
    conn: libsql::Connection,
    server_url: String,
    auth_token: String,
    ws: Arc<Mutex<Option<WsStream>>>,
    device_id: String,
    http: reqwest::Client,
    history_unsupported: Arc<std::sync::atomic::AtomicBool>,
    server_capabilities: Arc<Mutex<CapabilityCache>>,
    lock: Arc<RwLock<()>>,
}

// md:impl DbBackend
impl DbBackend {
    // md:impl DbBackend > fn new
    pub async fn new(
        db_path: impl AsRef<std::path::Path>,
        server_url: impl Into<String>,
        auth_token: impl Into<String>,
    ) -> Result<Self, StorageError> {
        let server_url = server_url.into();
        let auth_token = auth_token.into();

        let db = libsql::Builder::new_local(db_path.as_ref()).build().await?;
        let conn = db.connect()?;

        Self::run_migrations(&conn).await?;

        let device_id = Self::get_or_create_device_id(&conn).await?;

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        let mut startup_capabilities = CapabilityCache::Unknown;
        if !server_url.is_empty() {
            if let Some(base) = http_base_of(&server_url) {
                match crate::compat::negotiate(&http, &base).await {
                    crate::compat::Handshake::Compatible(info) => {
                        tracing::info!(
                            server = %info.name,
                            server_version = %info.version,
                            protocol = info.protocol_version,
                            capabilities = ?info.capabilities,
                            "sync server protocol negotiated"
                        );
                        startup_capabilities = CapabilityCache::Known(info.capabilities);
                    }
                    crate::compat::Handshake::Incompatible(info) => {
                        return Err(StorageError::InvalidState(
                            crate::compat::incompatible_message(&info),
                        ));
                    }
                    crate::compat::Handshake::Unavailable => {
                        tracing::warn!(
                            url = %server_url,
                            "sync server has no usable GET /version (older keeplin-srv?); \
                             continuing without protocol negotiation"
                        );
                    }
                }
            }
        }

        let ws = if !server_url.is_empty() {
            match Self::connect_ws(&server_url, &auth_token, &device_id).await {
                Ok(stream) => {
                    tracing::info!(url = %server_url, "WebSocket connected");
                    Some(stream)
                }
                Err(e) => {
                    tracing::warn!("Could not connect WebSocket: {e}. Running offline.");
                    None
                }
            }
        } else {
            None
        };

        Ok(Self {
            conn,
            server_url,
            auth_token,
            ws: Arc::new(Mutex::new(ws)),
            device_id,
            http,
            history_unsupported: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            server_capabilities: Arc::new(Mutex::new(startup_capabilities)),
            lock: Arc::new(RwLock::new(())),
        })
    }

    const SCHEMA_VERSION: u32 = 3;

    // md:impl DbBackend > fn run_migrations
    async fn run_migrations(conn: &libsql::Connection) -> Result<(), StorageError> {
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

    // md:impl DbBackend > fn schema_version
    async fn schema_version(conn: &libsql::Connection) -> Result<u32, StorageError> {
        let mut rows = conn.query("PRAGMA user_version", ()).await?;
        match rows.next().await? {
            Some(row) => Ok(row.get::<i64>(0)?.max(0) as u32),
            None => Ok(0),
        }
    }

    // md:impl DbBackend > fn apply_migration
    async fn apply_migration(conn: &libsql::Connection, version: u32) -> Result<(), StorageError> {
        match version {
            1 => Self::migrate_v1_baseline(conn).await,
            2 => Self::migrate_v2_ordering(conn).await,
            3 => Self::migrate_v3_tag_system(conn).await,
            other => Err(StorageError::InvalidState(format!(
                "no migration defined for schema version {other}"
            ))),
        }
    }

    // md:impl DbBackend > fn migrate_v1_baseline
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

    // md:impl DbBackend > fn add_column_if_missing
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

    // md:impl DbBackend > fn get_or_create_device_id
    async fn get_or_create_device_id(conn: &libsql::Connection) -> Result<String, StorageError> {
        let mut rows = conn.query("SELECT id FROM device LIMIT 1", ()).await?;

        if let Some(row) = rows.next().await? {
            return Ok(row.get::<String>(0)?);
        }

        let id = new_id().to_string();
        conn.execute("INSERT INTO device (id) VALUES (?1)", [id.clone()])
            .await?;
        Ok(id)
    }

    // md:impl DbBackend > fn record_change
    async fn record_change(
        &self,
        entity_type: &str,
        entity_id: &str,
        operation: &str,
        data: Option<String>,
    ) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT INTO entity_changes (entity_type, entity_id, operation, changed_at, data)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                libsql::params![
                    entity_type,
                    entity_id,
                    operation,
                    now().to_sortable_rfc3339(),
                    data,
                ],
            )
            .await?;
        Ok(())
    }

    // md:impl DbBackend > fn refresh_note_links
    async fn refresh_note_links(&self, note: &Note) -> Result<(), StorageError> {
        self.conn
            .execute(
                "DELETE FROM note_links WHERE source_note_id = ?1",
                [note.id.to_string()],
            )
            .await?;
        for link in &note.links {
            if let Some(target) = link.target_note_id {
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO note_links (source_note_id, target_note_id)
                         VALUES (?1, ?2)",
                        [note.id.to_string(), target.to_string()],
                    )
                    .await?;
            }
        }
        Ok(())
    }

    // md:impl DbBackend > fn connect_ws
    async fn connect_ws(url: &str, token: &str, device_id: &str) -> Result<WsStream, StorageError> {
        let (mut stream, _) = connect_async(url).await?;
        stream
            .send(Message::Text(
                serde_json::json!({ "type": "auth", "token": token, "device_id": device_id })
                    .to_string(),
            ))
            .await?;
        Ok(stream)
    }

    // md:impl DbBackend > fn row_to_note
    fn row_to_note(row: &libsql::Row) -> Result<Note, StorageError> {
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

    // md:impl DbBackend > fn parse_uuid
    fn parse_uuid(s: String) -> Result<Uuid, StorageError> {
        s.parse()
            .map_err(|e: uuid::Error| StorageError::InvalidState(e.to_string()))
    }

    // md:impl DbBackend > fn parse_required_dt
    fn parse_required_dt(s: String) -> Result<DateTime<Utc>, StorageError> {
        s.parse::<DateTime<Utc>>()
            .map_err(|e| StorageError::InvalidState(e.to_string()))
    }

    // md:impl DbBackend > fn parse_optional_dt
    fn parse_optional_dt(s: Option<String>) -> Result<Option<DateTime<Utc>>, StorageError> {
        match s {
            None => Ok(None),
            Some(v) => v
                .parse::<DateTime<Utc>>()
                .map(Some)
                .map_err(|e| StorageError::InvalidState(e.to_string())),
        }
    }

    // md:impl DbBackend > fn row_to_notebook
    fn row_to_notebook(row: &libsql::Row) -> Result<Notebook, StorageError> {
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

    // md:impl DbBackend > fn row_to_tag
    fn row_to_tag(row: &libsql::Row) -> Result<Tag, StorageError> {
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

    // md:impl DbBackend > fn row_to_resource
    fn row_to_resource(row: &libsql::Row) -> Result<Resource, StorageError> {
        Ok(Resource {
            id: Self::parse_uuid(row.get::<String>(0)?)?,
            title: row.get(1)?,
            mime_type: row.get(2)?,
            file_name: row.get(3)?,
            size: row.get::<i64>(4)? as u64,
            created_at: Self::parse_required_dt(row.get::<String>(5)?)?,
            deleted_at: Self::parse_optional_dt(row.get::<Option<String>>(6)?)?,
            vv: json_to_vv(&row.get::<String>(7)?),
            last_writer: row.get(8)?,
        })
    }

    // md:impl DbBackend > fn row_to_change
    fn row_to_change(
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

    // md:impl DbBackend > fn begin
    async fn begin(&self) -> Result<(), StorageError> {
        self.conn.execute("BEGIN IMMEDIATE", ()).await?;
        Ok(())
    }

    // md:impl DbBackend > fn commit
    async fn commit(&self) -> Result<(), StorageError> {
        self.conn.execute("COMMIT", ()).await?;
        Ok(())
    }

    // md:impl DbBackend > fn rollback
    async fn rollback(&self) {
        self.conn.execute("ROLLBACK", ()).await.ok();
    }

    // md:impl DbBackend > fn ensure_ws
    async fn ensure_ws(guard: &mut Option<WsStream>, url: &str, token: &str, device_id: &str) {
        if guard.is_none() && !url.is_empty() {
            match Self::connect_ws(url, token, device_id).await {
                Ok(stream) => {
                    tracing::info!("WebSocket reconnected");
                    *guard = Some(stream);
                }
                Err(e) => {
                    tracing::warn!("WebSocket reconnect failed: {e}");
                }
            }
        }
    }

    // md:impl DbBackend > fn migrate_v2_ordering
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

    // md:impl DbBackend > fn migrate_v3_tag_system
    async fn migrate_v3_tag_system(conn: &libsql::Connection) -> Result<(), StorageError> {
        Self::add_column_if_missing(conn, "tags", "system INTEGER NOT NULL DEFAULT 0").await?;
        Ok(())
    }

    // md:impl DbBackend > fn current_meta
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

    // md:impl DbBackend > fn incoming_wins
    async fn incoming_wins(
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

    // md:impl DbBackend > fn next_local_vv
    async fn next_local_vv(&self, table: &str, id: &str) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .current_meta(table, id)
            .await?
            .map(|(vv, _, _)| vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    // md:impl DbBackend > fn row_is_live
    async fn row_is_live(&self, table: &str, id: &str) -> Result<bool, StorageError> {
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

    // md:impl DbBackend > fn assoc_meta
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

    // md:impl DbBackend > fn next_assoc_vv
    async fn next_assoc_vv(
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

    // md:impl DbBackend > fn assoc_incoming_wins
    async fn assoc_incoming_wins(
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

    // md:impl DbBackend > fn upsert_assoc
    async fn upsert_assoc(
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

    // md:impl DbBackend > fn resource_meta
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

    // md:impl DbBackend > fn next_resource_vv
    async fn next_resource_vv(&self, id: &str) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .resource_meta(id)
            .await?
            .map(|(vv, _, _)| vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    // md:impl DbBackend > fn resource_incoming_wins
    async fn resource_incoming_wins(
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
}

// md:fn parse_cursor
fn parse_cursor(token: Option<&str>) -> (String, String) {
    match token.filter(|t| !t.is_empty()) {
        Some(cursor) => match cursor.split_once('|') {
            Some((ts, id)) => (ts.to_owned(), id.to_owned()),
            None => (String::new(), String::new()),
        },
        None => (String::new(), String::new()),
    }
}

// md:fn build_page
fn build_page<T, F>(mut rows: Vec<T>, limit: usize, token_fn: F) -> (Vec<T>, Option<String>)
where
    F: Fn(&T) -> String,
{
    let has_more = rows.len() > limit;
    if has_more {
        rows.truncate(limit);
    }
    let next_token = if has_more {
        rows.last().map(token_fn)
    } else {
        None
    };
    (rows, next_token)
}

// md:fn bookmarks_to_json
fn bookmarks_to_json(bookmarks: &[Bookmark]) -> String {
    serde_json::to_string(bookmarks).unwrap_or_else(|_| "[]".to_string())
}

// md:fn links_to_json
fn links_to_json(links: &[NoteLink]) -> String {
    serde_json::to_string(links).unwrap_or_else(|_| "[]".to_string())
}

// md:fn json_to_bookmarks
fn json_to_bookmarks(s: &str) -> Vec<Bookmark> {
    serde_json::from_str(s).unwrap_or_default()
}

// md:fn json_to_links
fn json_to_links(s: &str) -> Vec<NoteLink> {
    serde_json::from_str(s).unwrap_or_default()
}

// md:fn vv_to_json
fn vv_to_json(vv: &VersionVector) -> String {
    serde_json::to_string(vv).unwrap_or_else(|_| "{}".to_string())
}

// md:fn json_to_vv
fn json_to_vv(s: &str) -> VersionVector {
    serde_json::from_str(s).unwrap_or_default()
}

// md:fn tombstone_data
fn tombstone_data(deleted_at: DateTime<Utc>, vv: &VersionVector, last_writer: &str) -> String {
    serde_json::json!({
        "deleted_at": deleted_at,
        "vv": vv,
        "last_writer": last_writer,
    })
    .to_string()
}

// md:fn assoc_data
fn assoc_data(
    tag_id: Uuid,
    updated_at: DateTime<Utc>,
    vv: &VersionVector,
    last_writer: &str,
) -> String {
    serde_json::json!({
        "tag_id": tag_id,
        "updated_at": updated_at,
        "vv": vv,
        "last_writer": last_writer,
    })
    .to_string()
}

// md:fn assoc_from_data
fn assoc_from_data(
    data: &serde_json::Value,
    changed_at: DateTime<Utc>,
) -> (DateTime<Utc>, VersionVector, String) {
    let updated_at = data
        .get("updated_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(changed_at);
    let vv = data
        .get("vv")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let last_writer = data
        .get("last_writer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (updated_at, vv, last_writer)
}

// md:fn tombstone_from_data
fn tombstone_from_data(
    data: &serde_json::Value,
    changed_at: DateTime<Utc>,
) -> (DateTime<Utc>, VersionVector, String) {
    let deleted_at = data
        .get("deleted_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(changed_at);
    let vv = data
        .get("vv")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let last_writer = data
        .get("last_writer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (deleted_at, vv, last_writer)
}

// md:impl NoteRepository for DbBackend
#[async_trait]
impl NoteRepository for DbBackend {
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

    // md:impl NoteRepository for DbBackend > fn update_note
    async fn update_note(&self, mut note: Note) -> Result<Note, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        note.vv = self.next_local_vv("notes", &note.id.to_string()).await?;
        note.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
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
}

// md:impl NotebookRepository for DbBackend
#[async_trait]
impl NotebookRepository for DbBackend {
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
}

// md:impl TagRepository for DbBackend
#[async_trait]
impl TagRepository for DbBackend {
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
}

// md:impl ResourceRepository for DbBackend
#[async_trait]
impl ResourceRepository for DbBackend {
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
                    "INSERT INTO resources (id,title,mime_type,file_name,size,data,created_at,deleted_at,vv,last_writer)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
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

    // md:impl ResourceRepository for DbBackend > fn read_resource
    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,mime_type,file_name,size,created_at,deleted_at,vv,last_writer,data
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
                let blob: Vec<u8> = row.get(9)?;
                Ok((resource, blob))
            }
        }
    }

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
                "SELECT id,title,mime_type,file_name,size,created_at,deleted_at,vv,last_writer
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
}

// md:impl SyncBackend for DbBackend
#[async_trait]
impl SyncBackend for DbBackend {
    // md:impl SyncBackend for DbBackend > fn get_changes_since
    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError> {
        let _read_guard = self.lock.read().await;
        let since_str = since.to_sortable_rfc3339();
        let mut rows = self
            .conn
            .query(
                "SELECT entity_type, entity_id, operation, changed_at, data
                 FROM entity_changes
                 WHERE changed_at > ?1
                 ORDER BY id ASC",
                [since_str],
            )
            .await?;

        let mut changes = Vec::new();
        while let Some(row) = rows.next().await? {
            let entity_type: String = row.get(0)?;
            let entity_id: String = row.get(1)?;
            let operation: String = row.get(2)?;
            let changed_at = Self::parse_required_dt(row.get::<String>(3)?)?;
            let data_str: Option<String> = row.get(4)?;
            let data: serde_json::Value = data_str
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::Value::Null);

            match Self::row_to_change(&entity_type, &entity_id, &operation, changed_at, &data) {
                Some(change) => changes.push(change),
                None => tracing::warn!(
                    entity_type,
                    operation,
                    "Unknown entity_changes entry; skipped"
                ),
            }
        }
        Ok(changes)
    }

    // md:impl SyncBackend for DbBackend > fn apply_change
    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        match change {
            Change::NoteCreate { note } | Change::NoteUpdate { note } => {
                if !self
                    .incoming_wins(
                        "notes",
                        &note.id.to_string(),
                        &note.vv,
                        note.updated_at,
                        &note.last_writer,
                    )
                    .await?
                {
                    return Ok(());
                }
                self.begin().await?;
                let r: Result<(), StorageError> = async {
                    self.refresh_note_links(&note).await?;
                    self.conn
                        .execute(
                            "INSERT OR REPLACE INTO notes
                             (id,title,body,notebook_id,is_todo,todo_due,todo_completed,created_at,updated_at,deleted_at,alias,bookmarks,links,vv,last_writer,is_pinned,is_starred,sort_key)
                             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
                            libsql::params![
                                note.id.to_string(),
                                note.title,
                                note.body,
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
                    Ok(())
                }
                .await;
                if let Err(e) = r {
                    self.rollback().await;
                    return Err(e);
                }
                self.commit().await?;
            }
            Change::NoteDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                if !self
                    .incoming_wins("notes", &id.to_string(), &vv, deleted_at, &last_writer)
                    .await?
                {
                    return Ok(());
                }
                let affected = self
                    .conn
                    .execute(
                        "UPDATE notes SET deleted_at = ?2, updated_at = ?2, vv = ?3, last_writer = ?4 WHERE id = ?1",
                        libsql::params![
                            id.to_string(),
                            deleted_at.to_sortable_rfc3339(),
                            vv_to_json(&vv),
                            last_writer.clone(),
                        ],
                    )
                    .await?;
                if affected == 0 {
                    self.conn
                        .execute(
                            "INSERT OR IGNORE INTO notes (id, title, created_at, updated_at, deleted_at, vv, last_writer)
                             VALUES (?1, '', ?2, ?2, ?2, ?3, ?4)",
                            libsql::params![
                                id.to_string(),
                                deleted_at.to_sortable_rfc3339(),
                                vv_to_json(&vv),
                                last_writer,
                            ],
                        )
                        .await?;
                }
            }
            Change::NotebookCreate { notebook } | Change::NotebookUpdate { notebook } => {
                if !self
                    .incoming_wins(
                        "notebooks",
                        &notebook.id.to_string(),
                        &notebook.vv,
                        notebook.updated_at,
                        &notebook.last_writer,
                    )
                    .await?
                {
                    return Ok(());
                }
                self.conn
                    .execute(
                        "INSERT OR REPLACE INTO notebooks (id,title,created_at,updated_at,deleted_at,alias,vv,last_writer)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                        libsql::params![
                            notebook.id.to_string(),
                            notebook.title,
                            notebook.created_at.to_sortable_rfc3339(),
                            notebook.updated_at.to_sortable_rfc3339(),
                            notebook.deleted_at.map(|d| d.to_sortable_rfc3339()),
                            notebook.alias.clone(),
                            vv_to_json(&notebook.vv),
                            notebook.last_writer.clone(),
                        ],
                    )
                    .await?;
            }
            Change::NotebookDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                if !self
                    .incoming_wins("notebooks", &id.to_string(), &vv, deleted_at, &last_writer)
                    .await?
                {
                    return Ok(());
                }
                let affected = self
                    .conn
                    .execute(
                        "UPDATE notebooks SET deleted_at = ?2, updated_at = ?2, vv = ?3, last_writer = ?4 WHERE id = ?1",
                        libsql::params![id.to_string(), deleted_at.to_sortable_rfc3339(), vv_to_json(&vv), last_writer.clone()],
                    )
                    .await?;
                if affected == 0 {
                    self.conn
                        .execute(
                            "INSERT OR IGNORE INTO notebooks (id, title, created_at, updated_at, deleted_at, vv, last_writer)
                             VALUES (?1, '', ?2, ?2, ?2, ?3, ?4)",
                            libsql::params![id.to_string(), deleted_at.to_sortable_rfc3339(), vv_to_json(&vv), last_writer],
                        )
                        .await?;
                }
            }
            Change::TagCreate { tag } | Change::TagUpdate { tag } => {
                if !self
                    .incoming_wins(
                        "tags",
                        &tag.id.to_string(),
                        &tag.vv,
                        tag.updated_at,
                        &tag.last_writer,
                    )
                    .await?
                {
                    return Ok(());
                }
                self.conn
                    .execute(
                        "INSERT OR REPLACE INTO tags (id,title,created_at,updated_at,deleted_at,vv,last_writer)
                         VALUES (?1,?2,?3,?4,?5,?6,?7)",
                        libsql::params![
                            tag.id.to_string(),
                            tag.title,
                            tag.created_at.to_sortable_rfc3339(),
                            tag.updated_at.to_sortable_rfc3339(),
                            tag.deleted_at.map(|d| d.to_sortable_rfc3339()),
                            vv_to_json(&tag.vv),
                            tag.last_writer.clone(),
                        ],
                    )
                    .await?;
            }
            Change::TagDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                if !self
                    .incoming_wins("tags", &id.to_string(), &vv, deleted_at, &last_writer)
                    .await?
                {
                    return Ok(());
                }
                let affected = self
                    .conn
                    .execute(
                        "UPDATE tags SET deleted_at = ?2, updated_at = ?2, vv = ?3, last_writer = ?4 WHERE id = ?1",
                        libsql::params![id.to_string(), deleted_at.to_sortable_rfc3339(), vv_to_json(&vv), last_writer.clone()],
                    )
                    .await?;
                if affected == 0 {
                    self.conn
                        .execute(
                            "INSERT OR IGNORE INTO tags (id, title, created_at, updated_at, deleted_at, vv, last_writer)
                             VALUES (?1, '', ?2, ?2, ?2, ?3, ?4)",
                            libsql::params![id.to_string(), deleted_at.to_sortable_rfc3339(), vv_to_json(&vv), last_writer],
                        )
                        .await?;
                }
            }
            Change::NoteTagAdd {
                note_id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            } => {
                let (n, t) = (note_id.to_string(), tag_id.to_string());
                if self
                    .assoc_incoming_wins(&n, &t, &vv, updated_at, &last_writer)
                    .await?
                {
                    self.upsert_assoc(&n, &t, updated_at, None, &vv, &last_writer)
                        .await?;
                }
            }
            Change::NoteTagRemove {
                note_id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            } => {
                let (n, t) = (note_id.to_string(), tag_id.to_string());
                if self
                    .assoc_incoming_wins(&n, &t, &vv, updated_at, &last_writer)
                    .await?
                {
                    self.upsert_assoc(&n, &t, updated_at, Some(updated_at), &vv, &last_writer)
                        .await?;
                }
            }
            Change::ResourceCreate { resource, data } => {
                let id = resource.id.to_string();
                let ts = resource.deleted_at.unwrap_or(resource.created_at);
                if self
                    .resource_incoming_wins(&id, &resource.vv, ts, &resource.last_writer)
                    .await?
                {
                    let blob = data.unwrap_or_default();
                    self.conn
                        .execute(
                            "INSERT OR REPLACE INTO resources (id,title,mime_type,file_name,size,data,created_at,deleted_at,vv,last_writer)
                             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                            libsql::params![
                                id,
                                resource.title,
                                resource.mime_type,
                                resource.file_name,
                                resource.size as i64,
                                blob,
                                resource.created_at.to_sortable_rfc3339(),
                                resource.deleted_at.map(|d| d.to_sortable_rfc3339()),
                                vv_to_json(&resource.vv),
                                resource.last_writer,
                            ],
                        )
                        .await?;
                }
            }
            Change::ResourceDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                if self
                    .resource_incoming_wins(&id.to_string(), &vv, deleted_at, &last_writer)
                    .await?
                {
                    let affected = self
                        .conn
                        .execute(
                            "UPDATE resources SET deleted_at=?2, vv=?3, last_writer=?4 WHERE id=?1",
                            libsql::params![
                                id.to_string(),
                                deleted_at.to_sortable_rfc3339(),
                                vv_to_json(&vv),
                                last_writer.clone(),
                            ],
                        )
                        .await?;
                    if affected == 0 {
                        self.conn
                            .execute(
                                "INSERT OR IGNORE INTO resources (id, title, mime_type, file_name, size, created_at, deleted_at, vv, last_writer)
                                 VALUES (?1, '', '', '', 0, ?2, ?2, ?3, ?4)",
                                libsql::params![
                                    id.to_string(),
                                    deleted_at.to_sortable_rfc3339(),
                                    vv_to_json(&vv),
                                    last_writer,
                                ],
                            )
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }

    // md:impl SyncBackend for DbBackend > fn get_last_sync_time
    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query("SELECT value FROM sync_state WHERE key = 'last_sync'", ())
            .await?;
        match rows.next().await? {
            Some(row) => {
                let s: String = row.get(0)?;
                s.parse::<DateTime<Utc>>()
                    .map_err(|e| StorageError::InvalidState(e.to_string()))
            }
            None => Ok(DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default()),
        }
    }

    // md:impl SyncBackend for DbBackend > fn update_sync_time
    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO sync_state (key, value) VALUES ('last_sync', ?1)",
                [ts.to_sortable_rfc3339()],
            )
            .await?;
        Ok(())
    }

    // md:impl SyncBackend for DbBackend > fn send_changes
    async fn send_changes(&self, changes: Vec<Change>) -> Result<(), StorageError> {
        if changes.is_empty() {
            return Ok(());
        }
        if self.server_url.is_empty() {
            tracing::debug!("No server_url configured; changes stay local");
            return Ok(());
        }
        let n = changes.len();
        let batch_id = new_id();
        let payload = serde_json::json!({
            "type": "changes",
            "batch_id": batch_id,
            "device_id": self.device_id,
            "changes": changes,
        })
        .to_string();

        for attempt in 0u32..=3 {
            let mut guard = self.ws.lock().await;
            Self::ensure_ws(
                &mut guard,
                &self.server_url,
                &self.auth_token,
                &self.device_id,
            )
            .await;
            let Some(ws) = guard.as_mut() else {
                return Err(StorageError::WebSocket(format!(
                    "cannot send {n} change(s): no WebSocket connection to {}",
                    self.server_url
                )));
            };
            match ws.send(Message::Text(payload.clone())).await {
                Ok(()) => {
                    tracing::info!(count = n, %batch_id, "Changes sent via WebSocket");
                    return Ok(());
                }
                Err(e) => {
                    *guard = None;
                    if attempt < 3 {
                        let delay = Duration::from_secs(2u64.pow(attempt));
                        tracing::warn!(attempt, ?delay, "WS send failed, retrying: {e}");
                        drop(guard);
                        tokio::time::sleep(delay).await;
                    } else {
                        return Err(StorageError::WebSocket(e.to_string()));
                    }
                }
            }
        }
        Ok(())
    }

    // md:impl SyncBackend for DbBackend > fn receive_changes
    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        let mut guard = self.ws.lock().await;
        Self::ensure_ws(
            &mut guard,
            &self.server_url,
            &self.auth_token,
            &self.device_id,
        )
        .await;
        if guard.is_none() {
            tracing::warn!("No WebSocket connection; no changes received");
            return Ok(vec![]);
        }
        const MAX_WS_MESSAGES: usize = 1_000;
        let drain_timeout = Duration::from_millis(100);
        let mut changes = Vec::new();
        let mut connection_closed = false;
        let mut msg_count = 0usize;
        {
            let ws = guard.as_mut().unwrap();
            loop {
                if msg_count >= MAX_WS_MESSAGES {
                    tracing::warn!(
                        limit = MAX_WS_MESSAGES,
                        "WebSocket message limit reached; remaining messages will be delivered on the next sync cycle"
                    );
                    break;
                }
                match timeout(drain_timeout, ws.next()).await {
                    Ok(Some(Ok(Message::Text(text)))) => {
                        msg_count += 1;
                        let v: serde_json::Value = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!("Skipping malformed WebSocket frame: {e}");
                                continue;
                            }
                        };
                        if v["type"] == "changes" {
                            if let Ok(batch) =
                                serde_json::from_value::<Vec<Change>>(v["changes"].clone())
                            {
                                tracing::info!(
                                    count = batch.len(),
                                    "Changes received via WebSocket"
                                );
                                changes.extend(batch);
                            }
                        }
                    }
                    Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => {
                        connection_closed = true;
                        break;
                    }
                    Err(_elapsed) => break,
                    Ok(Some(Ok(_))) => {}
                }
            }
        }
        if connection_closed {
            *guard = None;
        }
        Ok(changes)
    }

    // md:impl SyncBackend for DbBackend > fn prune_change_journal
    async fn prune_change_journal(&self, older_than: DateTime<Utc>) -> Result<u64, StorageError> {
        let _write_guard = self.lock.write().await;
        let affected = self
            .conn
            .execute(
                "DELETE FROM entity_changes WHERE changed_at < ?1",
                [older_than.to_sortable_rfc3339()],
            )
            .await?;
        tracing::info!(rows = affected, "Pruned entity_changes journal");
        Ok(affected)
    }

    // md:impl SyncBackend for DbBackend > fn get_device_id
    async fn get_device_id(&self) -> Result<String, StorageError> {
        Ok(self.device_id.clone())
    }
}

// md:ServerVersion
#[derive(Debug, serde::Deserialize)]
struct ServerVersion {
    timestamp: DateTime<Utc>,
    device_id: String,
    entity: Option<serde_json::Value>,
}

// md:CapabilityCache
enum CapabilityCache {
    Unknown,
    Unavailable,
    Known(Vec<String>),
}

// md:fn http_base_of
fn http_base_of(server_url: &str) -> Option<String> {
    let (scheme, rest) = if let Some(rest) = server_url.strip_prefix("wss://") {
        ("https://", rest)
    } else {
        ("http://", server_url.strip_prefix("ws://")?)
    };
    let rest = rest.strip_suffix("/api/sync").unwrap_or(rest);
    Some(format!("{scheme}{}", rest.trim_end_matches('/')))
}

// md:impl DbBackend (server history)
impl DbBackend {
    // md:impl DbBackend (server history) > fn server_http_base
    fn server_http_base(&self) -> Option<String> {
        http_base_of(&self.server_url)
    }

    // md:impl DbBackend (server history) > fn server_has_capability
    async fn server_has_capability(&self, capability: &str) -> Option<bool> {
        let mut cache = self.server_capabilities.lock().await;
        if let CapabilityCache::Unknown = &*cache {
            *cache = match self.server_http_base() {
                Some(base) => {
                    let url = format!("{base}/version");
                    match self.http.get(&url).send().await {
                        Ok(r) if r.status().is_success() => {
                            match r.json::<serde_json::Value>().await {
                                Ok(v) => {
                                    let caps = v
                                        .get("capabilities")
                                        .and_then(|c| c.as_array())
                                        .map(|a| {
                                            a.iter()
                                                .filter_map(|x| x.as_str().map(String::from))
                                                .collect::<Vec<_>>()
                                        })
                                        .unwrap_or_default();
                                    CapabilityCache::Known(caps)
                                }
                                Err(_) => CapabilityCache::Unavailable,
                            }
                        }
                        _ => CapabilityCache::Unavailable,
                    }
                }
                None => CapabilityCache::Unavailable,
            };
        }
        match &*cache {
            CapabilityCache::Known(caps) => Some(caps.iter().any(|c| c == capability)),
            _ => None,
        }
    }

    // md:impl DbBackend (server history) > fn server_entity_history
    async fn server_entity_history<T: serde::de::DeserializeOwned>(
        &self,
        entity_type: &str,
        id: Uuid,
        cap: u32,
    ) -> Option<Vec<EntityVersion<T>>> {
        use std::sync::atomic::Ordering;
        if self.history_unsupported.load(Ordering::Relaxed) {
            return None;
        }
        if self.server_has_capability("history").await == Some(false) {
            self.history_unsupported.store(true, Ordering::Relaxed);
            return None;
        }
        let base = self.server_http_base()?;
        let url = format!("{base}/api/{entity_type}s/{id}/history?limit={cap}");
        let response = match self
            .http
            .get(&url)
            .bearer_auth(&self.auth_token)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(%url, "server history unreachable, using local journal: {e}");
                return None;
            }
        };
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            self.history_unsupported.store(true, Ordering::Relaxed);
            tracing::debug!(%url, "server has no history endpoint; using the local journal");
            return None;
        }
        let response = match response.error_for_status() {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(%url, "server history error, using local journal: {e}");
                return None;
            }
        };
        let versions: Vec<ServerVersion> = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(%url, "malformed server history, using local journal: {e}");
                return None;
            }
        };
        Some(
            versions
                .into_iter()
                .filter_map(|v| {
                    let entity = match v.entity {
                        Some(raw) => Some(serde_json::from_value::<T>(raw).ok()?),
                        None => None,
                    };
                    Some(EntityVersion {
                        timestamp: v.timestamp,
                        device_id: v.device_id,
                        entity,
                    })
                })
                .collect(),
        )
    }

    // md:impl DbBackend (server history) > fn entity_history
    async fn entity_history<T: serde::de::DeserializeOwned>(
        &self,
        entity_type: &str,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<T>>, StorageError> {
        let cap = if limit == 0 {
            DEFAULT_HISTORY_LIMIT
        } else {
            limit
        };
        if let Some(versions) = self.server_entity_history::<T>(entity_type, id, cap).await {
            return Ok(versions);
        }
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT operation, changed_at, data
                 FROM entity_changes
                 WHERE entity_type = ?1 AND entity_id = ?2
                 ORDER BY id DESC
                 LIMIT ?3",
                libsql::params![entity_type, id.to_string(), cap as i64],
            )
            .await?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            let operation: String = row.get(0)?;
            let changed_at = Self::parse_required_dt(row.get::<String>(1)?)?;
            let data_str: Option<String> = row.get(2)?;
            let entity = match operation.as_str() {
                "create" | "update" => {
                    match data_str
                        .as_deref()
                        .and_then(|s| serde_json::from_str::<T>(s).ok())
                    {
                        Some(e) => Some(e),
                        None => continue,
                    }
                }
                "delete" => None,
                _ => continue,
            };
            out.push(EntityVersion {
                timestamp: changed_at,
                device_id: self.device_id.clone(),
                entity,
            });
        }
        Ok(out)
    }
}

// md:impl HistoryRepository for DbBackend
#[async_trait]
impl HistoryRepository for DbBackend {
    // md:impl HistoryRepository for DbBackend > fn note_history
    async fn note_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Note>>, StorageError> {
        self.entity_history::<Note>("note", id, limit).await
    }

    // md:impl HistoryRepository for DbBackend > fn notebook_history
    async fn notebook_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Notebook>>, StorageError> {
        self.entity_history::<Notebook>("notebook", id, limit).await
    }
}

// md:mod migration_tests
#[cfg(test)]
mod migration_tests {
    use super::*;

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
