# `storage/db.rs` — DbBackend (LibSQL + WebSocket storage)

Self-contained companion for `keeplin-core/src/storage/db.rs`. It documents **every
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

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The LibSQL-backed `StorageBackend` with WebSocket
synchronisation. All data lives in a local SQLite-compatible database; every
mutation also appends a row to the `entity_changes` journal so
`get_changes_since` can enumerate ordered mutations after any instant. Binary
resource payloads are stored as BLOBs and embedded in the journal as Base64
(`_data_b64`) so peers can reconstruct resources from the journal alone.
Conflict resolution is **version vectors for every entity**
(`note_log::resolve` over the stored vs incoming `(vv, updated_at, last_writer)`)
— the same decision procedure `FsBackend` uses via log-based `merge`; only the
storage shape differs (see `SECURITY.md`).

**Dependencies** — `libsql`, `tokio_tungstenite`, `reqwest` (server history +
`/version`), `base64`, `serde_json`; `note_log`, the trait family,
`SortableRfc3339` (every stored timestamp uses the fixed nine-digit RFC 3339
shape so lexicographic = chronological).

**Used by** — `keeplin-daemon/src/main.rs` (`storage = "database"` mode),
`migrate.rs`, the DbBackend integration tests (`tests/db_backend.rs`,
`tests/ws_sync.rs`, `tests/sync.rs`).

**Repeated context** — the storage conventions restated: soft-delete-always,
idempotent `apply_change` (equal vectors → `Winner::Local` → no-op), cursor
pagination (`"<ts>|<uuid>"` keyset), and the `(timestamp, device_id)` LWW
tiebreak.

---

## WsStream

**Identification** —
`type WsStream = tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>;`
marker `// md:WsStream`.

**Code** — complete and verbatim:

```rust
// md:WsStream
type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
```

**What it does** — The WebSocket stream over plain TCP or TLS —
`MaybeTlsStream` handles both, so `ws://` and `wss://` need no type change.

**Dependencies** — `tokio_tungstenite`. **Used by** — the `ws` field,
`connect_ws`, `ensure_ws`. **Repeated context** — none.

---

## DbBackend

**Identification** — `pub struct DbBackend`; marker `// md:DbBackend`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The backend's state: `conn` (the open LibSQL connection),
`server_url` (`ws://`/`wss://`; empty = offline mode, no WebSocket),
`auth_token` (bearer token sent as the first WebSocket message; kept so
`ensure_ws` can re-authenticate on reconnect), `ws: Arc<Mutex<Option<WsStream>>>`
(`None` = not configured or lost), `device_id` (permanent installation UUID from
the `device` table; sent in every change batch), `http` (REST client for
history/`/version`), `history_unsupported: AtomicBool` (latched once the server
answers a history request with 404 — subsequent reads skip the wasted round-trip,
issue #113; a network error does **not** latch it),
`server_capabilities: Mutex<CapabilityCache>` (cached `GET /version` outcome,
keeplin#114), and `lock: Arc<RwLock<()>>` — the connection guard: writers take
the exclusive side for whole `BEGIN IMMEDIATE … COMMIT` spans (and bare writes),
readers the shared side, guaranteeing on the single shared connection that two
`BEGIN IMMEDIATE`s never overlap, a bare write never lands inside another task's
open transaction, and a query never observes uncommitted rows. SQLite allows
only one writer anyway, so the exclusive side costs no real throughput.

If the WebSocket fails, the local database keeps working and changes accumulate
in `entity_changes`; a `send_changes` that cannot deliver **errors** so the sync
cycle aborts without advancing the watermark — undelivered changes are re-sent
next cycle, never silently skipped.

**Dependencies** — `libsql`, `tokio` sync, `reqwest`.

**Used by** — everything below.

**Repeated context** — none.

---

## impl DbBackend

**Identification** — the first inherent impl; marker `// md:impl DbBackend`.
Constructor, migrations, journal/WS/row/versioning helpers.

**Code** — container: members documented as sub-blocks below: fn new, fn run_migrations, fn schema_version, fn apply_migration, fn migrate_v1_baseline, fn add_column_if_missing, fn get_or_create_device_id, fn record_change, fn refresh_note_links, fn connect_ws, fn row_to_note, fn parse_uuid, fn parse_required_dt, fn parse_optional_dt, fn row_to_notebook, fn row_to_tag, fn row_to_resource, fn row_to_change, fn begin, fn commit, fn rollback, fn ensure_ws, fn migrate_v2_ordering, fn current_meta, fn incoming_wins, fn next_local_vv, fn row_is_live, fn assoc_meta, fn next_assoc_vv, fn assoc_incoming_wins, fn upsert_assoc, fn resource_meta, fn next_resource_vv, fn resource_incoming_wins.

### fn new

**Identification** —
`pub async fn new(db_path, server_url, auth_token) -> Result<Self, StorageError>`;
marker `// md:impl DbBackend > fn new`.

**Code** — complete and verbatim:

```rust
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

    const SCHEMA_VERSION: u32 = 2;
```

**What it does** — Opens (or creates) the database, runs `run_migrations`, loads
or creates the device id, builds a 10 s-timeout HTTP client, then — when a
server is configured — runs the **`GET /version` protocol handshake** before any
sync: `Compatible` logs and primes the capability cache; `Incompatible` fails
construction loudly with the actionable `incompatible_message`; `Unavailable`
warns and continues (older keeplin-srv / bare test relay). Finally attempts the
WebSocket connection — a failure is a **non-fatal warning** (offline mode), never
a constructor error.

**Dependencies** — `libsql::Builder`, `run_migrations`,
`get_or_create_device_id`, `compat::negotiate`, `connect_ws`, `http_base_of`.

**Used by** — `main.rs::build_storage`; `migrate` subcommand; tests.

**Repeated context** — none.

### fn run_migrations

**Identification** — `async fn run_migrations(conn) -> Result<(), StorageError>`;
marker `// md:impl DbBackend > fn run_migrations`.

**Code** — complete and verbatim:

```rust
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

### fn schema_version

**Identification** — marker `// md:impl DbBackend > fn schema_version`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend > fn schema_version
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

### fn apply_migration

**Identification** — marker `// md:impl DbBackend > fn apply_migration`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Dispatches the step that advances **to** `version`: 1 →
`migrate_v1_baseline`, 2 → `migrate_v2_ordering`, 3 → `migrate_v3_tag_system`,
anything else → `InvalidState`. The caller wraps it in a transaction and bumps
the stamp. `SCHEMA_VERSION` is now `3`.

**Used by** — `run_migrations`.

### fn migrate_v1_baseline

**Identification** — marker `// md:impl DbBackend > fn migrate_v1_baseline`.

**Code** — complete and verbatim:

```rust
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

### fn add_column_if_missing

**Identification** — marker `// md:impl DbBackend > fn add_column_if_missing`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `ALTER TABLE … ADD COLUMN …`, treating "duplicate column
name" as success.

**Used by** — the migration steps.

### fn get_or_create_device_id

**Identification** — marker `// md:impl DbBackend > fn get_or_create_device_id`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Reads the single `device` row, or inserts a fresh UUID v4 on
first startup. Included in every change batch so the relay keeps a per-device
delivery cursor and never echoes a device's own changes back.

**Used by** — `new`.

### fn record_change

**Identification** — marker `// md:impl DbBackend > fn record_change`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Inserts one `entity_changes` row (`entity_type` ∈ note/
notebook/tag/note_tag/resource; `operation` ∈ create/update/delete/add/remove;
`changed_at = now()` in sortable form; `data` = full entity JSON or `None`).
Called by every mutating method inside the same transaction as the primary
write, so the pair commits or rolls back together.

**Used by** — every write path below.

### fn refresh_note_links

**Identification** — marker `// md:impl DbBackend > fn refresh_note_links`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Rebuilds the `note_links` projection for one note: delete
its rows, insert one per distinct resolved `target_note_id`. Called on every
note write (local and applied sync) so backlinks stay indexed.

**Used by** — `create_note`, `update_note`, `apply_change`.

### fn connect_ws

**Identification** — marker `// md:impl DbBackend > fn connect_ws`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Opens the WebSocket and performs the application-level
handshake: first message `{"type":"auth","token":…,"device_id":…}`. The server
validates the token or closes; a later closure is detected by
`send_changes`/`receive_changes`, which clear the slot and trigger a reconnect.
The `device_id` lets the relay keep a per-device delivery cursor and replay
missed batches on reconnect (keeplin-srv's durable journal); older relays
ignore it. **Security note**: the token travels in the socket — use `wss://` in
production.

**Used by** — `new`, `ensure_ws`.

### fn row_to_note

**Identification** — marker `// md:impl DbBackend > fn row_to_note`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Maps an 18-column `notes` row (the fixed SELECT order used
everywhere) to a `Note`: NULL `notebook_id` → nil UUID (Inbox), JSON columns
via the lenient `json_to_*` parsers, `sort_key` clamped non-negative.

**Used by** — every note read/list.

### fn parse_uuid

**Identification** — marker `// md:impl DbBackend > fn parse_uuid`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend > fn parse_uuid
    fn parse_uuid(s: String) -> Result<Uuid, StorageError> {
        s.parse()
            .map_err(|e: uuid::Error| StorageError::InvalidState(e.to_string()))
    }
```

**What it does** — `String → Uuid`, mapping failure to `InvalidState`
(corrupted row, server bug — not a caller error).

### fn parse_required_dt

**Identification** — marker `// md:impl DbBackend > fn parse_required_dt`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend > fn parse_required_dt
    fn parse_required_dt(s: String) -> Result<DateTime<Utc>, StorageError> {
        s.parse::<DateTime<Utc>>()
            .map_err(|e| StorageError::InvalidState(e.to_string()))
    }
```

**What it does** — `String → DateTime<Utc>`, failure → `InvalidState`.

### fn parse_optional_dt

**Identification** — marker `// md:impl DbBackend > fn parse_optional_dt`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `Option<String> → Option<DateTime<Utc>>`, failure →
`InvalidState`.

### fn row_to_notebook

**Identification** — marker `// md:impl DbBackend > fn row_to_notebook`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Maps the 8-column `notebooks` row shape.

### fn row_to_tag

**Identification** — marker `// md:impl DbBackend > fn row_to_tag`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Maps the 8-column `tags` row shape. Column `7` is `system`, stored as
an SQLite integer and read back as a bool (`!= 0`). Every `SELECT` that feeds this mapper
(`read_tag`, `list_tags`) must therefore select the columns in this exact order, `system`
last.

### fn row_to_resource

**Identification** — marker `// md:impl DbBackend > fn row_to_resource`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Maps the 9-column metadata row shape (no `data` BLOB).

### fn row_to_change

**Identification** — marker `// md:impl DbBackend > fn row_to_change`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Converts an `entity_changes` row into a typed `Change`:
create/update deserialise the full JSON; deletes reconstruct
`(deleted_at, vv, last_writer)` via `tombstone_from_data` (with `changed_at` as
fallback); note_tag add/remove via `assoc_from_data`; resource creates decode
`_data_b64` into `ResourceCreate.data`. Returns `None` for unknown
`(entity_type, operation)` pairs (a future build's rows) — callers log and skip
without aborting the sync.

**Used by** — `get_changes_since`.

### fn begin

**Identification** — marker `// md:impl DbBackend > fn begin`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend > fn begin
    async fn begin(&self) -> Result<(), StorageError> {
        self.conn.execute("BEGIN IMMEDIATE", ()).await?;
        Ok(())
    }
```

**What it does** — `BEGIN IMMEDIATE` (write lock up front so all writes in the
span commit or roll back atomically — the primary write and the journal row can
never diverge).

### fn commit

**Identification** — marker `// md:impl DbBackend > fn commit`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend > fn commit
    async fn commit(&self) -> Result<(), StorageError> {
        self.conn.execute("COMMIT", ()).await?;
        Ok(())
    }
```

**What it does** — `COMMIT`.

### fn rollback

**Identification** — marker `// md:impl DbBackend > fn rollback`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend > fn rollback
    async fn rollback(&self) {
        self.conn.execute("ROLLBACK", ()).await.ok();
    }
```

**What it does** — `ROLLBACK`, errors swallowed (a rollback failure means no
transaction was active — already clean).

### fn ensure_ws

**Identification** — marker `// md:impl DbBackend > fn ensure_ws`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Reconnects when the slot is empty and a URL is configured;
on failure the slot stays `None` and the caller skips the network operation
(changes accumulate locally).

**Used by** — `send_changes`, `receive_changes`.

### fn migrate_v2_ordering

**Identification** — marker `// md:impl DbBackend > fn migrate_v2_ordering`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Migration v2: `is_pinned`/`is_starred`/`sort_key` columns
(defaults keep old rows valid — `sort_key 0` is the never-positioned sentinel);
existing `NULL notebook_id` rows are moved to the Inbox (nil UUID) so queries
never see NULL again; and the `(notebook_id, sort_key, id)` index behind
`list_notes_in_notebook`.

**Used by** — `apply_migration`.

### fn migrate_v3_tag_system

**Identification** — marker `// md:impl DbBackend > fn migrate_v3_tag_system`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend > fn migrate_v3_tag_system
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

### fn current_meta

**Identification** — marker `// md:impl DbBackend > fn current_meta`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Reads `(vv, updated_at, last_writer)` of a row in
`notes`/`notebooks`/`tags` (hard-coded table literals — interpolation is safe),
or `None` when absent. Feeds `resolve`.

**Used by** — `incoming_wins`, `next_local_vv`.

### fn incoming_wins

**Identification** — marker `// md:impl DbBackend > fn incoming_wins`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Whether an incoming remote write replaces the local row:
`true` with no local row, else `resolve(local, incoming) == Winner::Incoming`.
Replaces the old bare-`updated_at` LWW so concurrent edits converge
deterministically.

**Used by** — `apply_change` (notes/notebooks/tags).

### fn next_local_vv

**Identification** — marker `// md:impl DbBackend > fn next_local_vv`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The vector for a **local** write: current stored vector (or
empty) with this device's component incremented; the caller stamps the entity
and sets `last_writer = device_id`.

**Used by** — every local create/update/delete.

### fn row_is_live

**Identification** — marker `// md:impl DbBackend > fn row_is_live`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Whether a row exists with `deleted_at IS NULL`
(`notes`/`tags` literals only). Used to refuse dangling associations.

**Used by** — `add_note_tag`.

### fn assoc_meta

**Identification** — marker `// md:impl DbBackend > fn assoc_meta`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Version metadata of a note↔tag association; a pre-version
row (NULL `updated_at`) is reported at the epoch so any real write dominates.

**Used by** — `next_assoc_vv`, `assoc_incoming_wins`.

### fn next_assoc_vv

**Identification** — marker `// md:impl DbBackend > fn next_assoc_vv`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Local association write vector (current + increment).

**Used by** — `add_note_tag`, `remove_note_tag`.

### fn assoc_incoming_wins

**Identification** — marker `// md:impl DbBackend > fn assoc_incoming_wins`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `resolve` for association writes; `true` when the pair has
no row.

**Used by** — `apply_change` (NoteTagAdd/Remove).

### fn upsert_assoc

**Identification** — marker `// md:impl DbBackend > fn upsert_assoc`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `INSERT OR REPLACE` of the association's versioned state:
`deleted_at = NULL` for an add (present), `Some(ts)` for a remove (tombstone).

**Used by** — `add_note_tag`, `remove_note_tag`, `apply_change`.

### fn resource_meta

**Identification** — marker `// md:impl DbBackend > fn resource_meta`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Resource version metadata; resources have no `updated_at`,
so the tiebreak timestamp is `deleted_at` when tombstoned else `created_at`.

**Used by** — `next_resource_vv`, `resource_incoming_wins`.

### fn next_resource_vv

**Identification** — marker `// md:impl DbBackend > fn next_resource_vv`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Local resource write vector (current + increment).

**Used by** — `create_resource`, `delete_resource`.

### fn resource_incoming_wins

**Identification** — marker `// md:impl DbBackend > fn resource_incoming_wins`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `resolve` for resource changes; `true` with no local row.

**Used by** — `apply_change` (ResourceCreate/Delete).

---

## fn parse_cursor

**Identification** — `fn parse_cursor(token: Option<&str>) -> (String, String)`;
marker `// md:fn parse_cursor`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Splits a `"<created_at>|<uuid>"` cursor into its parts;
absent/empty/malformed → `("", "")`, which makes the keyset SQL condition
`?1 = ''` match all rows (no offset).

**Used by** — every list method. **Repeated context** — none.

---

## fn build_page

**Identification** —
`fn build_page<T, F>(rows: Vec<T>, limit: usize, token_fn: F) -> (Vec<T>, Option<String>)`;
marker `// md:fn build_page`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Turns a `LIMIT limit + 1` fetch into `(page, next_token)`:
more than `limit` rows ⇒ truncate and build the token from the page's last item;
otherwise no token.

**Used by** — every list method. **Repeated context** — none.

---

## fn bookmarks_to_json

**Identification** — marker `// md:fn bookmarks_to_json`.

**Code** — complete and verbatim:

```rust
// md:fn bookmarks_to_json
fn bookmarks_to_json(bookmarks: &[Bookmark]) -> String {
    serde_json::to_string(bookmarks).unwrap_or_else(|_| "[]".to_string())
}
```

**What it does** — Serialises `notes.bookmarks` (`"[]"` fallback — a `Vec` of
small structs cannot fail in practice).

---

## fn links_to_json

**Identification** — marker `// md:fn links_to_json`.

**Code** — complete and verbatim:

```rust
// md:fn links_to_json
fn links_to_json(links: &[NoteLink]) -> String {
    serde_json::to_string(links).unwrap_or_else(|_| "[]".to_string())
}
```

**What it does** — Serialises `notes.links` (`"[]"` fallback).

---

## fn json_to_bookmarks

**Identification** — marker `// md:fn json_to_bookmarks`.

**Code** — complete and verbatim:

```rust
// md:fn json_to_bookmarks
fn json_to_bookmarks(s: &str) -> Vec<Bookmark> {
    serde_json::from_str(s).unwrap_or_default()
}
```

**What it does** — Parses the bookmarks column; malformed → empty list rather
than failing the read.

---

## fn json_to_links

**Identification** — marker `// md:fn json_to_links`.

**Code** — complete and verbatim:

```rust
// md:fn json_to_links
fn json_to_links(s: &str) -> Vec<NoteLink> {
    serde_json::from_str(s).unwrap_or_default()
}
```

**What it does** — Parses the links column; malformed → empty list.

---

## fn vv_to_json

**Identification** — marker `// md:fn vv_to_json`.

**Code** — complete and verbatim:

```rust
// md:fn vv_to_json
fn vv_to_json(vv: &VersionVector) -> String {
    serde_json::to_string(vv).unwrap_or_else(|_| "{}".to_string())
}
```

**What it does** — Serialises a version vector (`"{}"` fallback).

---

## fn json_to_vv

**Identification** — marker `// md:fn json_to_vv`.

**Code** — complete and verbatim:

```rust
// md:fn json_to_vv
fn json_to_vv(s: &str) -> VersionVector {
    serde_json::from_str(s).unwrap_or_default()
}
```

**What it does** — Parses a `vv` column; malformed → empty vector (behaves as
an uninformed write).

---

## fn tombstone_data

**Identification** — marker `// md:fn tombstone_data`.

**Code** — complete and verbatim:

```rust
// md:fn tombstone_data
fn tombstone_data(deleted_at: DateTime<Utc>, vv: &VersionVector, last_writer: &str) -> String {
    serde_json::json!({
        "deleted_at": deleted_at,
        "vv": vv,
        "last_writer": last_writer,
    })
    .to_string()
}
```

**What it does** — Builds the journal `data` JSON for a delete:
`deleted_at` + the deleting write's `vv`/`last_writer`, so `row_to_change`
reconstructs a delete `Change` carrying everything `resolve` needs on the
receiving peer.

**Used by** — every delete path.

---

## fn assoc_data

**Identification** — marker `// md:fn assoc_data`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Journal `data` JSON for a note↔tag add/remove: `tag_id` +
version metadata.

**Used by** — `add_note_tag`, `remove_note_tag`.

---

## fn assoc_from_data

**Identification** — marker `// md:fn assoc_from_data`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Reconstructs `(updated_at, vv, last_writer)` from a journal
value, falling back to `changed_at` and empty vv/writer for pre-version
records.

**Used by** — `row_to_change`.

---

## fn tombstone_from_data

**Identification** — marker `// md:fn tombstone_from_data`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Reconstructs `(deleted_at, vv, last_writer)` from a journal
value, same fallbacks.

**Used by** — `row_to_change`.

---

## impl NoteRepository for DbBackend

**Identification** — marker `// md:impl NoteRepository for DbBackend`; each
method carries `// md:impl NoteRepository for DbBackend > fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_note, fn read_note, fn update_note, fn delete_note, fn list_notes, fn note_backlinks, fn list_notes_in_notebook, fn list_starred_notes, fn notebook_sort_profile.

**What it does** — the note surface. Common write pattern: exclusive lock →
`begin` → stamp `vv = next_local_vv` + `last_writer = device_id` → primary
write (+ `refresh_note_links` for notes) → `record_change` with the full
snapshot → `commit` (or `rollback` on any error).

### fn create_note

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn create_note`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Plain `INSERT` (an existing id errors — fresh-destination
contract), links projection refresh, `"create"` journal row with the full
snapshot; returns the stamped note.

### fn read_note

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn read_note`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Single-row SELECT (18 columns); `NotFound` when absent.
Note: tombstoned rows **are** returned (needed for resolution and revival);
user-facing layers re-check `deleted_at`.

### fn update_note

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn update_note`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `UPDATE … WHERE id` (0 rows → `NotFound`), links refresh,
`"update"` journal row.

### fn delete_note

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn delete_note`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Soft delete: `deleted_at = updated_at = now`, bumped vv,
`"delete"` journal row with `tombstone_data`.

### fn list_notes

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn list_notes`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Live notes in `(created_at, id)` order with the
`"<ts>|<id>"` keyset cursor, `LIMIT limit + 1` + `build_page`.

### fn note_backlinks

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn note_backlinks`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The indexed override of the trait default: `note_links`
joined back to live notes (`idx_note_links_target` makes the WHERE an index
seek), keyset cursor on `(created_at, id)`.

### fn list_notes_in_notebook

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn list_notes_in_notebook`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — One notebook's live notes ordered by the **effective** sort
key — the SQL `CASE WHEN sort_key = 0 THEN 1000 …` mirrors
`Note::effective_sort_key`, and the cursor carries the effective key compared
numerically (`CAST`).

### fn list_starred_notes

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn list_starred_notes`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `is_starred = 1` live notes, `(created_at, id)` keyset.

### fn notebook_sort_profile

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn notebook_sort_profile`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Keys-only scan (`idx_notes_notebook_sort`) mapped through
the 0→`DEFAULT_SORT_KEY` sentinel into
`NotebookSortProfile::from_effective_keys`.

---

## impl NotebookRepository for DbBackend

**Identification** — marker `// md:impl NotebookRepository for DbBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_notebook, fn read_notebook, fn update_notebook, fn delete_notebook, fn list_notebooks.

**What it does** — the notebook CRUD, same transactional write pattern.

### fn create_notebook

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn create_notebook`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Stamped `INSERT` + `"create"` journal row.

### fn read_notebook

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn read_notebook`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Single-row SELECT; `NotFound` when absent.

### fn update_notebook

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn update_notebook`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Stamped `UPDATE` (0 rows → `NotFound`) + `"update"` row.

### fn delete_notebook

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn delete_notebook`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Soft delete + tombstone journal row.

### fn list_notebooks

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn list_notebooks`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Live notebooks, `(created_at, id)` keyset.

---

## impl TagRepository for DbBackend

**Identification** — marker `// md:impl TagRepository for DbBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_tag, fn read_tag, fn update_tag, fn delete_tag, fn list_tags, fn add_note_tag, fn remove_note_tag, fn list_note_tags.

**What it does** — tags + versioned associations.

### fn create_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn create_tag`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Stamped `INSERT` + journal row. `system` is persisted as `?8`
(`tag.system as i64`); the journal `data` is the full serde JSON of the tag, so `system`
travels in the change feed too.

### fn read_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn read_tag`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Single-row SELECT; `NotFound` when absent.

### fn update_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn update_tag`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Stamped `UPDATE` (0 rows → `NotFound`) + journal row.

### fn delete_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn delete_tag`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Soft delete + tombstone journal row.

### fn list_tags

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn list_tags`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Live tags, `(created_at, id)` keyset.

### fn add_note_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn add_note_tag`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Verifies **both ends are live** (`row_is_live`; `NotFound`
otherwise — the API must not create dangling associations; `apply_change`
deliberately skips this because sync delivery order is not guaranteed), then
`upsert_assoc` with `deleted_at = NULL` (the present state, versioned so a
concurrent add-vs-remove converges) + an `"add"` journal row. Idempotent.

### fn remove_note_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn remove_note_tag`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `upsert_assoc` with a tombstone (kept so it can beat a
concurrent add) + a `"remove"` journal row. Idempotent.

### fn list_note_tags

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn list_note_tags`.

**Code** — complete and verbatim:

```rust
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
            format!("{}|{}", t.created_at.to_sortable_rfc3339(), t.id)
        }))
    }
```

**What it does** — Tags joined through live (`nt.deleted_at IS NULL`)
associations, `(created_at, id)` keyset.

---

## impl ResourceRepository for DbBackend

**Identification** — marker `// md:impl ResourceRepository for DbBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn create_resource, fn read_resource, fn delete_resource, fn list_resources, fn purge_deleted_resources.

**What it does** — resources with BLOB payloads.

### fn create_resource

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn create_resource`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Stamped `INSERT` storing the BLOB, plus a `"create"` journal
row whose JSON carries `_data_b64` (the Base64 payload) so peers receiving the
change via the relay reconstruct the full resource without a separate download.

### fn read_resource

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn read_resource`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Metadata + BLOB; a tombstoned resource reads as `NotFound`
(before touching data).

### fn delete_resource

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn delete_resource`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Soft delete: tombstone + bumped vector; the payload is
retained (reclaim is `purge_deleted_resources`); tombstone journal row.

### fn list_resources

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn list_resources`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Live metadata (no BLOBs), `(created_at, id)` keyset.

### fn purge_deleted_resources

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn purge_deleted_resources`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `UPDATE … SET data = NULL` for tombstones older than the
cutoff — frees the dead bytes but keeps the tombstone row (`deleted_at`, vv,
`last_writer`) so the deletion keeps converging; `size` remains as a record.

---

## impl SyncBackend for DbBackend

**Identification** — marker `// md:impl SyncBackend for DbBackend`; per-method
markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn get_changes_since, fn apply_change, fn get_last_sync_time, fn update_sync_time, fn send_changes, fn receive_changes, fn prune_change_journal, fn get_device_id.

**What it does** — the journal + WebSocket sync surface.

### fn get_changes_since

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn get_changes_since`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Journal rows with `changed_at > since` in insertion order
(`ORDER BY id`), each mapped through `row_to_change`; unknown rows are logged
and skipped, never abort the sync.

### fn apply_change

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn apply_change`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Applies one relayed change under the exclusive lock.
**Deliberately does not `record_change`**: the journal holds only changes that
*originated* on this device, so `get_changes_since` never re-sends something
merely received — the relay is a broadcast (it forwards each device's change to
every other peer), so re-journaling would echo every change back out each
cycle. Do not add `record_change` here without also switching the relay away
from broadcast. Per variant, everything is version-vector gated
(`incoming_wins`/`assoc_incoming_wins`/`resource_incoming_wins` — a losing or
equal-vector change is a silent idempotent no-op):

- **Note create/update** — winner ⇒ an atomic transaction refreshing the
  `note_links` projection and `INSERT OR REPLACE`-ing the row (so a crash
  mid-apply cannot desync the index; still idempotent on retry).
- **Note/notebook/tag/resource delete** — winner ⇒ stamp the tombstone; if the
  entity is **unknown locally** (out-of-order delivery), insert a minimal
  tombstone row so a later stale create/update loses in `resolve` instead of
  resurrecting it (issue #71).
- **Notebook/tag create/update** — winner ⇒ `INSERT OR REPLACE`.
- **NoteTagAdd/Remove** — winner ⇒ `upsert_assoc` present/tombstone.
- **ResourceCreate** — winner ⇒ `INSERT OR REPLACE` storing the carried
  payload (empty when the change was blob-stripped).

### fn get_last_sync_time

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn get_last_sync_time`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `sync_state['last_sync']`, epoch when never synced.

### fn update_sync_time

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn update_sync_time`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `INSERT OR REPLACE` of the watermark.

### fn send_changes

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn send_changes`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Empty batch → Ok. No `server_url` → Ok (deliberately
local-only; nowhere to send is not a failure). Otherwise one
`{"type":"changes","batch_id","device_id","changes"}` frame, retried up to 4
attempts with 2/4/8 s backoff; a failed send clears the slot for `ensure_ws`.
If the connection cannot be (re-)established, **fail fast with an error** —
returning Ok would advance the watermark past changes the relay never saw,
silently dropping them forever; the same batch is re-collected next cycle.
`batch_id` + `device_id` drive the server's `(user, batch, index)` dedup.

### fn receive_changes

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn receive_changes`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Ensure/reconnect (no connection → empty vec), then drain
buffered frames with a 100 ms silence timeout (bounded-time — later messages
arrive next cycle) and a hard cap of 1 000 messages per call (a misbehaving
server cannot exhaust memory; the remainder is delivered next cycle). Malformed
frames are logged and skipped (one bad frame must not block well-formed batches
or fail the cycle); `{"type":"changes"}` frames contribute their batch; a Close
frame or stream error clears the slot for reconnect.

### fn prune_change_journal

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn prune_change_journal`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `DELETE FROM entity_changes WHERE changed_at < cutoff`,
returning the row count.

### fn get_device_id

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn get_device_id`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncBackend for DbBackend > fn get_device_id
    async fn get_device_id(&self) -> Result<String, StorageError> {
        Ok(self.device_id.clone())
    }
```

**What it does** — The cached installation id.

---

## ServerVersion

**Identification** — private deserialise struct; marker `// md:ServerVersion`.

**Code** — complete and verbatim:

```rust
// md:ServerVersion
#[derive(Debug, serde::Deserialize)]
struct ServerVersion {
    timestamp: DateTime<Utc>,
    device_id: String,
    entity: Option<serde_json::Value>,
}
```

**What it does** — One version as served by keeplin-srv's history endpoints
(`GET /api/{notes,notebooks}/:id/history`): the edit's instant, the authoring
sync device, and the snapshot exactly as pushed (`None` = tombstone). Encrypted
fields are still ciphertext here; `EncryptedBackend` decrypts on the way up,
same as for the local journal.

**Used by** — `server_entity_history`.

---

## CapabilityCache

**Identification** — private enum; marker `// md:CapabilityCache`.

**Code** — complete and verbatim:

```rust
// md:CapabilityCache
enum CapabilityCache {
    Unknown,
    Unavailable,
    Known(Vec<String>),
}
```

**What it does** — Cached `GET /version` outcome (keeplin#114): `Unknown` (not
fetched — a lazy probe may retry), `Unavailable` (no `/version`; capabilities
indeterminate), `Known(Vec<String>)`.

**Used by** — the `server_capabilities` field, `server_has_capability`.

---

## fn http_base_of

**Identification** — `fn http_base_of(server_url: &str) -> Option<String>`;
marker `// md:fn http_base_of`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Derives the HTTP base from the WebSocket URL (`ws`→`http`,
`wss`→`https`, the `/api/sync` relay path stripped); `None` for empty or
non-WebSocket URLs (offline). A free function so `DbBackend::new` can run the
handshake before `self` exists.

**Used by** — `new`, `server_http_base`.

---

## impl DbBackend (server history)

**Identification** — the second inherent impl; marker
`// md:impl DbBackend (server history)`. Four methods.

**Code** — container: members documented as sub-blocks below: fn server_http_base, fn server_has_capability, fn server_entity_history, fn entity_history.

### fn server_http_base

**Identification** — marker
`// md:impl DbBackend (server history) > fn server_http_base`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (server history) > fn server_http_base
    fn server_http_base(&self) -> Option<String> {
        http_base_of(&self.server_url)
    }
```

**What it does** — `http_base_of(&self.server_url)`.

### fn server_has_capability

**Identification** — marker
`// md:impl DbBackend (server history) > fn server_has_capability`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Whether the server advertises `capability` at
`GET /version`, fetched once and cached: `Some(true/false)` when the server has
`/version`; `None` when it doesn't (older server) — the caller falls back to
feature-specific probing.

**Used by** — `server_entity_history`.

### fn server_entity_history

**Identification** — marker
`// md:impl DbBackend (server history) > fn server_entity_history`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Fetches an entity's history from the server (the durable
**cross-device** record). `None` (→ local fallback) when: the 404 latch is set;
capability negotiation says the server lacks `history` (which also sets the
latch); no server configured; a transient network error (does **not** latch);
any HTTP error; malformed JSON. A definitive 404 latches
`history_unsupported` so future reads skip the round-trip (issue #113).
Unparseable snapshots are skipped rather than mislabelled as deletes.

### fn entity_history

**Identification** — marker
`// md:impl DbBackend (server history) > fn entity_history`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Past versions newest-first: server journal first (a fresh
device sees every device's history, cross-device rollback works), local
`entity_changes` fallback (this device's own changes only). `limit = 0` →
`DEFAULT_HISTORY_LIMIT`. Local mapping: create/update → snapshot (unparseable
→ skip), delete → `entity: None`.

**Used by** — the `HistoryRepository` impl.

---

## impl HistoryRepository for DbBackend

**Identification** — marker `// md:impl HistoryRepository for DbBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn note_history, fn notebook_history.

**What it does** — thin typed wrappers.

### fn note_history

**Identification** — marker
`// md:impl HistoryRepository for DbBackend > fn note_history`.

**Code** — complete and verbatim:

```rust
    // md:impl HistoryRepository for DbBackend > fn note_history
    async fn note_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Note>>, StorageError> {
        self.entity_history::<Note>("note", id, limit).await
    }
```

**What it does** — `entity_history::<Note>("note", …)`.

### fn notebook_history

**Identification** — marker
`// md:impl HistoryRepository for DbBackend > fn notebook_history`.

**Code** — complete and verbatim:

```rust
    // md:impl HistoryRepository for DbBackend > fn notebook_history
    async fn notebook_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Notebook>>, StorageError> {
        self.entity_history::<Notebook>("notebook", id, limit).await
    }
```

**What it does** — `entity_history::<Notebook>("notebook", …)`.

---

## mod migration_tests

**Identification** — `#[cfg(test)] mod migration_tests`; marker
`// md:mod migration_tests`. Two helpers + four tests.

**Code** — container: members documented as sub-blocks below: fn raw_conn, fn user_version, fn note_history_reads_this_devices_versions_newest_first, fn fresh_database_is_stamped_current_and_reopen_is_a_noop, fn tag_system_flag_round_trips, fn migrates_a_pre_framework_database_without_losing_data, fn refuses_to_open_a_newer_schema.

**What it does** — Pins the migration framework and the journal-derived
history.

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
        assert!(created.system, "create_tag keeps the system flag it was given");
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

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `DbBackend` — defined here (EXTRACTED; the database backend root)
- the repository-trait implementations (implements×6) and the row/versioning helpers (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs` — error types (EXTRACTED: references×69)
- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: calls×2, references×32)
- `keeplin-core/src/links.rs` — `Bookmark`/`NoteLink` column (de)serialisation (EXTRACTED: references×4)
- `keeplin-core/src/storage/backend.rs` — the trait family (EXTRACTED: implements×6, references×8)
- `keeplin-core/src/storage/note_log.rs` — `resolve`/`VersionVector`/`Winner` (EXTRACTED)
- `keeplin-core/src/compat.rs` — the `/version` handshake (INFERRED: fully-qualified paths)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/main.rs` — `build_storage` (INFERRED)
- `keeplin-core/tests/db_backend.rs`, `tests/ws_sync.rs`, `tests/sync.rs`, `tests/migrate.rs` — integration tests (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `WsStream` | `// md:WsStream` |
| 3 | `DbBackend` | `// md:DbBackend` |
| 4 | `impl DbBackend` (container) | `// md:impl DbBackend` |
| 5 | `fn new` | `// md:impl DbBackend > fn new` |
| 6 | `fn run_migrations` | `// md:impl DbBackend > fn run_migrations` |
| 7 | `fn schema_version` | `// md:impl DbBackend > fn schema_version` |
| 8 | `fn apply_migration` | `// md:impl DbBackend > fn apply_migration` |
| 9 | `fn migrate_v1_baseline` | `// md:impl DbBackend > fn migrate_v1_baseline` |
| 10 | `fn add_column_if_missing` | `// md:impl DbBackend > fn add_column_if_missing` |
| 11 | `fn get_or_create_device_id` | `// md:impl DbBackend > fn get_or_create_device_id` |
| 12 | `fn record_change` | `// md:impl DbBackend > fn record_change` |
| 13 | `fn refresh_note_links` | `// md:impl DbBackend > fn refresh_note_links` |
| 14 | `fn connect_ws` | `// md:impl DbBackend > fn connect_ws` |
| 15 | `fn row_to_note` | `// md:impl DbBackend > fn row_to_note` |
| 16 | `fn parse_uuid` | `// md:impl DbBackend > fn parse_uuid` |
| 17 | `fn parse_required_dt` | `// md:impl DbBackend > fn parse_required_dt` |
| 18 | `fn parse_optional_dt` | `// md:impl DbBackend > fn parse_optional_dt` |
| 19 | `fn row_to_notebook` | `// md:impl DbBackend > fn row_to_notebook` |
| 20 | `fn row_to_tag` | `// md:impl DbBackend > fn row_to_tag` |
| 21 | `fn row_to_resource` | `// md:impl DbBackend > fn row_to_resource` |
| 22 | `fn row_to_change` | `// md:impl DbBackend > fn row_to_change` |
| 23 | `fn begin` | `// md:impl DbBackend > fn begin` |
| 24 | `fn commit` | `// md:impl DbBackend > fn commit` |
| 25 | `fn rollback` | `// md:impl DbBackend > fn rollback` |
| 26 | `fn ensure_ws` | `// md:impl DbBackend > fn ensure_ws` |
| 27 | `fn migrate_v2_ordering` | `// md:impl DbBackend > fn migrate_v2_ordering` |
| 28 | `fn migrate_v3_tag_system` | `// md:impl DbBackend > fn migrate_v3_tag_system` |
| 29 | `fn current_meta` | `// md:impl DbBackend > fn current_meta` |
| 30 | `fn incoming_wins` | `// md:impl DbBackend > fn incoming_wins` |
| 31 | `fn next_local_vv` | `// md:impl DbBackend > fn next_local_vv` |
| 32 | `fn row_is_live` | `// md:impl DbBackend > fn row_is_live` |
| 33 | `fn assoc_meta` | `// md:impl DbBackend > fn assoc_meta` |
| 34 | `fn next_assoc_vv` | `// md:impl DbBackend > fn next_assoc_vv` |
| 35 | `fn assoc_incoming_wins` | `// md:impl DbBackend > fn assoc_incoming_wins` |
| 36 | `fn upsert_assoc` | `// md:impl DbBackend > fn upsert_assoc` |
| 37 | `fn resource_meta` | `// md:impl DbBackend > fn resource_meta` |
| 38 | `fn next_resource_vv` | `// md:impl DbBackend > fn next_resource_vv` |
| 39 | `fn resource_incoming_wins` | `// md:impl DbBackend > fn resource_incoming_wins` |
| 40 | `fn parse_cursor` | `// md:fn parse_cursor` |
| 41 | `fn build_page` | `// md:fn build_page` |
| 42 | `fn bookmarks_to_json` | `// md:fn bookmarks_to_json` |
| 43 | `fn links_to_json` | `// md:fn links_to_json` |
| 44 | `fn json_to_bookmarks` | `// md:fn json_to_bookmarks` |
| 45 | `fn json_to_links` | `// md:fn json_to_links` |
| 46 | `fn vv_to_json` | `// md:fn vv_to_json` |
| 47 | `fn json_to_vv` | `// md:fn json_to_vv` |
| 48 | `fn tombstone_data` | `// md:fn tombstone_data` |
| 49 | `fn assoc_data` | `// md:fn assoc_data` |
| 50 | `fn assoc_from_data` | `// md:fn assoc_from_data` |
| 51 | `fn tombstone_from_data` | `// md:fn tombstone_from_data` |
| 52 | `impl NoteRepository for DbBackend` (container) | `// md:impl NoteRepository for DbBackend` |
| 53 | `fn create_note` | `// md:impl NoteRepository for DbBackend > fn create_note` |
| 54 | `fn read_note` | `// md:impl NoteRepository for DbBackend > fn read_note` |
| 55 | `fn update_note` | `// md:impl NoteRepository for DbBackend > fn update_note` |
| 56 | `fn delete_note` | `// md:impl NoteRepository for DbBackend > fn delete_note` |
| 57 | `fn list_notes` | `// md:impl NoteRepository for DbBackend > fn list_notes` |
| 58 | `fn note_backlinks` | `// md:impl NoteRepository for DbBackend > fn note_backlinks` |
| 59 | `fn list_notes_in_notebook` | `// md:impl NoteRepository for DbBackend > fn list_notes_in_notebook` |
| 60 | `fn list_starred_notes` | `// md:impl NoteRepository for DbBackend > fn list_starred_notes` |
| 61 | `fn notebook_sort_profile` | `// md:impl NoteRepository for DbBackend > fn notebook_sort_profile` |
| 62 | `impl NotebookRepository for DbBackend` (container) | `// md:impl NotebookRepository for DbBackend` |
| 63 | `fn create_notebook` | `// md:impl NotebookRepository for DbBackend > fn create_notebook` |
| 64 | `fn read_notebook` | `// md:impl NotebookRepository for DbBackend > fn read_notebook` |
| 65 | `fn update_notebook` | `// md:impl NotebookRepository for DbBackend > fn update_notebook` |
| 66 | `fn delete_notebook` | `// md:impl NotebookRepository for DbBackend > fn delete_notebook` |
| 67 | `fn list_notebooks` | `// md:impl NotebookRepository for DbBackend > fn list_notebooks` |
| 68 | `impl TagRepository for DbBackend` (container) | `// md:impl TagRepository for DbBackend` |
| 69 | `fn create_tag` | `// md:impl TagRepository for DbBackend > fn create_tag` |
| 70 | `fn read_tag` | `// md:impl TagRepository for DbBackend > fn read_tag` |
| 71 | `fn update_tag` | `// md:impl TagRepository for DbBackend > fn update_tag` |
| 72 | `fn delete_tag` | `// md:impl TagRepository for DbBackend > fn delete_tag` |
| 73 | `fn list_tags` | `// md:impl TagRepository for DbBackend > fn list_tags` |
| 74 | `fn add_note_tag` | `// md:impl TagRepository for DbBackend > fn add_note_tag` |
| 75 | `fn remove_note_tag` | `// md:impl TagRepository for DbBackend > fn remove_note_tag` |
| 76 | `fn list_note_tags` | `// md:impl TagRepository for DbBackend > fn list_note_tags` |
| 77 | `impl ResourceRepository for DbBackend` (container) | `// md:impl ResourceRepository for DbBackend` |
| 78 | `fn create_resource` | `// md:impl ResourceRepository for DbBackend > fn create_resource` |
| 79 | `fn read_resource` | `// md:impl ResourceRepository for DbBackend > fn read_resource` |
| 80 | `fn delete_resource` | `// md:impl ResourceRepository for DbBackend > fn delete_resource` |
| 81 | `fn list_resources` | `// md:impl ResourceRepository for DbBackend > fn list_resources` |
| 82 | `fn purge_deleted_resources` | `// md:impl ResourceRepository for DbBackend > fn purge_deleted_resources` |
| 83 | `impl SyncBackend for DbBackend` (container) | `// md:impl SyncBackend for DbBackend` |
| 84 | `fn get_changes_since` | `// md:impl SyncBackend for DbBackend > fn get_changes_since` |
| 85 | `fn apply_change` | `// md:impl SyncBackend for DbBackend > fn apply_change` |
| 86 | `fn get_last_sync_time` | `// md:impl SyncBackend for DbBackend > fn get_last_sync_time` |
| 87 | `fn update_sync_time` | `// md:impl SyncBackend for DbBackend > fn update_sync_time` |
| 88 | `fn send_changes` | `// md:impl SyncBackend for DbBackend > fn send_changes` |
| 89 | `fn receive_changes` | `// md:impl SyncBackend for DbBackend > fn receive_changes` |
| 90 | `fn prune_change_journal` | `// md:impl SyncBackend for DbBackend > fn prune_change_journal` |
| 91 | `fn get_device_id` | `// md:impl SyncBackend for DbBackend > fn get_device_id` |
| 92 | `ServerVersion` | `// md:ServerVersion` |
| 93 | `CapabilityCache` | `// md:CapabilityCache` |
| 94 | `fn http_base_of` | `// md:fn http_base_of` |
| 95 | `impl DbBackend (server history)` (container) | `// md:impl DbBackend (server history)` |
| 96 | `fn server_http_base` | `// md:impl DbBackend (server history) > fn server_http_base` |
| 97 | `fn server_has_capability` | `// md:impl DbBackend (server history) > fn server_has_capability` |
| 98 | `fn server_entity_history` | `// md:impl DbBackend (server history) > fn server_entity_history` |
| 99 | `fn entity_history` | `// md:impl DbBackend (server history) > fn entity_history` |
| 100 | `impl HistoryRepository for DbBackend` (container) | `// md:impl HistoryRepository for DbBackend` |
| 101 | `fn note_history` | `// md:impl HistoryRepository for DbBackend > fn note_history` |
| 102 | `fn notebook_history` | `// md:impl HistoryRepository for DbBackend > fn notebook_history` |
| 103 | `mod migration_tests` (container) | `// md:mod migration_tests` |
| 104 | `fn raw_conn` | `// md:mod migration_tests > fn raw_conn` |
| 105 | `fn user_version` | `// md:mod migration_tests > fn user_version` |
| 106 | `fn note_history_reads_this_devices_versions_newest_first` | `// md:mod migration_tests > fn note_history_reads_this_devices_versions_newest_first` |
| 107 | `fn fresh_database_is_stamped_current_and_reopen_is_a_noop` | `// md:mod migration_tests > fn fresh_database_is_stamped_current_and_reopen_is_a_noop` |
| 108 | `fn tag_system_flag_round_trips` | `// md:mod migration_tests > fn tag_system_flag_round_trips` |
| 109 | `fn migrates_a_pre_framework_database_without_losing_data` | `// md:mod migration_tests > fn migrates_a_pre_framework_database_without_losing_data` |
| 110 | `fn refuses_to_open_a_newer_schema` | `// md:mod migration_tests > fn refuses_to_open_a_newer_schema` |