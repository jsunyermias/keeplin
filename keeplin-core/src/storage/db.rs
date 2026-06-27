use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::Mutex;
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
    #[allow(dead_code)]
    server_url: String,
    #[allow(dead_code)]
    auth_token: String,
    ws: Arc<Mutex<Option<WsStream>>>,
    device_id: String,
    #[allow(dead_code)]
    http_client: reqwest::Client,
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
            http_client: reqwest::Client::new(),
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
        let parse_dt = |s: Option<String>| -> Result<Option<DateTime<Utc>>, StorageError> {
            match s {
                None => Ok(None),
                Some(v) => v
                    .parse::<DateTime<Utc>>()
                    .map(Some)
                    .map_err(|e| StorageError::InvalidState(e.to_string())),
            }
        };
        let parse_required_dt = |s: String| -> Result<DateTime<Utc>, StorageError> {
            s.parse::<DateTime<Utc>>()
                .map_err(|e| StorageError::InvalidState(e.to_string()))
        };

        let id: Uuid = row
            .get::<String>(0)?
            .parse()
            .map_err(|e: uuid::Error| StorageError::Database(e.to_string()))?;
        let title: String = row.get(1)?;
        let body: String = row.get(2)?;
        let notebook_id: Option<Uuid> = row
            .get::<Option<String>>(3)?
            .as_deref()
            .map(|s| s.parse().map_err(|e: uuid::Error| StorageError::Database(e.to_string())))
            .transpose()?;
        let is_todo: bool = row.get::<i64>(4)? != 0;
        let todo_due = parse_dt(row.get::<Option<String>>(5)?)?;
        let todo_completed = parse_dt(row.get::<Option<String>>(6)?)?;
        let created_at = parse_required_dt(row.get::<String>(7)?)?;
        let updated_at = parse_required_dt(row.get::<String>(8)?)?;
        let deleted_at = parse_dt(row.get::<Option<String>>(9)?)?;

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

    // ── Notebooks (deferred) ──────────────────────────────────────────────────

    async fn create_notebook(&self, _notebook: Notebook) -> Result<Notebook, StorageError> {
        unimplemented!("Notebook support is planned for a later phase")
    }

    async fn read_notebook(&self, _id: Uuid) -> Result<Notebook, StorageError> {
        unimplemented!("Notebook support is planned for a later phase")
    }

    async fn update_notebook(&self, _notebook: Notebook) -> Result<Notebook, StorageError> {
        unimplemented!("Notebook support is planned for a later phase")
    }

    async fn delete_notebook(&self, _id: Uuid) -> Result<(), StorageError> {
        unimplemented!("Notebook support is planned for a later phase")
    }

    async fn list_notebooks(&self) -> Result<Vec<Notebook>, StorageError> {
        unimplemented!("Notebook support is planned for a later phase")
    }

    // ── Tags (deferred) ───────────────────────────────────────────────────────

    async fn create_tag(&self, _tag: Tag) -> Result<Tag, StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn read_tag(&self, _id: Uuid) -> Result<Tag, StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn update_tag(&self, _tag: Tag) -> Result<Tag, StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn delete_tag(&self, _id: Uuid) -> Result<(), StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn list_tags(&self) -> Result<Vec<Tag>, StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn add_note_tag(&self, _note_tag: NoteTag) -> Result<(), StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn remove_note_tag(&self, _note_id: Uuid, _tag_id: Uuid) -> Result<(), StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn list_note_tags(&self, _note_id: Uuid) -> Result<Vec<Tag>, StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    // ── Resources (deferred) ──────────────────────────────────────────────────

    async fn create_resource(
        &self,
        _resource: Resource,
        _data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        unimplemented!("Resource support is planned for a later phase")
    }

    async fn read_resource(&self, _id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        unimplemented!("Resource support is planned for a later phase")
    }

    async fn delete_resource(&self, _id: Uuid) -> Result<(), StorageError> {
        unimplemented!("Resource support is planned for a later phase")
    }

    async fn list_resources(&self) -> Result<Vec<Resource>, StorageError> {
        unimplemented!("Resource support is planned for a later phase")
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
        match guard.as_mut() {
            None => {
                tracing::warn!("No WebSocket connection; changes not sent");
            }
            Some(ws) => {
                let payload = serde_json::json!({
                    "type": "changes",
                    "device_id": self.device_id,
                    "changes": changes,
                });
                ws.send(Message::Text(payload.to_string())).await?;
                tracing::info!(count = changes.len(), "Changes sent via WebSocket");
            }
        }
        Ok(())
    }

    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        let mut guard = self.ws.lock().await;
        match guard.as_mut() {
            None => {
                tracing::warn!("No WebSocket connection; no changes received");
                Ok(vec![])
            }
            Some(ws) => {
                // Non-blocking: drain all currently buffered messages
                let mut changes = Vec::new();
                while let Some(msg) = ws.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
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
                        Ok(Message::Close(_)) | Err(_) => break,
                        _ => {}
                    }
                }
                Ok(changes)
            }
        }
    }

    async fn get_device_id(&self) -> Result<String, StorageError> {
        Ok(self.device_id.clone())
    }
}
