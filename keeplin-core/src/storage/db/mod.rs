// md:Overview
mod conflict;
mod convert;
mod migrations;
mod notebooks;
mod notes;
mod resources;
mod rows;
mod server;
mod sync;
mod tags;

use std::sync::Arc;

use futures_util::SinkExt;
use tokio::sync::{Mutex, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::{
    error::StorageError,
    models::{new_id, now, Note},
};

use crate::storage::SortableRfc3339;

// md:re-exports
pub(super) use super::{effective_page_size, NotebookSortProfile};
use server::{http_base_of, CapabilityCache};

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

    const SCHEMA_VERSION: u32 = 5;

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
}
