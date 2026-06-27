use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag, new_id, now},
};

use super::StorageBackend;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// ── DbBackend ─────────────────────────────────────────────────────────────────

/// Server-backed storage. Uses LibSQL for local persistence and a WebSocket
/// connection to a central server for real-time synchronisation.
pub struct DbBackend {
    conn: libsql::Connection,
    server_url: String,
    auth_token: String,
    ws: Arc<Mutex<Option<WsStream>>>,
    device_id: String,
}

impl DbBackend {
    pub async fn new(
        db_path: impl AsRef<std::path::Path>,
        server_url: impl Into<String>,
        auth_token: impl Into<String>,
    ) -> Result<Self, StorageError> {
        let server_url = server_url.into();
        let auth_token = auth_token.into();

        let db = libsql::Builder::new_local(db_path.as_ref())
            .build()
            .await?;
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

            CREATE INDEX IF NOT EXISTS idx_notes_updated_at  ON notes(updated_at);
            CREATE INDEX IF NOT EXISTS idx_note_tags_note_id ON note_tags(note_id);
            CREATE INDEX IF NOT EXISTS idx_note_tags_tag_id  ON note_tags(tag_id);
            ",
        )
        .await?;
        Ok(())
    }

    // ── Device ID ─────────────────────────────────────────────────────────────

    async fn get_or_create_device_id(conn: &libsql::Connection) -> Result<String, StorageError> {
        let mut rows = conn
            .query("SELECT id FROM device LIMIT 1", ())
            .await?;

        if let Some(row) = rows.next().await? {
            return Ok(row.get::<String>(0)?);
        }

        let id = new_id().to_string();
        conn.execute("INSERT INTO device (id) VALUES (?1)", [id.clone()])
            .await?;
        Ok(id)
    }

    // ── WebSocket ─────────────────────────────────────────────────────────────

    async fn connect_ws(url: &str, token: &str) -> Result<WsStream, StorageError> {
        let (mut stream, _) = connect_async(url).await?;
        // Send auth token as first message
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
        s.parse().map_err(|e: uuid::Error| StorageError::InvalidState(e.to_string()))
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
        Ok(())
    }

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        self.conn
            .execute(
                "DELETE FROM note_tags WHERE note_id=?1 AND tag_id=?2",
                [note_id.to_string(), tag_id.to_string()],
            )
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

    async fn create_resource(&self, resource: Resource, data: Vec<u8>) -> Result<Resource, StorageError> {
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
                let data: Vec<u8> = row.get(5)?;
                Ok((resource, data))
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
                "SELECT id,title,body,notebook_id,is_todo,todo_due,todo_completed,
                        created_at,updated_at,deleted_at
                 FROM notes WHERE updated_at > ?1",
                [since_str],
            )
            .await?;
        let mut changes = Vec::new();
        while let Some(row) = rows.next().await? {
            let note = Self::row_to_note(&row)?;
            let change = if note.deleted_at.is_some() {
                Change::Delete { id: note.id }
            } else if note.created_at > since {
                Change::Create { note }
            } else {
                Change::Update { note }
            };
            changes.push(change);
        }
        Ok(changes)
    }

    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        match change {
            Change::Create { note } => {
                // Use INSERT OR REPLACE to handle re-delivered creates
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
            Change::Update { note } => {
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
            Change::Delete { id } => {
                let ts = now().to_rfc3339();
                self.conn
                    .execute(
                        "UPDATE notes SET deleted_at = ?2 WHERE id = ?1",
                        [id.to_string(), ts],
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT value FROM sync_state WHERE key = 'last_sync'",
                (),
            )
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
        let mut guard = self.ws.lock().await;
        Self::ensure_ws(&mut guard, &self.server_url, &self.auth_token).await;
        if guard.is_none() {
            tracing::warn!("No WebSocket connection; changes not sent");
            return Ok(());
        }
        let n = changes.len();
        let payload = serde_json::json!({
            "type": "changes",
            "device_id": self.device_id,
            "changes": changes,
        });
        let result = {
            guard.as_mut().unwrap().send(Message::Text(payload.to_string())).await
        };
        match result {
            Ok(()) => tracing::info!(count = n, "Changes sent via WebSocket"),
            Err(e) => {
                *guard = None;
                return Err(StorageError::WebSocket(e.to_string()));
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
        // Drain all buffered messages; give up after 100 ms of silence.
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
                                tracing::info!(count = batch.len(), "Changes received via WebSocket");
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
