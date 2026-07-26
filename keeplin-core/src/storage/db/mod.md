# `storage/db/mod.rs` — DbBackend — type, lifecycle and transaction handles

Self-contained companion for `keeplin-core/src/storage/db/mod.rs`. It documents **every
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

**Identification** — file-level block: the module's imports, the submodule declarations that make up this directory module, and the two re-exports (`effective_page_size`, `NotebookSortProfile`) that relocated sibling code reaches through `super::`. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Root of the `storage::db` directory module. It owns the `DbBackend` struct itself — the LibSQL connection, the server URL and token, the shared WebSocket handle, the device id, the HTTP client and the read/write lock — plus construction (`new`), the transaction handles (`begin`/`commit`/`rollback`) and the WebSocket lifecycle (`connect_ws`/`ensure_ws`). Everything else about the backend lives in a sibling module declared here. Private items in this file are visible to every sibling, which is why the struct's fields need no widening; items defined in a sibling and used across the directory carry `pub(super)`.

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

## re-exports

**Identification** — file-level block: the two parent-module items that relocated
sibling code reaches through `super::`; marker `// md:re-exports`.

**Code** — complete and verbatim:

```rust
// md:re-exports
pub(super) use super::{effective_page_size, NotebookSortProfile};
use server::{http_base_of, CapabilityCache};
```

**What it does** — Before the split, `storage/db.rs` sat directly inside `storage`, so
its bodies said `super::effective_page_size` and `super::NotebookSortProfile`. In the
directory module `super::` now resolves to `storage::db`, one level deeper. Re-exporting
both names here keeps every relocated body byte-identical instead of rewriting call
sites — the split stays a pure relocation.

**Dependencies** —
- `crate::storage::effective_page_size` — clamps a caller-supplied page size; expects: the
  same clamping the pre-split bodies relied on, since a changed bound would silently
  alter every paginated listing in this directory module.
- `crate::storage::NotebookSortProfile` — the sort profile the note listings resolve;
  expects: `from_effective_keys` to keep accepting the key list `notes.rs` builds.

**Used by** — `notes.rs` and `notebooks.rs` (`super::effective_page_size`),
`notes.rs` (`super::NotebookSortProfile`).

**Repeated context** — `pub(super)` here means visible to `storage`; sibling modules of
this directory reach it as `super::<name>` because a module's items are visible to its
descendants.

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
Construction, device identity, change journaling, the link projection and the transaction and WebSocket handles.

**Code** — container: members documented as sub-blocks below: fn new, fn get_or_create_device_id, fn record_change, fn refresh_note_links, fn connect_ws, fn begin, fn commit, fn rollback, fn ensure_ws.

---

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

    const SCHEMA_VERSION: u32 = 5;
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

---

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

---

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

---

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

---

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

---

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

---

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

---

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

---

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

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `DbBackend` — defined here (INFERRED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/backend.rs` — the repository traits and shared types (INFERRED)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/storage/mod.rs` — declares `pub mod db` (INFERRED)

**Invariants** (the rules this file must keep true — restated here even if stated
elsewhere)

- The split is a relocation: `storage::db::DbBackend` stays the public path, so no caller outside this directory module changes.

---

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | re-exports | `// md:re-exports` |
| 3 | WsStream | `// md:WsStream` |
| 4 | DbBackend | `// md:DbBackend` |
| 5 | impl DbBackend | `// md:impl DbBackend` |
| 6 | fn new | `// md:impl DbBackend > fn new` |
| 7 | fn get_or_create_device_id | `// md:impl DbBackend > fn get_or_create_device_id` |
| 8 | fn record_change | `// md:impl DbBackend > fn record_change` |
| 9 | fn refresh_note_links | `// md:impl DbBackend > fn refresh_note_links` |
| 10 | fn connect_ws | `// md:impl DbBackend > fn connect_ws` |
| 11 | fn begin | `// md:impl DbBackend > fn begin` |
| 12 | fn commit | `// md:impl DbBackend > fn commit` |
| 13 | fn rollback | `// md:impl DbBackend > fn rollback` |
| 14 | fn ensure_ws | `// md:impl DbBackend > fn ensure_ws` |
