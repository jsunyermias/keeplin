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
use tokio::sync::{Mutex, RwLock};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::{
    error::StorageError,
    links::{Bookmark, NoteLink},
    models::{new_id, now, Change, Note, NoteTag, Notebook, Resource, Tag},
};

use super::note_log::{self, resolve, VersionVector, Winner};
use super::{NoteRepository, NotebookRepository, ResourceRepository, SyncBackend, TagRepository};

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
///
/// ## Conflict resolution
///
/// `DbBackend` resolves concurrent edits with **last-write-wins by `updated_at`** for
/// every entity, notes included (see `apply_change`). It does **not** implement the
/// per-note version-vector merge that [`super::fs::FsBackend`] uses (see
/// [`super::note_log`]): if two devices edit the same note while offline and then sync,
/// the edit with the later `updated_at` wins and the other is overwritten without a
/// merge. Choose `FsBackend` (offline mode) when strong note-merge guarantees matter;
/// `DbBackend` trades that for a central WebSocket relay. This difference is documented
/// in `SECURITY.md`.
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
    /// Guards access to the shared `libsql::Connection` so that reads and writes are
    /// correctly isolated even though every operation runs on a **single** connection.
    ///
    /// Writers take the **write** (exclusive) side for the whole `BEGIN IMMEDIATE …
    /// COMMIT` span (and for bare writes); readers take the **read** (shared) side.
    /// This guarantees three things on the shared connection:
    /// - two `BEGIN IMMEDIATE`s never overlap (which would fail with "cannot start a
    ///   transaction within a transaction"),
    /// - a bare write never lands inside another task's open transaction, and
    /// - a query never observes another task's *uncommitted* rows mid-transaction (which
    ///   would otherwise be possible because all tasks share one connection).
    ///
    /// SQLite permits only one writer at a time regardless, so the exclusive write side
    /// costs no real throughput, while multiple readers still run concurrently.
    lock: Arc<RwLock<()>>,
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
            lock: Arc::new(RwLock::new(())),
        })
    }

    // ── Migrations ────────────────────────────────────────────────────────────

    /// Create all required tables and indexes if they do not already exist.
    ///
    /// All statements use `CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS`
    /// so this method is safe to call on every startup without checking the current
    /// schema version. Columns added after a table's first creation (the bookmark/link
    /// fields) are applied via [`add_column_if_missing`](Self::add_column_if_missing),
    /// which tolerates the column already existing.
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

        // Additive migration for databases created before the bookmark/link feature: the
        // `CREATE TABLE IF NOT EXISTS` above is a no-op on an existing table, so add the new
        // columns explicitly. `ADD COLUMN` errors with "duplicate column name" on a fresh
        // database (where the columns already exist) — that case is ignored.
        Self::add_column_if_missing(conn, "notes", "alias TEXT").await?;
        Self::add_column_if_missing(conn, "notes", "bookmarks TEXT NOT NULL DEFAULT '[]'").await?;
        Self::add_column_if_missing(conn, "notes", "links TEXT NOT NULL DEFAULT '[]'").await?;
        Self::add_column_if_missing(conn, "notebooks", "alias TEXT").await?;

        // Additive migration for the version-vector conflict-resolution columns (see
        // `note_log::resolve`). `vv` is the JSON per-device counter map, `last_writer` the
        // authoring device id used as the concurrent tiebreak. Pre-VV rows default to an empty
        // vector, so they behave like today until the next local write stamps them.
        for table in ["notes", "notebooks", "tags"] {
            Self::add_column_if_missing(conn, table, "vv TEXT NOT NULL DEFAULT '{}'").await?;
            Self::add_column_if_missing(conn, table, "last_writer TEXT NOT NULL DEFAULT ''")
                .await?;
        }
        // Versioned note↔tag associations: an add is the present state, a remove sets the
        // `deleted_at` tombstone; both resolve with the same version vectors as other entities.
        Self::add_column_if_missing(conn, "note_tags", "updated_at TEXT").await?;
        Self::add_column_if_missing(conn, "note_tags", "deleted_at TEXT").await?;
        Self::add_column_if_missing(conn, "note_tags", "vv TEXT NOT NULL DEFAULT '{}'").await?;
        Self::add_column_if_missing(conn, "note_tags", "last_writer TEXT NOT NULL DEFAULT ''")
            .await?;

        // Alias uniqueness is enforced at the application layer by `LinkingBackend`, which
        // checks the *plaintext* alias before a local write. A database UNIQUE index is
        // deliberately NOT used: under at-rest encryption the stored alias is ciphertext
        // with a fresh random nonce per write (so it never compares equal), and a hard
        // constraint would make `apply_change` error — instead of silently tolerating — a
        // duplicate alias arriving through sync, breaking the sync cycle.
        Ok(())
    }

    /// Run `ALTER TABLE {table} ADD COLUMN {column_def}`, treating a "duplicate column name"
    /// error (the column already exists, e.g. on a freshly created database) as success.
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

    /// Rebuild the `note_links` projection rows for `note`: clear the note's existing rows
    /// and insert one row per distinct resolved `target_note_id`. Called on every note write
    /// (create/update and applied sync changes) so backlinks stay indexed. Runs on the same
    /// connection as the surrounding note write.
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
        let alias: Option<String> = row.get(10)?;
        let bookmarks = json_to_bookmarks(&row.get::<String>(11)?);
        let links = json_to_links(&row.get::<String>(12)?);
        let vv = json_to_vv(&row.get::<String>(13)?);
        let last_writer: String = row.get(14)?;

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
            alias: row.get(5)?,
            vv: json_to_vv(&row.get::<String>(6)?),
            last_writer: row.get(7)?,
        })
    }

    fn row_to_tag(row: &libsql::Row) -> Result<Tag, StorageError> {
        Ok(Tag {
            id: Self::parse_uuid(row.get::<String>(0)?)?,
            title: row.get(1)?,
            created_at: Self::parse_required_dt(row.get::<String>(2)?)?,
            updated_at: Self::parse_required_dt(row.get::<String>(3)?)?,
            deleted_at: Self::parse_optional_dt(row.get::<Option<String>>(4)?)?,
            vv: json_to_vv(&row.get::<String>(5)?),
            last_writer: row.get(6)?,
        })
    }

    /// Convert a row from the `entity_changes` table into a typed [`Change`] variant.
    ///
    /// The arguments correspond to the `entity_type`, `entity_id`, `operation`,
    /// `changed_at`, and `data` columns of the `entity_changes` table. The `data`
    /// argument is a `serde_json::Value` already parsed from the stored JSON string (or
    /// `Null` when the column value is `NULL`); `changed_at` is the time the mutation was
    /// recorded and becomes the tombstone timestamp for delete variants.
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
            ("resource", "delete") => Some(Change::ResourceDelete { id }),
            _ => None,
        }
    }

    // ── Transaction helpers ───────────────────────────────────────────────────

    /// Start a `BEGIN IMMEDIATE` transaction.
    ///
    /// `IMMEDIATE` acquires a write lock at the start so that all subsequent writes
    /// succeed or fail atomically. Use `commit` to persist changes or `rollback` to
    /// discard them. Prefer wrapping multiple operations in a transaction so that a
    /// crash between the primary-table write and the `entity_changes` journal write
    /// cannot leave the two tables in an inconsistent state.
    async fn begin(&self) -> Result<(), StorageError> {
        self.conn.execute("BEGIN IMMEDIATE", ()).await?;
        Ok(())
    }

    /// Commit the current transaction and make all changes durable.
    async fn commit(&self) -> Result<(), StorageError> {
        self.conn.execute("COMMIT", ()).await?;
        Ok(())
    }

    /// Roll back the current transaction, discarding all changes since `begin`.
    ///
    /// Errors from `ROLLBACK` are intentionally swallowed (`.ok()`) because a rollback
    /// failure means no transaction was active — the database is already in a clean
    /// state and there is nothing to recover from.
    async fn rollback(&self) {
        self.conn.execute("ROLLBACK", ()).await.ok();
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

    /// Read the version-vector metadata `(vv, updated_at, last_writer)` of a row, or `None`
    /// when the row does not exist. Used by `apply_change` to feed [`resolve`].
    ///
    /// `table` is always one of the hard-coded literals `"notes"`, `"notebooks"`, or `"tags"`
    /// — never caller-supplied — so interpolating it into the query is safe.
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

    /// Decide whether an incoming remote write should replace the local row via [`resolve`]:
    /// `true` when there is no local row, or when the incoming `(vv, updated_at, last_writer)`
    /// wins the version-vector comparison against the stored one. This replaces the old bare
    /// `updated_at` last-write-wins, so concurrent edits converge deterministically.
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

    /// Compute the version vector for a **local** write to `(table, id)`: the current stored
    /// vector (or empty for a new row) with this device's component incremented. The caller
    /// stamps the entity with this vector and sets `last_writer = self.device_id`.
    async fn next_local_vv(&self, table: &str, id: &str) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .current_meta(table, id)
            .await?
            .map(|(vv, _, _)| vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    // ── note↔tag association version helpers ──────────────────────────────────

    /// Version metadata `(vv, updated_at, last_writer)` of a note↔tag association, or `None`
    /// when the pair has never been written. A pre-version row (NULL `updated_at`) is reported
    /// at the epoch so any real incoming write dominates it.
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

    /// Version vector for a **local** association write: current vector (empty if new) with this
    /// device's component incremented.
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

    /// Whether an incoming association write (add or remove) should replace the local state,
    /// via [`resolve`]. `true` when the pair has no local row.
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

    /// Upsert an association's versioned state: `deleted_at = None` for an add (present),
    /// `Some(ts)` for a remove (tombstone).
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
                    updated_at.to_rfc3339(),
                    deleted_at.map(|d| d.to_rfc3339()),
                    vv_to_json(vv),
                    last_writer.to_owned(),
                ],
            )
            .await?;
        Ok(())
    }
}

// ── Pagination helpers ────────────────────────────────────────────────────────

/// Parse a cursor token of the form `"<created_at_rfc3339>|<uuid>"` into its
/// two components. Returns `("", "")` when the token is absent or empty, which
/// causes the keyset SQL condition `?1 = ''` to match all rows (no offset).
fn parse_cursor(token: Option<&str>) -> (String, String) {
    match token.filter(|t| !t.is_empty()) {
        Some(cursor) => match cursor.split_once('|') {
            Some((ts, id)) => (ts.to_owned(), id.to_owned()),
            None => (String::new(), String::new()),
        },
        None => (String::new(), String::new()),
    }
}

/// Given a `rows` vec that was fetched with `LIMIT limit + 1`, return `(page, next_token)`.
///
/// If `rows.len() > limit`, there is a next page: `next_token` is built from
/// the last item of the actual page (index `limit - 1`) using `token_fn`.
/// The extra row is discarded so the returned page never exceeds `limit` items.
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

/// Serialise a note's bookmarks to the JSON stored in the `notes.bookmarks` column.
/// Serialisation of a plain `Vec` of small structs cannot fail in practice; `"[]"` is a
/// safe fallback that round-trips to an empty list.
fn bookmarks_to_json(bookmarks: &[Bookmark]) -> String {
    serde_json::to_string(bookmarks).unwrap_or_else(|_| "[]".to_string())
}

/// Serialise a note's links to the JSON stored in the `notes.links` column.
fn links_to_json(links: &[NoteLink]) -> String {
    serde_json::to_string(links).unwrap_or_else(|_| "[]".to_string())
}

/// Parse the `notes.bookmarks` JSON column; a malformed value yields an empty list rather
/// than failing the whole read.
fn json_to_bookmarks(s: &str) -> Vec<Bookmark> {
    serde_json::from_str(s).unwrap_or_default()
}

/// Parse the `notes.links` JSON column; a malformed value yields an empty list.
fn json_to_links(s: &str) -> Vec<NoteLink> {
    serde_json::from_str(s).unwrap_or_default()
}

/// Serialise a version vector to the JSON stored in the `vv` column (`"{}"` fallback).
fn vv_to_json(vv: &VersionVector) -> String {
    serde_json::to_string(vv).unwrap_or_else(|_| "{}".to_string())
}

/// Parse a `vv` JSON column; a malformed value yields an empty vector.
fn json_to_vv(s: &str) -> VersionVector {
    serde_json::from_str(s).unwrap_or_default()
}

/// Build the `entity_changes.data` JSON for a delete: the tombstone timestamp plus the
/// deleting write's version vector and author, so `row_to_change` can reconstruct a delete
/// `Change` that carries everything `resolve` needs on the receiving peer.
fn tombstone_data(deleted_at: DateTime<Utc>, vv: &VersionVector, last_writer: &str) -> String {
    serde_json::json!({
        "deleted_at": deleted_at,
        "vv": vv,
        "last_writer": last_writer,
    })
    .to_string()
}

/// Build the `entity_changes.data` JSON for a note↔tag add/remove: the other key plus the
/// association's version metadata, so `row_to_change` reconstructs a versioned association change.
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

/// Reconstruct an association change's `(updated_at, vv, last_writer)` from a journal `data`
/// value. Falls back to `changed_at` and empty vv/writer for pre-version records.
fn assoc_from_data(
    data: &serde_json::Value,
    changed_at: DateTime<Utc>,
) -> (DateTime<Utc>, VersionVector, String) {
    // Same shape as a tombstone minus the semantics; reuse the field extraction.
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

/// Reconstruct a delete's `(deleted_at, vv, last_writer)` from a journal `data` value.
/// Falls back to `changed_at` and empty vv/writer for pre-VV records that stored `NULL`.
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

// ── NoteRepository impl ───────────────────────────────────────────────────────

#[async_trait]
impl NoteRepository for DbBackend {
    async fn create_note(&self, mut note: Note) -> Result<Note, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        // Stamp this local write with a freshly incremented version vector so remote peers can
        // resolve it against their own edits (see `note_log::resolve`).
        note.vv = self.next_local_vv("notes", &note.id.to_string()).await?;
        note.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            self.conn
                .execute(
                    "INSERT INTO notes
                     (id, title, body, notebook_id, is_todo, todo_due, todo_completed, created_at, updated_at, deleted_at, alias, bookmarks, links, vv, last_writer)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
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
                        note.alias.clone(),
                        bookmarks_to_json(&note.bookmarks),
                        links_to_json(&note.links),
                        vv_to_json(&note.vv),
                        note.last_writer.clone(),
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

    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,body,notebook_id,is_todo,todo_due,todo_completed,
                        created_at,updated_at,deleted_at,alias,bookmarks,links,vv,last_writer
                 FROM notes WHERE id = ?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Self::row_to_note(&row),
            None => Err(StorageError::NotFound(id.to_string())),
        }
    }

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
                     alias=?10, bookmarks=?11, links=?12, vv=?13, last_writer=?14
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
                        note.alias.clone(),
                        bookmarks_to_json(&note.bookmarks),
                        links_to_json(&note.links),
                        vv_to_json(&note.vv),
                        note.last_writer.clone(),
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
                    libsql::params![id.to_string(), ts.to_rfc3339(), vv_to_json(&vv), writer.clone()],
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

    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = if page_size == 0 { 100u32 } else { page_size };
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,body,notebook_id,is_todo,todo_due,todo_completed,
                        created_at,updated_at,deleted_at,alias,bookmarks,links,vv,last_writer
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
            format!("{}|{}", n.created_at.to_rfc3339(), n.id)
        }))
    }

    async fn note_backlinks(
        &self,
        target_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        // Indexed lookup via the `note_links` projection joined back to live notes, instead
        // of the default full scan. `idx_note_links_target` makes the `WHERE` an index seek,
        // and a keyset cursor on `(created_at, id)` plus `LIMIT` keeps the response bounded.
        let _read_guard = self.lock.read().await;
        let limit = if page_size == 0 { 100u32 } else { page_size };
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT n.id,n.title,n.body,n.notebook_id,n.is_todo,n.todo_due,n.todo_completed,
                        n.created_at,n.updated_at,n.deleted_at,n.alias,n.bookmarks,n.links,n.vv,n.last_writer
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
            format!("{}|{}", n.created_at.to_rfc3339(), n.id)
        }))
    }
}

// ── NotebookRepository impl ───────────────────────────────────────────────────

#[async_trait]
impl NotebookRepository for DbBackend {
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
                        notebook.created_at.to_rfc3339(),
                        notebook.updated_at.to_rfc3339(),
                        notebook.deleted_at.map(|d| d.to_rfc3339()),
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
                        notebook.updated_at.to_rfc3339(),
                        notebook.deleted_at.map(|d| d.to_rfc3339()),
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
                    libsql::params![id.to_string(), ts.to_rfc3339(), vv_to_json(&vv), writer.clone()],
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

    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = if page_size == 0 { 100u32 } else { page_size };
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
            format!("{}|{}", nb.created_at.to_rfc3339(), nb.id)
        }))
    }
}

// ── TagRepository impl ────────────────────────────────────────────────────────

#[async_trait]
impl TagRepository for DbBackend {
    async fn create_tag(&self, mut tag: Tag) -> Result<Tag, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        tag.vv = self.next_local_vv("tags", &tag.id.to_string()).await?;
        tag.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            self.conn
                .execute(
                    "INSERT INTO tags (id,title,created_at,updated_at,deleted_at,vv,last_writer)
                     VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    libsql::params![
                        tag.id.to_string(),
                        tag.title.clone(),
                        tag.created_at.to_rfc3339(),
                        tag.updated_at.to_rfc3339(),
                        tag.deleted_at.map(|d| d.to_rfc3339()),
                        vv_to_json(&tag.vv),
                        tag.last_writer.clone(),
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

    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at,vv,last_writer
                 FROM tags WHERE id = ?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Self::row_to_tag(&row),
            None => Err(StorageError::NotFound(id.to_string())),
        }
    }

    async fn update_tag(&self, mut tag: Tag) -> Result<Tag, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        tag.vv = self.next_local_vv("tags", &tag.id.to_string()).await?;
        tag.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let affected = self
                .conn
                .execute(
                    "UPDATE tags SET title=?2,updated_at=?3,deleted_at=?4,vv=?5,last_writer=?6 WHERE id=?1",
                    libsql::params![
                        tag.id.to_string(),
                        tag.title.clone(),
                        tag.updated_at.to_rfc3339(),
                        tag.deleted_at.map(|d| d.to_rfc3339()),
                        vv_to_json(&tag.vv),
                        tag.last_writer.clone(),
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
                    libsql::params![id.to_string(), ts.to_rfc3339(), vv_to_json(&vv), writer.clone()],
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

    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = if page_size == 0 { 100u32 } else { page_size };
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at,vv,last_writer
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
            format!("{}|{}", t.created_at.to_rfc3339(), t.id)
        }))
    }

    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let note_id = note_tag.note_id.to_string();
        let tag_id = note_tag.tag_id.to_string();
        let vv = self.next_assoc_vv(&note_id, &tag_id).await?;
        let writer = self.device_id.clone();
        let ts = now();
        let r: Result<(), StorageError> = async {
            // An add is the association's *present* state (deleted_at = NULL), versioned so a
            // concurrent add-vs-remove converges through `resolve`.
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

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let note_id_s = note_id.to_string();
        let tag_id_s = tag_id.to_string();
        let vv = self.next_assoc_vv(&note_id_s, &tag_id_s).await?;
        let writer = self.device_id.clone();
        let ts = now();
        let r: Result<(), StorageError> = async {
            // A remove is a *tombstone* (deleted_at set), kept so it can beat a concurrent add.
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

    async fn list_note_tags(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = if page_size == 0 { 100u32 } else { page_size };
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT t.id,t.title,t.created_at,t.updated_at,t.deleted_at,t.vv,t.last_writer
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
            format!("{}|{}", t.created_at.to_rfc3339(), t.id)
        }))
    }
}

// ── ResourceRepository impl ───────────────────────────────────────────────────

#[async_trait]
impl ResourceRepository for DbBackend {
    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let r: Result<(), StorageError> = async {
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

    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let _read_guard = self.lock.read().await;
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
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let r: Result<(), StorageError> = async {
            let affected = self
                .conn
                .execute("DELETE FROM resources WHERE id=?1", [id.to_string()])
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(id.to_string()));
            }
            self.record_change("resource", &id.to_string(), "delete", None)
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

    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = if page_size == 0 { 100u32 } else { page_size };
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,mime_type,file_name,size,created_at
                 FROM resources
                 WHERE ?1 = '' OR created_at > ?2
                    OR (created_at = ?2 AND id > ?3)
                 ORDER BY created_at ASC, id ASC
                 LIMIT ?4",
                libsql::params![cursor_ts.clone(), cursor_ts, cursor_id, limit + 1],
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
        Ok(build_page(resources, limit as usize, |r| {
            format!("{}|{}", r.created_at.to_rfc3339(), r.id)
        }))
    }
}

// ── SyncBackend impl ──────────────────────────────────────────────────────────

#[async_trait]
impl SyncBackend for DbBackend {
    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError> {
        let _read_guard = self.lock.read().await;
        let since_str = since.to_rfc3339();
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

    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        // Applies a change pulled from the relay. Deliberately does NOT call `record_change`:
        // the `entity_changes` journal holds only changes that ORIGINATED on this device, so
        // `get_changes_since`/`send_changes` never re-send something we merely received. The
        // sync server is a broadcast relay (it forwards each device's change to every other
        // peer), so re-propagation is unnecessary; re-journaling applied changes would just
        // echo every change back out on the next cycle. Do not add `record_change` here
        // without also switching the relay away from broadcast — see `db.md`.
        //
        // Hold the write lock for the whole apply so this write cannot interleave with
        // another task's open transaction on the shared connection.
        let _write_guard = self.lock.write().await;
        match change {
            // Notes
            Change::NoteCreate { note } | Change::NoteUpdate { note } => {
                // Version-vector conflict resolution: apply only when the incoming write wins
                // against the local row (see `resolve`), so concurrent edits converge instead
                // of the old bare-`updated_at` last-write-wins.
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
                // Atomically refresh the `note_links` projection and upsert the note, so a
                // crash mid-apply cannot leave the index out of sync with the note row
                // (mirrors the create/update transactions; still idempotent on retry).
                self.begin().await?;
                let r: Result<(), StorageError> = async {
                    // Refresh the link index before the INSERT consumes the note's fields.
                    self.refresh_note_links(&note).await?;
                    self.conn
                        .execute(
                            "INSERT OR REPLACE INTO notes
                             (id,title,body,notebook_id,is_todo,todo_due,todo_completed,created_at,updated_at,deleted_at,alias,bookmarks,links,vv,last_writer)
                             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
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
                                note.alias.clone(),
                                bookmarks_to_json(&note.bookmarks),
                                links_to_json(&note.links),
                                vv_to_json(&note.vv),
                                note.last_writer.clone(),
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
                // Tombstone competes in `resolve` exactly like an edit (using `deleted_at` as
                // its timestamp), so a stale delete never overrides a newer edit and a causal
                // edit made after the delete can still revive the note. Store the tombstone's
                // own vv/writer so it can beat later concurrent edits deterministically.
                if !self
                    .incoming_wins("notes", &id.to_string(), &vv, deleted_at, &last_writer)
                    .await?
                {
                    return Ok(());
                }
                self.conn
                    .execute(
                        "UPDATE notes SET deleted_at = ?2, updated_at = ?2, vv = ?3, last_writer = ?4 WHERE id = ?1",
                        libsql::params![
                            id.to_string(),
                            deleted_at.to_rfc3339(),
                            vv_to_json(&vv),
                            last_writer,
                        ],
                    )
                    .await?;
            }
            // Notebooks
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
                            notebook.created_at.to_rfc3339(),
                            notebook.updated_at.to_rfc3339(),
                            notebook.deleted_at.map(|d| d.to_rfc3339()),
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
                self.conn
                    .execute(
                        "UPDATE notebooks SET deleted_at = ?2, updated_at = ?2, vv = ?3, last_writer = ?4 WHERE id = ?1",
                        libsql::params![id.to_string(), deleted_at.to_rfc3339(), vv_to_json(&vv), last_writer],
                    )
                    .await?;
            }
            // Tags
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
                            tag.created_at.to_rfc3339(),
                            tag.updated_at.to_rfc3339(),
                            tag.deleted_at.map(|d| d.to_rfc3339()),
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
                self.conn
                    .execute(
                        "UPDATE tags SET deleted_at = ?2, updated_at = ?2, vv = ?3, last_writer = ?4 WHERE id = ?1",
                        libsql::params![id.to_string(), deleted_at.to_rfc3339(), vv_to_json(&vv), last_writer],
                    )
                    .await?;
            }
            // NoteTag associations
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

    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
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
        // Reject sync batches that exceed this message count. Enforcing an upper bound
        // prevents a malicious or misbehaving server from exhausting the daemon's memory
        // by sending an unbounded stream of messages in a single receive call. Any
        // messages not consumed here will be delivered on the next sync cycle.
        const MAX_WS_MESSAGES: usize = 1_000;
        // Drain all messages that have already been buffered in the WebSocket stream,
        // but stop waiting after 100 milliseconds of silence. This makes `receive_changes`
        // a bounded-time operation: it will not block indefinitely waiting for new
        // messages to arrive. Any messages that arrive after the timeout will be picked
        // up on the next sync cycle.
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

    async fn prune_change_journal(&self, older_than: DateTime<Utc>) -> Result<u64, StorageError> {
        let _write_guard = self.lock.write().await;
        let affected = self
            .conn
            .execute(
                "DELETE FROM entity_changes WHERE changed_at < ?1",
                [older_than.to_rfc3339()],
            )
            .await?;
        tracing::info!(rows = affected, "Pruned entity_changes journal");
        Ok(affected)
    }

    async fn get_device_id(&self) -> Result<String, StorageError> {
        Ok(self.device_id.clone())
    }
}
