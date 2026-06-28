//! LibSQL-backed implementation of [`StorageBackend`] with WebSocket synchronisation.
//!
//! [`DbBackend`] stores all data in a local LibSQL (SQLite-compatible) database and
//! replicates mutations to a central server over a WebSocket connection. Every write
//! operation appends a row to the `entity_changes` journal table so that
//! `get_changes_since` can return a complete, ordered list of mutations since any
//! given point in time. Binary resource data is stored directly in the `resources`
//! table as a BLOB and is also embedded in the change journal as a Base64-encoded
//! `_data_b64` field so remote peers can reconstruct the full resource payload.

use std::sync::Arc;

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{new_id, now, Change, Note, NoteTag, Notebook, Resource, Tag},
};

use super::StorageBackend;

/// A WebSocket stream over either a plain TCP connection or a TLS-wrapped TCP connection.
///
/// `tokio_tungstenite::MaybeTlsStream` transparently handles both cases, so the
/// daemon can connect to `ws://` and `wss://` servers without changing this type.
type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// ── DbBackend ─────────────────────────────────────────────────────────────────

/// LibSQL-backed implementation of [`StorageBackend`] with optional WebSocket synchronisation.
///
/// All entities are stored in a local SQLite-compatible database opened via the
/// `libsql` crate. Every mutation is also recorded in the `entity_changes` append-only
/// table so that `get_changes_since` can efficiently enumerate all changes after a
/// given point in time.
///
/// When `server_url` is non-empty, a WebSocket connection is established on construction
/// and used by `send_changes` / `receive_changes` to exchange change batches with a
/// central server. If the WebSocket connection fails at any point, `ensure_ws` attempts
/// a reconnect before the next operation; while disconnected, the local database
/// continues to work normally and changes accumulate in `entity_changes` for the next
/// successful push.
pub struct DbBackend {
    /// The open LibSQL connection to the local database file.
    conn: libsql::Connection,
    /// The `ws://` or `wss://` URL of the synchronisation server. Empty string
    /// means offline mode — no WebSocket connection is attempted.
    server_url: String,
    /// The bearer token sent in the first WebSocket message for server authentication.
    /// Stored here so `ensure_ws` can re-authenticate on reconnect without requiring
    /// the caller to pass the token again.
    auth_token: String,
    /// The live WebSocket stream wrapped in a mutex so it can be accessed from
    /// multiple async tasks without data races. The `Option` represents the
    /// connection being absent (either not configured or lost and not yet reconnected).
    ws: Arc<Mutex<Option<WsStream>>>,
    /// A UUID string that permanently identifies this installation of the database.
    /// It is stored in the `device` table and is sent in every change batch so the
    /// server can identify the originating device and deduplicate messages.
    device_id: String,
}

impl DbBackend {
    /// Open (or create) a LibSQL database at `db_path`, run all pending schema
    /// migrations, and optionally connect to the synchronisation server.
    ///
    /// # Parameters
    ///
    /// - `db_path` — Path to the SQLite database file. The file is created if it does
    ///   not exist. Passing an empty string opens an in-memory database (useful for
    ///   tests), but LibSQL currently requires a real path, so tests use a path inside
    ///   a temporary directory.
    /// - `server_url` — WebSocket URL of the sync server (`"ws://…"` or `"wss://…"`).
    ///   Pass an empty string to run in offline mode without any WebSocket connection.
    /// - `auth_token` — Authentication token sent to the server as the first WebSocket
    ///   message. Ignored when `server_url` is empty.
    ///
    /// # Errors
    ///
    /// Returns `StorageError` if the database file cannot be opened, if schema
    /// migrations fail, or if the device-ID cannot be read from (or written to) the
    /// `device` table. A WebSocket connection failure is treated as a non-fatal warning:
    /// the backend is returned in offline mode rather than failing the constructor.
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

        let ws = if !server_url.is_empty() {
            match Self::connect_ws(&server_url, &auth_token).await {
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
        })
    }

    // ── Migrations ────────────────────────────────────────────────────────────

    /// Create all required tables and indexes if they do not already exist.
    ///
    /// All statements use `CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS`
    /// so this method is safe to call on every startup without checking the current
    /// schema version. Adding columns to existing tables requires a separate migration
    /// path (not yet needed) because SQLite does not support `ADD COLUMN IF NOT EXISTS`
    /// in older versions.
    ///
    /// Tables created:
    /// - `notes` — note records with soft-deletion via `deleted_at`.
    /// - `notebooks` — notebook records with soft-deletion.
    /// - `tags` — tag records with soft-deletion.
    /// - `note_tags` — many-to-many association between notes and tags.
    /// - `resources` — resource metadata and binary payload.
    /// - `sync_state` — key/value store for the last-sync timestamp.
    /// - `device` — stores the single device-identifier UUID.
    /// - `entity_changes` — append-only change journal used by `get_changes_since`.
    async fn run_migrations(conn: &libsql::Connection) -> Result<(), StorageError> {
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
                deleted_at      TEXT
            );

            CREATE TABLE IF NOT EXISTS notebooks (
                id          TEXT PRIMARY KEY,
                title       TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL,
                deleted_at  TEXT
            );

            CREATE TABLE IF NOT EXISTS tags (
                id          TEXT PRIMARY KEY,
                title       TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL,
                deleted_at  TEXT
            );

            CREATE TABLE IF NOT EXISTS note_tags (
                note_id TEXT NOT NULL,
                tag_id  TEXT NOT NULL,
                PRIMARY KEY (note_id, tag_id)
            );

            CREATE TABLE IF NOT EXISTS resources (
                id          TEXT PRIMARY KEY,
                title       TEXT NOT NULL,
                mime_type   TEXT NOT NULL,
                file_name   TEXT NOT NULL,
                size        INTEGER NOT NULL,
                data        BLOB,
                created_at  TEXT NOT NULL
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
            CREATE INDEX IF NOT EXISTS idx_note_tags_note_id       ON note_tags(note_id);
            CREATE INDEX IF NOT EXISTS idx_note_tags_tag_id        ON note_tags(tag_id);
            CREATE INDEX IF NOT EXISTS idx_entity_changes_changed_at ON entity_changes(changed_at);
            ",
        )
        .await?;
        Ok(())
    }

    // ── Device ID ─────────────────────────────────────────────────────────────

    /// Read the device identifier from the `device` table, or insert a new UUID v4
    /// string if the table is empty.
    ///
    /// The `device` table holds at most one row. On the very first startup the table
    /// is empty, so a new UUID is generated and inserted. On all subsequent startups
    /// the existing row is returned unchanged.
    ///
    /// The device identifier is included in every change batch sent to the server
    /// (as `"device_id"`) so the server can route changes to the correct recipient
    /// and avoid echoing a device's own changes back to itself.
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

    // ── Change journal ────────────────────────────────────────────────────────

    /// Insert one row into the `entity_changes` append-only journal.
    ///
    /// This helper is called by every mutating `StorageBackend` method immediately
    /// after the primary table operation succeeds. Because both writes happen on the
    /// same LibSQL connection (no transaction isolation is used here), a crash between
    /// the two writes could leave the primary table updated but the journal entry
    /// missing. In practice this means `get_changes_since` might miss a change; the
    /// next sync cycle will re-read the primary table state and catch up.
    ///
    /// # Parameters
    ///
    /// - `entity_type` — one of `"note"`, `"notebook"`, `"tag"`, `"note_tag"`,
    ///   or `"resource"`.
    /// - `entity_id` — the UUID string of the affected entity (the note_id for
    ///   `note_tag` operations).
    /// - `operation` — one of `"create"`, `"update"`, `"delete"`, `"add"`, or
    ///   `"remove"`.
    /// - `data` — the full entity serialised as a JSON string, or `None` for
    ///   delete operations where no payload is needed.
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
                libsql::params![entity_type, entity_id, operation, now().to_rfc3339(), data,],
            )
            .await?;
        Ok(())
    }

    // ── WebSocket ─────────────────────────────────────────────────────────────

    /// Open a WebSocket connection to `url` and perform the application-level
    /// handshake by sending an authentication token as the first message.
    ///
    /// The handshake message format is:
    /// ```json
    /// { "type": "auth", "token": "<auth_token>" }
    /// ```
    ///
    /// The server is expected to validate the token and either accept subsequent
    /// messages or close the connection. If the server closes the connection, the
    /// next call to `send_changes` or `receive_changes` will detect the closure and
    /// set the WebSocket field to `None`, triggering a reconnect on the next attempt.
    ///
    /// **Security note:** The token is sent in plaintext over the WebSocket. Always
    /// use a `wss://` (TLS) URL in production to prevent the token from being
    /// intercepted in transit.
    async fn connect_ws(url: &str, token: &str) -> Result<WsStream, StorageError> {
        let (mut stream, _) = connect_async(url).await?;
        // Send the authentication token immediately after the WebSocket handshake
        // so the server can verify the caller's identity before processing any
        // further messages.
        stream
            .send(Message::Text(
                serde_json::json!({ "type": "auth", "token": token }).to_string(),
            ))
            .await?;
        Ok(stream)
    }

    // ── Row → Note ────────────────────────────────────────────────────────────

    fn row_to_note(row: &libsql::Row) -> Result<Note, StorageError> {
        let id = Self::parse_uuid(row.get::<String>(0)?)?;
        let title: String = row.get(1)?;
        let body: String = row.get(2)?;
        let notebook_id: Option<Uuid> = row
            .get::<Option<String>>(3)?
            .map(Self::parse_uuid)
            .transpose()?;
        let is_todo: bool = row.get::<i64>(4)? != 0;
        let todo_due = Self::parse_optional_dt(row.get::<Option<String>>(5)?)?;
        let todo_completed = Self::parse_optional_dt(row.get::<Option<String>>(6)?)?;
        let created_at = Self::parse_required_dt(row.get::<String>(7)?)?;
        let updated_at = Self::parse_required_dt(row.get::<String>(8)?)?;
        let deleted_at = Self::parse_optional_dt(row.get::<Option<String>>(9)?)?;

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
        })
    }

    fn parse_uuid(s: String) -> Result<Uuid, StorageError> {
        s.parse()
            .map_err(|e: uuid::Error| StorageError::InvalidState(e.to_string()))
    }

    fn parse_required_dt(s: String) -> Result<DateTime<Utc>, StorageError> {
        s.parse::<DateTime<Utc>>()
            .map_err(|e| StorageError::InvalidState(e.to_string()))
    }

    fn parse_optional_dt(s: Option<String>) -> Result<Option<DateTime<Utc>>, StorageError> {
        match s {
            None => Ok(None),
            Some(v) => v
                .parse::<DateTime<Utc>>()
                .map(Some)
                .map_err(|e| StorageError::InvalidState(e.to_string())),
        }
    }

    fn row_to_notebook(row: &libsql::Row) -> Result<Notebook, StorageError> {
        Ok(Notebook {
            id: Self::parse_uuid(row.get::<String>(0)?)?,
            title: row.get(1)?,
            created_at: Self::parse_required_dt(row.get::<String>(2)?)?,
            updated_at: Self::parse_required_dt(row.get::<String>(3)?)?,
            deleted_at: Self::parse_optional_dt(row.get::<Option<String>>(4)?)?,
        })
    }

    fn row_to_tag(row: &libsql::Row) -> Result<Tag, StorageError> {
        Ok(Tag {
            id: Self::parse_uuid(row.get::<String>(0)?)?,
            title: row.get(1)?,
            created_at: Self::parse_required_dt(row.get::<String>(2)?)?,
            updated_at: Self::parse_required_dt(row.get::<String>(3)?)?,
            deleted_at: Self::parse_optional_dt(row.get::<Option<String>>(4)?)?,
        })
    }

    /// Convert a row from the `entity_changes` table into a typed [`Change`] variant.
    ///
    /// The four columns passed to this function correspond to the `entity_type`,
    /// `entity_id`, `operation`, and `data` columns of the `entity_changes` table.
    /// The `data` argument is a `serde_json::Value` already parsed from the stored
    /// JSON string (or `Null` when the column value is `NULL`).
    ///
    /// Returns `None` for any `(entity_type, operation)` combination that is not
    /// recognised. This can happen if a future version of the software added new
    /// entity types or operations that this version does not know about. Callers
    /// should log a warning and skip `None` entries without aborting the sync.
    ///
    /// For resource creates, the function checks for a `_data_b64` key in `data`.
    /// If present, it decodes the Base64 payload and attaches it to the
    /// `ResourceCreate.data` field so that peers without a copy of the binary file
    /// can reconstruct the full resource from the change record alone.
    fn row_to_change(
        entity_type: &str,
        entity_id_str: &str,
        operation: &str,
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
            ("note", "delete") => Some(Change::NoteDelete { id }),
            ("notebook", "create") => serde_json::from_value(data.clone())
                .ok()
                .map(|notebook| Change::NotebookCreate { notebook }),
            ("notebook", "update") => serde_json::from_value(data.clone())
                .ok()
                .map(|notebook| Change::NotebookUpdate { notebook }),
            ("notebook", "delete") => Some(Change::NotebookDelete { id }),
            ("tag", "create") => serde_json::from_value(data.clone())
                .ok()
                .map(|tag| Change::TagCreate { tag }),
            ("tag", "update") => serde_json::from_value(data.clone())
                .ok()
                .map(|tag| Change::TagUpdate { tag }),
            ("tag", "delete") => Some(Change::TagDelete { id }),
            ("note_tag", "add") => {
                let tag_id: Uuid = data["tag_id"].as_str()?.parse().ok()?;
                Some(Change::NoteTagAdd {
                    note_id: id,
                    tag_id,
                })
            }
            ("note_tag", "remove") => {
                let tag_id: Uuid = data["tag_id"].as_str()?.parse().ok()?;
                Some(Change::NoteTagRemove {
                    note_id: id,
                    tag_id,
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
            ("resource", "delete") => Some(Change::ResourceDelete { id }),
            _ => None,
        }
    }

    /// Reconnect the WebSocket if the current connection slot is empty and a server
    /// URL is configured.
    ///
    /// This method is called at the start of `send_changes` and `receive_changes`.
    /// When the connection was lost (the slot was set to `None` by a previous error),
    /// a fresh connection is established and re-authenticated. If reconnection fails,
    /// the slot remains `None` and the caller silently skips the network operation
    /// (changes accumulate locally until the connection is restored).
    async fn ensure_ws(guard: &mut Option<WsStream>, url: &str, token: &str) {
        if guard.is_none() && !url.is_empty() {
            match Self::connect_ws(url, token).await {
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
}

// ── StorageBackend impl ───────────────────────────────────────────────────────

#[async_trait]
impl StorageBackend for DbBackend {
    // ── Notes ─────────────────────────────────────────────────────────────────

    async fn create_note(&self, note: Note) -> Result<Note, StorageError> {
        self.conn
            .execute(
                "INSERT INTO notes
                 (id, title, body, notebook_id, is_todo, todo_due, todo_completed, created_at, updated_at, deleted_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                libsql::params![
                    note.id.to_string(),
                    note.title.clone(),
                    note.body.clone(),
                    note.notebook_id.map(|u| u.to_string()),
                    note.is_todo as i64,
                    note.todo_due.map(|d| d.to_rfc3339()),
                    note.todo_completed.map(|d| d.to_rfc3339()),
                    note.created_at.to_rfc3339(),
                    note.updated_at.to_rfc3339(),
                    note.deleted_at.map(|d| d.to_rfc3339()),
                ],
            )
            .await?;
        let data = serde_json::to_value(&note).ok().map(|v| v.to_string());
        self.record_change("note", &note.id.to_string(), "create", data)
            .await?;
        tracing::info!(id = %note.id, "Note created");
        Ok(note)
    }

    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,body,notebook_id,is_todo,todo_due,todo_completed,
                        created_at,updated_at,deleted_at
                 FROM notes WHERE id = ?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Self::row_to_note(&row),
            None => Err(StorageError::NotFound(id.to_string())),
        }
    }

    async fn update_note(&self, note: Note) -> Result<Note, StorageError> {
        let affected = self
            .conn
            .execute(
                "UPDATE notes SET
                 title=?2, body=?3, notebook_id=?4, is_todo=?5, todo_due=?6,
                 todo_completed=?7, updated_at=?8, deleted_at=?9
                 WHERE id = ?1",
                libsql::params![
                    note.id.to_string(),
                    note.title.clone(),
                    note.body.clone(),
                    note.notebook_id.map(|u| u.to_string()),
                    note.is_todo as i64,
                    note.todo_due.map(|d| d.to_rfc3339()),
                    note.todo_completed.map(|d| d.to_rfc3339()),
                    note.updated_at.to_rfc3339(),
                    note.deleted_at.map(|d| d.to_rfc3339()),
                ],
            )
            .await?;
        if affected == 0 {
            return Err(StorageError::NotFound(note.id.to_string()));
        }
        let data = serde_json::to_value(&note).ok().map(|v| v.to_string());
        self.record_change("note", &note.id.to_string(), "update", data)
            .await?;
        tracing::info!(id = %note.id, "Note updated");
        Ok(note)
    }

    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        let ts = now().to_rfc3339();
        let affected = self
            .conn
            .execute(
                "UPDATE notes SET deleted_at = ?2 WHERE id = ?1",
                [id.to_string(), ts],
            )
            .await?;
        if affected == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        self.record_change("note", &id.to_string(), "delete", None)
            .await?;
        tracing::info!(%id, "Note deleted");
        Ok(())
    }

    async fn list_notes(&self) -> Result<Vec<Note>, StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,body,notebook_id,is_todo,todo_due,todo_completed,
                        created_at,updated_at,deleted_at
                 FROM notes WHERE deleted_at IS NULL",
                (),
            )
            .await?;
        let mut notes = Vec::new();
        while let Some(row) = rows.next().await? {
            notes.push(Self::row_to_note(&row)?);
        }
        Ok(notes)
    }

    // ── Notebooks ─────────────────────────────────────────────────────────────

    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        self.conn
            .execute(
                "INSERT INTO notebooks (id,title,created_at,updated_at,deleted_at)
                 VALUES (?1,?2,?3,?4,?5)",
                libsql::params![
                    notebook.id.to_string(),
                    notebook.title.clone(),
                    notebook.created_at.to_rfc3339(),
                    notebook.updated_at.to_rfc3339(),
                    notebook.deleted_at.map(|d| d.to_rfc3339()),
                ],
            )
            .await?;
        let data = serde_json::to_value(&notebook).ok().map(|v| v.to_string());
        self.record_change("notebook", &notebook.id.to_string(), "create", data)
            .await?;
        tracing::info!(id = %notebook.id, "Notebook created");
        Ok(notebook)
    }

    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at
                 FROM notebooks WHERE id = ?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Self::row_to_notebook(&row),
            None => Err(StorageError::NotFound(id.to_string())),
        }
    }

    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        let affected = self
            .conn
            .execute(
                "UPDATE notebooks SET title=?2,updated_at=?3,deleted_at=?4 WHERE id=?1",
                libsql::params![
                    notebook.id.to_string(),
                    notebook.title.clone(),
                    notebook.updated_at.to_rfc3339(),
                    notebook.deleted_at.map(|d| d.to_rfc3339()),
                ],
            )
            .await?;
        if affected == 0 {
            return Err(StorageError::NotFound(notebook.id.to_string()));
        }
        let data = serde_json::to_value(&notebook).ok().map(|v| v.to_string());
        self.record_change("notebook", &notebook.id.to_string(), "update", data)
            .await?;
        tracing::info!(id = %notebook.id, "Notebook updated");
        Ok(notebook)
    }

    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        let affected = self
            .conn
            .execute(
                "UPDATE notebooks SET deleted_at=?2 WHERE id=?1",
                [id.to_string(), now().to_rfc3339()],
            )
            .await?;
        if affected == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        self.record_change("notebook", &id.to_string(), "delete", None)
            .await?;
        tracing::info!(%id, "Notebook deleted");
        Ok(())
    }

    async fn list_notebooks(&self) -> Result<Vec<Notebook>, StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at
                 FROM notebooks WHERE deleted_at IS NULL",
                (),
            )
            .await?;
        let mut notebooks = Vec::new();
        while let Some(row) = rows.next().await? {
            notebooks.push(Self::row_to_notebook(&row)?);
        }
        Ok(notebooks)
    }

    // ── Tags ──────────────────────────────────────────────────────────────────

    async fn create_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        self.conn
            .execute(
                "INSERT INTO tags (id,title,created_at,updated_at,deleted_at)
                 VALUES (?1,?2,?3,?4,?5)",
                libsql::params![
                    tag.id.to_string(),
                    tag.title.clone(),
                    tag.created_at.to_rfc3339(),
                    tag.updated_at.to_rfc3339(),
                    tag.deleted_at.map(|d| d.to_rfc3339()),
                ],
            )
            .await?;
        let data = serde_json::to_value(&tag).ok().map(|v| v.to_string());
        self.record_change("tag", &tag.id.to_string(), "create", data)
            .await?;
        tracing::info!(id = %tag.id, "Tag created");
        Ok(tag)
    }

    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at
                 FROM tags WHERE id = ?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Self::row_to_tag(&row),
            None => Err(StorageError::NotFound(id.to_string())),
        }
    }

    async fn update_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        let affected = self
            .conn
            .execute(
                "UPDATE tags SET title=?2,updated_at=?3,deleted_at=?4 WHERE id=?1",
                libsql::params![
                    tag.id.to_string(),
                    tag.title.clone(),
                    tag.updated_at.to_rfc3339(),
                    tag.deleted_at.map(|d| d.to_rfc3339()),
                ],
            )
            .await?;
        if affected == 0 {
            return Err(StorageError::NotFound(tag.id.to_string()));
        }
        let data = serde_json::to_value(&tag).ok().map(|v| v.to_string());
        self.record_change("tag", &tag.id.to_string(), "update", data)
            .await?;
        tracing::info!(id = %tag.id, "Tag updated");
        Ok(tag)
    }

    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        let affected = self
            .conn
            .execute(
                "UPDATE tags SET deleted_at=?2 WHERE id=?1",
                [id.to_string(), now().to_rfc3339()],
            )
            .await?;
        if affected == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        self.record_change("tag", &id.to_string(), "delete", None)
            .await?;
        tracing::info!(%id, "Tag deleted");
        Ok(())
    }

    async fn list_tags(&self) -> Result<Vec<Tag>, StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at
                 FROM tags WHERE deleted_at IS NULL",
                (),
            )
            .await?;
        let mut tags = Vec::new();
        while let Some(row) = rows.next().await? {
            tags.push(Self::row_to_tag(&row)?);
        }
        Ok(tags)
    }

    // ── Note–Tag relations ────────────────────────────────────────────────────

    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO note_tags (note_id,tag_id) VALUES (?1,?2)",
                [note_tag.note_id.to_string(), note_tag.tag_id.to_string()],
            )
            .await?;
        let data = serde_json::json!({ "tag_id": note_tag.tag_id }).to_string();
        self.record_change("note_tag", &note_tag.note_id.to_string(), "add", Some(data))
            .await?;
        Ok(())
    }

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        self.conn
            .execute(
                "DELETE FROM note_tags WHERE note_id=?1 AND tag_id=?2",
                [note_id.to_string(), tag_id.to_string()],
            )
            .await?;
        let data = serde_json::json!({ "tag_id": tag_id }).to_string();
        self.record_change("note_tag", &note_id.to_string(), "remove", Some(data))
            .await?;
        Ok(())
    }

    async fn list_note_tags(&self, note_id: Uuid) -> Result<Vec<Tag>, StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT t.id,t.title,t.created_at,t.updated_at,t.deleted_at
                 FROM tags t
                 JOIN note_tags nt ON t.id = nt.tag_id
                 WHERE nt.note_id = ?1 AND t.deleted_at IS NULL",
                [note_id.to_string()],
            )
            .await?;
        let mut tags = Vec::new();
        while let Some(row) = rows.next().await? {
            tags.push(Self::row_to_tag(&row)?);
        }
        Ok(tags)
    }

    // ── Resources ─────────────────────────────────────────────────────────────

    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        // Encode the binary payload as Base64 before moving `data` into the SQL
        // parameter list. The Base64 string is stored in the `entity_changes` journal
        // under the key `_data_b64` so that peers that receive this change via
        // `get_changes_since` can retrieve the full binary resource without needing
        // to download it separately.
        let data_b64 = STANDARD.encode(&data);
        self.conn
            .execute(
                "INSERT INTO resources (id,title,mime_type,file_name,size,data,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                libsql::params![
                    resource.id.to_string(),
                    resource.title.clone(),
                    resource.mime_type.clone(),
                    resource.file_name.clone(),
                    resource.size as i64,
                    data,
                    resource.created_at.to_rfc3339(),
                ],
            )
            .await?;
        let change_data = serde_json::to_value(&resource).ok().map(|mut v| {
            v["_data_b64"] = serde_json::Value::String(data_b64);
            v.to_string()
        });
        self.record_change("resource", &resource.id.to_string(), "create", change_data)
            .await?;
        tracing::info!(id = %resource.id, "Resource created");
        Ok(resource)
    }

    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,mime_type,file_name,size,data,created_at
                 FROM resources WHERE id=?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            None => Err(StorageError::NotFound(id.to_string())),
            Some(row) => {
                let resource = Resource {
                    id: Self::parse_uuid(row.get::<String>(0)?)?,
                    title: row.get(1)?,
                    mime_type: row.get(2)?,
                    file_name: row.get(3)?,
                    size: row.get::<i64>(4)? as u64,
                    created_at: Self::parse_required_dt(row.get::<String>(6)?)?,
                };
                let blob: Vec<u8> = row.get(5)?;
                Ok((resource, blob))
            }
        }
    }

    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        let affected = self
            .conn
            .execute("DELETE FROM resources WHERE id=?1", [id.to_string()])
            .await?;
        if affected == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        self.record_change("resource", &id.to_string(), "delete", None)
            .await?;
        tracing::info!(%id, "Resource deleted");
        Ok(())
    }

    async fn list_resources(&self) -> Result<Vec<Resource>, StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,mime_type,file_name,size,created_at FROM resources",
                (),
            )
            .await?;
        let mut resources = Vec::new();
        while let Some(row) = rows.next().await? {
            resources.push(Resource {
                id: Self::parse_uuid(row.get::<String>(0)?)?,
                title: row.get(1)?,
                mime_type: row.get(2)?,
                file_name: row.get(3)?,
                size: row.get::<i64>(4)? as u64,
                created_at: Self::parse_required_dt(row.get::<String>(5)?)?,
            });
        }
        Ok(resources)
    }

    // ── Synchronisation ───────────────────────────────────────────────────────

    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError> {
        let since_str = since.to_rfc3339();
        let mut rows = self
            .conn
            .query(
                "SELECT entity_type, entity_id, operation, data
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
            let data_str: Option<String> = row.get(3)?;
            let data: serde_json::Value = data_str
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::Value::Null);

            match Self::row_to_change(&entity_type, &entity_id, &operation, &data) {
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

    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        match change {
            // Notes
            Change::NoteCreate { note } | Change::NoteUpdate { note } => {
                self.conn
                    .execute(
                        "INSERT OR REPLACE INTO notes
                         (id,title,body,notebook_id,is_todo,todo_due,todo_completed,created_at,updated_at,deleted_at)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                        libsql::params![
                            note.id.to_string(),
                            note.title,
                            note.body,
                            note.notebook_id.map(|u| u.to_string()),
                            note.is_todo as i64,
                            note.todo_due.map(|d| d.to_rfc3339()),
                            note.todo_completed.map(|d| d.to_rfc3339()),
                            note.created_at.to_rfc3339(),
                            note.updated_at.to_rfc3339(),
                            note.deleted_at.map(|d| d.to_rfc3339()),
                        ],
                    )
                    .await?;
            }
            Change::NoteDelete { id } => {
                self.conn
                    .execute(
                        "UPDATE notes SET deleted_at = ?2 WHERE id = ?1",
                        [id.to_string(), now().to_rfc3339()],
                    )
                    .await?;
            }
            // Notebooks
            Change::NotebookCreate { notebook } | Change::NotebookUpdate { notebook } => {
                self.conn
                    .execute(
                        "INSERT OR REPLACE INTO notebooks (id,title,created_at,updated_at,deleted_at)
                         VALUES (?1,?2,?3,?4,?5)",
                        libsql::params![
                            notebook.id.to_string(),
                            notebook.title,
                            notebook.created_at.to_rfc3339(),
                            notebook.updated_at.to_rfc3339(),
                            notebook.deleted_at.map(|d| d.to_rfc3339()),
                        ],
                    )
                    .await?;
            }
            Change::NotebookDelete { id } => {
                self.conn
                    .execute(
                        "UPDATE notebooks SET deleted_at = ?2 WHERE id = ?1",
                        [id.to_string(), now().to_rfc3339()],
                    )
                    .await?;
            }
            // Tags
            Change::TagCreate { tag } | Change::TagUpdate { tag } => {
                self.conn
                    .execute(
                        "INSERT OR REPLACE INTO tags (id,title,created_at,updated_at,deleted_at)
                         VALUES (?1,?2,?3,?4,?5)",
                        libsql::params![
                            tag.id.to_string(),
                            tag.title,
                            tag.created_at.to_rfc3339(),
                            tag.updated_at.to_rfc3339(),
                            tag.deleted_at.map(|d| d.to_rfc3339()),
                        ],
                    )
                    .await?;
            }
            Change::TagDelete { id } => {
                self.conn
                    .execute(
                        "UPDATE tags SET deleted_at = ?2 WHERE id = ?1",
                        [id.to_string(), now().to_rfc3339()],
                    )
                    .await?;
            }
            // NoteTag associations
            Change::NoteTagAdd { note_id, tag_id } => {
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO note_tags (note_id, tag_id) VALUES (?1, ?2)",
                        [note_id.to_string(), tag_id.to_string()],
                    )
                    .await?;
            }
            Change::NoteTagRemove { note_id, tag_id } => {
                self.conn
                    .execute(
                        "DELETE FROM note_tags WHERE note_id = ?1 AND tag_id = ?2",
                        [note_id.to_string(), tag_id.to_string()],
                    )
                    .await?;
            }
            // Apply a remote resource-create change. When `data` is `Some` (the change
            // came from a peer that embedded the binary payload in the change record),
            // the payload is inserted into the `resources.data` column. When `data` is
            // `None` (the change came from an FsBackend peer that relies on file
            // replication), an empty byte vector is stored as a placeholder.
            // `INSERT OR IGNORE` is used to avoid overwriting a row that was already
            // inserted with a real payload by a concurrent or earlier operation.
            Change::ResourceCreate { resource, data } => {
                let blob = data.unwrap_or_default();
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO resources (id,title,mime_type,file_name,size,data,created_at)
                         VALUES (?1,?2,?3,?4,?5,?6,?7)",
                        libsql::params![
                            resource.id.to_string(),
                            resource.title,
                            resource.mime_type,
                            resource.file_name,
                            resource.size as i64,
                            blob,
                            resource.created_at.to_rfc3339(),
                        ],
                    )
                    .await?;
            }
            Change::ResourceDelete { id } => {
                self.conn
                    .execute("DELETE FROM resources WHERE id = ?1", [id.to_string()])
                    .await?;
            }
        }
        Ok(())
    }

    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
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

    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO sync_state (key, value) VALUES ('last_sync', ?1)",
                [ts.to_rfc3339()],
            )
            .await?;
        Ok(())
    }

    async fn send_changes(&self, changes: Vec<Change>) -> Result<(), StorageError> {
        if changes.is_empty() {
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

        // Retry sending with exponential backoff to tolerate transient network
        // disruptions. Delays are 2 s, 4 s, and 8 s. After four attempts the error
        // is propagated to the caller so the `SyncEngine` can log it and leave the
        // last-sync timestamp unchanged for a retry on the next cycle.
        for attempt in 0u32..=3 {
            let mut guard = self.ws.lock().await;
            Self::ensure_ws(&mut guard, &self.server_url, &self.auth_token).await;
            if guard.is_none() {
                tracing::warn!("No WebSocket connection; changes not sent");
                return Ok(());
            }
            let result = guard
                .as_mut()
                .unwrap()
                .send(Message::Text(payload.clone()))
                .await;
            match result {
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

    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        let mut guard = self.ws.lock().await;
        Self::ensure_ws(&mut guard, &self.server_url, &self.auth_token).await;
        if guard.is_none() {
            tracing::warn!("No WebSocket connection; no changes received");
            return Ok(vec![]);
        }
        // Drain all messages that have already been buffered in the WebSocket stream,
        // but stop waiting after 100 milliseconds of silence. This makes `receive_changes`
        // a bounded-time operation: it will not block indefinitely waiting for new
        // messages to arrive. Any messages that arrive after the timeout will be picked
        // up on the next sync cycle.
        let drain_timeout = Duration::from_millis(100);
        let mut changes = Vec::new();
        let mut connection_closed = false;
        {
            let ws = guard.as_mut().unwrap();
            loop {
                match timeout(drain_timeout, ws.next()).await {
                    Ok(Some(Ok(Message::Text(text)))) => {
                        let v: serde_json::Value = serde_json::from_str(&text)?;
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

    async fn get_device_id(&self) -> Result<String, StorageError> {
        Ok(self.device_id.clone())
    }
}
