# `collab/mod.rs` — client of the keeplin-srv collaborative channel

Self-contained companion for `keeplin-core/src/collab/mod.rs`. It documents **every
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

**Identification** — file-level block: the child-module declarations and imports.
Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
pub mod protocol;
```

**What it does** — The client side of keeplin-srv's collaborative channel.
Architecture: frontends talk to the daemon (gRPC/REST/feed); the daemon — through
`CollabBackend` — talks to keeplin-srv. The server stores every note (lines +
versioned order + metadata) and is the durable source of truth; this client keeps
the user's notes (own and shared) in the local database and mirrors line state in
memory, rebuilt from each `Welcome` snapshot on (re)connect.

- Local note writes are diffed into `protocol::LineOp`s (signed with this
  **device**'s id, the vv actor) and pushed over the WebSocket; title/metadata
  changes go over REST.
- Remote ops and snapshots are applied through the daemon's full decorator stack
  (so links are re-derived and the live feed fires), with an in-flight
  **suppression set** preventing those writes from echoing back into ops.
- Note discovery (own + shared) is a paged REST `GET /api/notes` at connect,
  following the server's `X-Next-Cursor` header.
- Note `Change`s are **filtered out of the relay sync path**: the collab channel
  owns notes; notebooks/tags/resources keep syncing via the relay.

**Dependencies** — `tokio`/`tokio_tungstenite`/`futures_util` (WebSocket),
`reqwest` (REST), `serde_json`, `base64`, `anyhow` (connection task); the sibling
`protocol`/`state` modules; the crate's error/model/storage-trait types;
`crate::compat` for the handshake; `crate::format` for the shared hard limits —
`check_body` gates every local write and `is_limit_code` classifies a server
rejection, and both expect the constants to be the ones keeplin-srv also enforces.

**Used by** — `keeplin-daemon/src/main.rs` (constructs and starts it in server
mode), `keeplin-daemon/src/rest.rs` (heavily — it is the note surface in server
mode plus the presence/permission proxy), `keeplin-core/tests/collab_client.rs`.

**Repeated context** — Core invariants: (1) the relay must never carry note
`Change`s or a note would travel both paths and double-apply; (2) server-driven
writes go through `top` with the id suppressed — no echo; (3) a note pending its
first `Welcome` is reconciled against the snapshot, never pushed eagerly — a late
empty `Welcome` must not clobber local content; (4) resource binaries never ride
the relay journal; (5) `start` runs the `compat` handshake first — an
incompatible server spawns no connection task; (6) a body that breaks a format
limit is rejected **before** the local write, and an op the server rejects over a
limit triggers a resync rather than a silent divergence — the client never keeps
content it knows the server refused.

---

## CollabConfig

**Identification** — struct deriving `Debug, Clone`; marker
`// md:CollabConfig`.

**Code** — complete and verbatim:

```rust
// md:CollabConfig
#[derive(Debug, Clone)]
pub struct CollabConfig {
    pub api_url: String,
    pub ws_url: String,
    pub token: String,
}
```

**What it does** — Connection settings: `api_url` (HTTP base, e.g.
`http://host:3000`), `ws_url` (e.g. `ws://host:3000/api/ws`), `token` (device
token from keeplin-srv `POST /api/login` — one per device).

**Dependencies** — none.

**Used by** — `CollabBackend::new`; built by `keeplin-daemon/src/main.rs` from
config.

**Repeated context** — none.

---

## fn device_id_from_token

**Identification** — `pub fn device_id_from_token(token: &str) -> Option<String>`;
marker `// md:fn device_id_from_token`.

**Code** — complete and verbatim:

```rust
// md:fn device_id_from_token
pub fn device_id_from_token(token: &str) -> Option<String> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    Some(claims.get("device_id")?.as_str()?.to_string())
}
```

**What it does** — Extracts the `device_id` claim from a JWT **without
verifying** it: split on `.`, base64url-decode the payload, read
`claims.device_id`. The server verifies tokens; the client only needs to know
its own identity to sign ops (the vv actor).

**Dependencies** — `base64`, `serde_json`.

**Used by** — `CollabBackend::new` (fails construction with `InvalidState` when
the claim is missing).

**Repeated context** — device-as-vv-actor: every op's `last_writer` and
advancing vv component is this device id, and the server rejects mismatches
(`bad_writer`).

---

## Shared

**Identification** — private `struct Shared`; marker `// md:Shared`.

**Code** — complete and verbatim:

```rust
// md:Shared
struct Shared {
    cfg: CollabConfig,
    device_id: String,
    http: reqwest::Client,
    notes: Mutex<HashMap<Uuid, NoteLines>>,
    suppress: Mutex<HashSet<Uuid>>,
    presence: Mutex<HashMap<Uuid, Vec<protocol::PresenceInfo>>>,
    out: mpsc::UnboundedSender<CollabClientMsg>,
    pending_push: Mutex<HashSet<Uuid>>,
    top: OnceLock<Arc<dyn StorageBackend>>,
}
```

**What it does** — The state shared between the decorator, the handle, and the
connection task: `cfg`, `device_id`, a `reqwest::Client`,
`notes: Mutex<HashMap<Uuid, NoteLines>>` (per-note in-memory mirror),
`suppress: Mutex<HashSet<Uuid>>` (ids currently being written *from* the server
so the decorator doesn't diff them back — no echo),
`presence: Mutex<HashMap<Uuid, Vec<PresenceInfo>>>` (latest per-note presence,
served to frontends via the daemon's REST API), `out` (unbounded outbound queue
drained by the connection task), `pending_push: Mutex<HashSet<Uuid>>` (notes
with local content not yet on the server — freshly created, or edited before the
first `Welcome`; their body is reconciled against the join snapshot instead of
pushed eagerly, so a late empty `Welcome` cannot clobber the local content),
`top: OnceLock<Arc<dyn StorageBackend>>` (the daemon's outermost decorator, set
once by `start`, used to apply remote state so linking/eventing run on those
writes too).

**Dependencies** — `NoteLines`, `CollabClientMsg`, tokio sync primitives.

**Used by** — everything in this file.

**Repeated context** — none.

---

## impl Shared

**Identification** — inherent impl; marker `// md:impl Shared`. Five methods.

**Code** — container: members documented as sub-blocks below: fn auth, fn suppressed, fn apply_from_server, fn upload_blob, fn download_blob.

### fn auth

**Identification** — `fn auth(&self, req) -> reqwest::RequestBuilder`; marker
`// md:impl Shared > fn auth`.

**Code** — complete and verbatim:

```rust
    // md:impl Shared > fn auth
    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.bearer_auth(&self.cfg.token)
    }
```

**What it does** — Adds `Authorization: Bearer <token>` to a request builder.

**Dependencies** — `reqwest`.

**Used by** — every REST call in the file.

**Repeated context** — the token never travels in URLs (query strings end up in
proxy and access logs).

### fn suppressed

**Identification** — `async fn suppressed(&self, id: Uuid) -> bool`; marker
`// md:impl Shared > fn suppressed`.

**Code** — complete and verbatim:

```rust
    // md:impl Shared > fn suppressed
    async fn suppressed(&self, id: Uuid) -> bool {
        self.suppress.lock().await.contains(&id)
    }
```

**What it does** — Whether `id` is currently in the suppression set.

**Dependencies** — `suppress`.

**Used by** — the `NoteRepository` write methods.

**Repeated context** — none.

### fn apply_from_server

**Identification** —
`async fn apply_from_server(&self, note: Note, create: bool) -> Result<(), StorageError>`;
marker `// md:impl Shared > fn apply_from_server`.

**Code** — complete and verbatim:

```rust
    // md:impl Shared > fn apply_from_server
    async fn apply_from_server(&self, note: Note, create: bool) -> Result<(), StorageError> {
        let Some(top) = self.top.get() else {
            return Ok(());
        };
        self.suppress.lock().await.insert(note.id);
        let result = if create {
            top.create_note(note.clone()).await.map(|_| ())
        } else {
            top.update_note(note.clone()).await.map(|_| ())
        };
        self.suppress.lock().await.remove(&note.id);
        result
    }
```

**What it does** — Writes `note` through the top of the stack with echo
suppression: insert the id into `suppress`, `create_note` or `update_note` on
`top`, remove the id. A no-op (Ok) before `start` set `top`.

**Dependencies** — `top`, `suppress`.

**Used by** — `ensure_local`, `write_body`.

**Repeated context** — going through `top` (not `inner`) is what makes remote
writes re-derive links and fire the daemon's live feed exactly like local ones.

### fn upload_blob

**Identification** — `async fn upload_blob(&self, id: Uuid, data: Vec<u8>)`;
marker `// md:impl Shared > fn upload_blob`.

**Code** — complete and verbatim:

```rust
    // md:impl Shared > fn upload_blob
    async fn upload_blob(&self, id: Uuid, data: Vec<u8>) {
        let url = format!("{}/api/resources/{}/data", self.cfg.api_url, id);
        for attempt in 0..5u32 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt))).await;
            }
            match self
                .auth(self.http.put(&url))
                .body(data.clone())
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => return,
                Ok(resp) => {
                    tracing::debug!(
                        resource = %id,
                        status = %resp.status(),
                        attempt,
                        "collab: blob upload not accepted yet"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, resource = %id, "collab: resource blob upload failed");
                    return;
                }
            }
        }
        tracing::warn!(resource = %id, "collab: resource blob upload not accepted after retries");
    }
```

**What it does** — Uploads a resource's binary to keeplin-srv out-of-band
(`PUT /api/resources/{id}/data`) so bytes never ride the relay journal. The
server accepts a blob only once the resource's metadata has been
**materialised** from the relay journal — asynchronous after the eager push in
`create_resource` — so a non-success answer (e.g. a 404 while metadata is in
flight) is retried up to 5 times with linear backoff (200 ms × attempt). A
transport failure aborts immediately (the server is unreachable; an immediate
retry cannot help). Best effort beyond that: persistent failure is logged,
metadata still syncs, and the blob can be re-uploaded by a later
create/replace.

**Dependencies** — `auth`, `reqwest`, `tokio::time`.

**Used by** — `create_resource`.

**Repeated context** — none.

### fn download_blob

**Identification** — `async fn download_blob(&self, id: Uuid) -> Option<Vec<u8>>`;
marker `// md:impl Shared > fn download_blob`.

**Code** — complete and verbatim:

```rust
    // md:impl Shared > fn download_blob
    async fn download_blob(&self, id: Uuid) -> Option<Vec<u8>> {
        let url = format!("{}/api/resources/{}/data", self.cfg.api_url, id);
        let resp = self.auth(self.http.get(url)).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.bytes().await.ok().map(|b| b.to_vec())
    }
```

**What it does** — Fetches a resource's binary from keeplin-srv (the source of
truth for blobs); `None` on any failure so the caller falls back to the local
cache.

**Dependencies** — `auth`, `reqwest`.

**Used by** — `read_resource`.

**Repeated context** — none.

---

## CollabHandle

**Identification** — `#[derive(Clone)] pub struct CollabHandle`; marker
`// md:CollabHandle`.

**Code** — complete and verbatim:

```rust
// md:CollabHandle
#[derive(Clone)]
pub struct CollabHandle {
    shared: Arc<Shared>,
}
```

**What it does** — An ungeneric, cloneable view of the collaborative session
for the daemon's HTTP/gRPC surfaces: read presence and publish this device's
cursor without knowing the storage type behind `CollabBackend`.

**Dependencies** — `Shared`.

**Used by** — `keeplin-daemon/src/rest.rs` (presence endpoint, cursor endpoint,
permission proxy).

**Repeated context** — none.

---

## impl CollabHandle

**Identification** — inherent impl; marker `// md:impl CollabHandle`. Three
methods.

**Code** — container: members documented as sub-blocks below: fn presence, fn send_cursor, fn proxy_request.

### fn presence

**Identification** — `pub async fn presence(&self, note_id: Uuid) -> Vec<PresenceInfo>`;
marker `// md:impl CollabHandle > fn presence`.

**Code** — complete and verbatim:

```rust
    // md:impl CollabHandle > fn presence
    pub async fn presence(&self, note_id: Uuid) -> Vec<protocol::PresenceInfo> {
        self.shared
            .presence
            .lock()
            .await
            .get(&note_id)
            .cloned()
            .unwrap_or_default()
    }
```

**What it does** — The latest presence list the server broadcast for `note_id`
(empty when the note has no live session or is unknown).

**Dependencies** — the `presence` map.

**Used by** — `rest.rs::note_presence`.

**Repeated context** — none.

### fn send_cursor

**Identification** — `pub fn send_cursor(&self, note_id: Uuid, cursor: Cursor)`;
marker `// md:impl CollabHandle > fn send_cursor`.

**Code** — complete and verbatim:

```rust
    // md:impl CollabHandle > fn send_cursor
    pub fn send_cursor(&self, note_id: Uuid, cursor: protocol::Cursor) {
        let _ = self
            .shared
            .out
            .send(CollabClientMsg::Cursor { note_id, cursor });
    }
```

**What it does** — Queues this device's caret position; the server fans the
updated presence out to every participant. Fire-and-forget (send errors
ignored — the connection task may be between reconnects).

**Dependencies** — the `out` queue.

**Used by** — `rest.rs`'s cursor endpoint.

**Repeated context** — none.

### fn proxy_request

**Identification** —
`pub async fn proxy_request(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> Result<(u16, serde_json::Value), StorageError>`;
marker `// md:impl CollabHandle > fn proxy_request`.

**Code** — complete and verbatim:

```rust
    // md:impl CollabHandle > fn proxy_request
    pub async fn proxy_request(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<(u16, serde_json::Value), StorageError> {
        let verb = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|e| StorageError::InvalidInput(format!("bad method {method}: {e}")))?;
        let url = format!("{}{}", self.shared.cfg.api_url, path);
        let mut req = self.shared.auth(self.shared.http.request(verb, url));
        if let Some(b) = &body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| {
            StorageError::Database(format!("permission proxy to server failed: {e}"))
        })?;
        let status = resp.status().as_u16();
        let json = resp
            .json::<serde_json::Value>()
            .await
            .unwrap_or(serde_json::Value::Null);
        Ok((status, json))
    }
```

**What it does** — Forwards a **permission-management** request (share,
transfer, list, revoke) to keeplin-srv — the authority for permissions in
server mode — returning the server's status code and JSON body. The daemon's
REST surface proxies its own permission endpoints through this, so a frontend
manages shares without the client re-implementing (or locally enforcing) the
model. A bad verb is `InvalidInput`; a transport failure is
`StorageError::Database`; a non-JSON body becomes `Null`.

**Dependencies** — `auth`, `reqwest`.

**Used by** — `rest.rs`'s share/permission endpoints.

**Repeated context** — permissions are never enforced client-side; the
capability bitset model (READ/WRITE/SHARE_*/MANAGE with the destructive
notebook→note cascade) lives entirely in keeplin-srv.

---

## CollabBackend

**Identification** — `pub struct CollabBackend<B>`; marker
`// md:CollabBackend`.

**Code** — complete and verbatim:

```rust
// md:CollabBackend
pub struct CollabBackend<B> {
    inner: Arc<B>,
    shared: Arc<Shared>,
    out_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<CollabClientMsg>>>>,
}
```

**What it does** — The storage decorator that turns local note writes into
collaborative ops and REST calls. Sits **below** `LinkingBackend`/`EventBackend`
in the stack. Fields: `inner: Arc<B>`, `shared: Arc<Shared>`,
`out_rx: Arc<Mutex<Option<Receiver<…>>>>` (the outbound queue's receive half,
taken exactly once by `start`).

**Dependencies** — `Shared`, `StorageBackend`.

**Used by** — `keeplin-daemon/src/main.rs` (server-mode stack assembly).

**Repeated context** — none.

---

## impl Clone for CollabBackend

**Identification** — manual impl; marker `// md:impl Clone for CollabBackend`.

**Code** — complete and verbatim:

```rust
// md:impl Clone for CollabBackend
impl<B> Clone for CollabBackend<B> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            shared: self.shared.clone(),
            out_rx: self.out_rx.clone(),
        }
    }
}
```

**What it does** — Clones the three `Arc`s. Manual because `B` itself need not
be `Clone`.

**Dependencies** — none.

**Used by** — daemon plumbing.

**Repeated context** — none.

---

## impl CollabBackend

**Identification** — inherent impl `impl<B: StorageBackend> CollabBackend<B>`;
marker `// md:impl CollabBackend`. Five methods.

**Code** — container: members documented as sub-blocks below: fn new, fn handle, fn start, fn push_local_edit, fn patch_meta.

### fn new

**Identification** — `pub fn new(inner: B, cfg: CollabConfig) -> Result<Self, StorageError>`;
marker `// md:impl CollabBackend > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl CollabBackend > fn new
    pub fn new(inner: B, cfg: CollabConfig) -> Result<Self, StorageError> {
        let device_id = device_id_from_token(&cfg.token).ok_or_else(|| {
            StorageError::InvalidState("collab token has no device_id claim".into())
        })?;
        let (out, out_rx) = mpsc::unbounded_channel();
        Ok(Self {
            inner: Arc::new(inner),
            shared: Arc::new(Shared {
                cfg,
                device_id,
                http: reqwest::Client::new(),
                notes: Mutex::new(HashMap::new()),
                suppress: Mutex::new(HashSet::new()),
                presence: Mutex::new(HashMap::new()),
                out,
                pending_push: Mutex::new(HashSet::new()),
                top: OnceLock::new(),
            }),
            out_rx: Arc::new(Mutex::new(Some(out_rx))),
        })
    }
```

**What it does** — Extracts the device id from the token (`InvalidState` when
the claim is missing), creates the outbound channel, and assembles the shared
state with empty mirrors/sets and an unset `top`.

**Dependencies** — `device_id_from_token`, `mpsc`.

**Used by** — `main.rs::build_storage` in server mode.

**Repeated context** — none.

### fn handle

**Identification** — `pub fn handle(&self) -> CollabHandle`; marker
`// md:impl CollabBackend > fn handle`.

**Code** — complete and verbatim:

```rust
    // md:impl CollabBackend > fn handle
    pub fn handle(&self) -> CollabHandle {
        CollabHandle {
            shared: self.shared.clone(),
        }
    }
```

**What it does** — A cloneable presence/cursor/proxy view for the daemon's
surfaces.

**Dependencies** — `Shared`.

**Used by** — `main.rs` → `rest.rs` state.

**Repeated context** — none.

### fn start

**Identification** —
`pub async fn start(&self, top: Arc<dyn StorageBackend>) -> Result<(), StorageError>`;
marker `// md:impl CollabBackend > fn start`.

**Code** — complete and verbatim:

```rust
    // md:impl CollabBackend > fn start
    pub async fn start(&self, top: Arc<dyn StorageBackend>) -> Result<(), StorageError> {
        match crate::compat::negotiate(&self.shared.http, &self.shared.cfg.api_url).await {
            crate::compat::Handshake::Compatible(info) => {
                tracing::info!(
                    server = %info.name,
                    server_version = %info.version,
                    protocol = info.protocol_version,
                    capabilities = ?info.capabilities,
                    "collab: server protocol negotiated"
                );
            }
            crate::compat::Handshake::Incompatible(info) => {
                return Err(StorageError::InvalidState(
                    crate::compat::incompatible_message(&info),
                ));
            }
            crate::compat::Handshake::Unavailable => {
                tracing::warn!(
                    api_url = %self.shared.cfg.api_url,
                    "collab: server has no usable GET /version (older keeplin-srv?); \
                     continuing without protocol negotiation"
                );
            }
        }
        let _ = self.shared.top.set(top);
        let rx = self
            .out_rx
            .lock()
            .await
            .take()
            .expect("collab task started twice");
        tokio::spawn(run_connection(self.shared.clone(), rx));
        Ok(())
    }
```

**What it does** — Runs the `GET /version` handshake first (`crate::compat`):
`Compatible` → log negotiated protocol + capabilities and proceed;
`Incompatible` → **loud error** (`InvalidState` with `incompatible_message`)
and the connection task is never spawned (no sync attempted); `Unavailable` →
warn and proceed (backward compatible with older keeplin-srv). Then sets `top`
(must be the outermost backend of the stack so remote writes flow through every
decorator exactly once), takes the outbound receiver (panics if started twice),
and spawns `run_connection`.

**Dependencies** — `compat::negotiate`/`incompatible_message`,
`run_connection`, `tokio::spawn`.

**Used by** — `main.rs` after assembling the stack.

**Repeated context** — none.

### fn push_local_edit

**Identification** — `async fn push_local_edit(&self, note: &Note)`; marker
`// md:impl CollabBackend > fn push_local_edit`.

**Code** — complete and verbatim:

```rust
    // md:impl CollabBackend > fn push_local_edit
    async fn push_local_edit(&self, note: &Note) {
        let mut notes = self.shared.notes.lock().await;
        let lines = notes.entry(note.id).or_default();
        let ops = lines.diff_body(&note.body, &self.shared.device_id);
        drop(notes);
        match ops {
            Ok(ops) if ops.is_empty() => {}
            Ok(ops) => {
                let _ = self.shared.out.send(CollabClientMsg::Op {
                    note_id: note.id,
                    ops,
                });
            }
            Err(violation) => {
                tracing::warn!(
                    error = %violation,
                    note = %note.id,
                    "collab: local body breaks a format limit; no op emitted"
                );
            }
        }
    }
```

**What it does** — Diffs `note.body` against the mirror (creating an empty
mirror for an unknown id) via `NoteLines::diff_body` with this device as
actor, and queues the resulting ops (if any) as one `CollabClientMsg::Op`.
`diff_body` now returns a `Result`: a `LimitViolation` is logged and **no op is
emitted**, and because a failed diff mutates nothing the mirror stays consistent.
Reaching that branch would mean an over-limit body got past `update_note`'s
up-front `format::check_body` (the only caller validates first), so it is a
defensive log rather than an expected path — but silently emitting nothing without
a trace is exactly the failure mode issue keeplin#130 exists to remove.

**Dependencies** — `state::NoteLines::diff_body` (expects a rejected diff to leave
the mirror untouched), the `out` queue.

**Used by** — `update_note`.

**Repeated context** — none.

### fn patch_meta

**Identification** — `async fn patch_meta(&self, note: &Note)`; marker
`// md:impl CollabBackend > fn patch_meta`.

**Code** — complete and verbatim:

```rust
    // md:impl CollabBackend > fn patch_meta
    async fn patch_meta(&self, note: &Note) {
        let url = format!("{}/api/notes/{}", self.shared.cfg.api_url, note.id);
        let body = serde_json::json!({
            "title": note.title,
            "notebook_id": note.notebook_id,
            "is_todo": note.is_todo,
            "todo_due": note.todo_due,
            "todo_completed": note.todo_completed,
        });
        match self
            .shared
            .auth(self.shared.http.patch(url))
            .json(&body)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {}
            Ok(resp) => {
                tracing::warn!(status = %resp.status(), note = %note.id, "collab: PATCH note rejected by server")
            }
            Err(e) => tracing::warn!(error = %e, note = %note.id, "collab: PATCH note failed"),
        }
    }
```

**What it does** — Mirrors title/notebook/todo metadata to the server
(`PATCH /api/notes/{id}`). Checks the HTTP status, not just transport success:
a 4xx/5xx means the server rejected the metadata mirror (e.g. an auth or
validation change), which is logged as a warning rather than treated as
delivered (issue #112). Failures are retried implicitly by the next edit — the
server holds the durable truth and the local row already has the change.

**Dependencies** — `auth`, `reqwest`, `serde_json`.

**Used by** — `create_note`, `update_note`.

**Repeated context** — none.

---

## impl NoteRepository for CollabBackend

**Identification** —
`#[async_trait] impl<B: StorageBackend> NoteRepository for CollabBackend<B>`;
marker `// md:impl NoteRepository for CollabBackend` (one marker for the impl
block; the methods are documented here).

**Code** — complete and verbatim:

```rust
// md:impl NoteRepository for CollabBackend
#[async_trait]
impl<B: StorageBackend> NoteRepository for CollabBackend<B> {
    async fn create_note(&self, note: Note) -> Result<Note, StorageError> {
        if !self.shared.suppressed(note.id).await {
            format::check_body(&note.body)?;
        }
        let created = self.inner.create_note(note).await?;
        if self.shared.suppressed(created.id).await {
            return Ok(created);
        }
        self.shared.pending_push.lock().await.insert(created.id);
        let url = format!("{}/api/notes", self.shared.cfg.api_url);
        let body = serde_json::json!({ "id": created.id, "title": created.title });
        match self
            .shared
            .auth(self.shared.http.post(url))
            .json(&body)
            .send()
            .await
        {
            Ok(resp)
                if resp.status().is_success() || resp.status() == reqwest::StatusCode::CONFLICT => {
            }
            Ok(resp) => {
                tracing::warn!(status = %resp.status(), note = %created.id, "collab: POST note rejected by server")
            }
            Err(e) => tracing::warn!(error = %e, note = %created.id, "collab: POST note failed"),
        }
        self.patch_meta(&created).await;
        let _ = self.shared.out.send(CollabClientMsg::Join {
            note_id: created.id,
        });
        Ok(created)
    }

    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        self.inner.read_note(id).await
    }

    async fn update_note(&self, note: Note) -> Result<Note, StorageError> {
        if !self.shared.suppressed(note.id).await {
            format::check_body(&note.body)?;
        }
        let previous = self.inner.read_note(note.id).await.ok();
        let updated = self.inner.update_note(note).await?;
        if self.shared.suppressed(updated.id).await {
            return Ok(updated);
        }
        let meta_changed = previous.as_ref().is_none_or(|p| {
            p.title != updated.title
                || p.notebook_id != updated.notebook_id
                || p.is_todo != updated.is_todo
                || p.todo_due != updated.todo_due
                || p.todo_completed != updated.todo_completed
        });
        if meta_changed {
            self.patch_meta(&updated).await;
        }
        if !self.shared.pending_push.lock().await.contains(&updated.id) {
            self.push_local_edit(&updated).await;
        }
        Ok(updated)
    }

    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_note(id).await?;
        if !self.shared.suppressed(id).await {
            let url = format!("{}/api/notes/{}", self.shared.cfg.api_url, id);
            if let Err(e) = self.shared.auth(self.shared.http.delete(url)).send().await {
                tracing::warn!(error = %e, note = %id, "collab: DELETE note failed");
            }
            self.shared.notes.lock().await.remove(&id);
        }
        Ok(())
    }

    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        self.inner.list_notes(page_size, page_token).await
    }

    async fn note_backlinks(
        &self,
        target_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        self.inner
            .note_backlinks(target_id, page_size, page_token)
            .await
    }

    async fn list_notes_in_notebook(
        &self,
        notebook_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        self.inner
            .list_notes_in_notebook(notebook_id, page_size, page_token)
            .await
    }

    async fn list_starred_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        self.inner.list_starred_notes(page_size, page_token).await
    }

    async fn notebook_sort_profile(
        &self,
        notebook_id: Uuid,
    ) -> Result<crate::storage::NotebookSortProfile, StorageError> {
        self.inner.notebook_sort_profile(notebook_id).await
    }
}
```

**What it does** — The note surface, where local writes become collab traffic:

- `create_note` — **validate the body first**: unless the write is suppressed
  (server-driven, therefore already accepted by the server), `format::check_body`
  rejects an over-limit body with `StorageError::TooLarge` *before* the inner
  create, so the note never reaches local storage in a state the server would
  refuse. Then inner create; if suppressed (a server-driven write), stop
  there. Otherwise: **mark `pending_push` BEFORE the note becomes visible on
  the server** — once the POST lands, periodic rediscovery can Join the note
  on its own, and if that early Join's (empty) `Welcome` arrived while the
  note was not yet pending, the Welcome handler's cache branch would overwrite
  the fresh local body with the server's empty snapshot, destroying the
  content. Then `POST /api/notes` registering the note under its local id
  (2xx ok; **409 is the expected "already exists"** — created on another
  device — and fine, the body still converges through resolution; any other
  status is logged as a real rejection, issue #112), `patch_meta`, and queue a
  `Join`. The body is pushed when the Join's `Welcome` arrives (reconciled
  against the snapshot), never eagerly; the connection task drops the Join if
  rediscovery already joined (a duplicate Join would fetch a second, possibly
  stale snapshot).
- `update_note` — same up-front `format::check_body` gate as `create_note`, with
  the same suppression exemption, then read the previous row, inner update; if
  suppressed, done.
  `patch_meta` only when title/notebook/todo fields changed; body ops via
  `push_local_edit` — unless the note is still in `pending_push`, in which
  case the first `Welcome`'s reconcile (which reads the latest local body)
  will push it instead of diffing against an uninitialised mirror.
- `delete_note` — inner delete; unless suppressed, `DELETE /api/notes/{id}`
  (transport failures logged) and drop the mirror entry.
- `read_note`, `list_notes`, `note_backlinks` (explicit delegation so inner
  indexes are reached), `list_notes_in_notebook`, `list_starred_notes`,
  `notebook_sort_profile` — pure delegation.

**Dependencies** — the inner backend, `Shared`, `patch_meta`,
`push_local_edit`, `format::check_body` (expects it to be total and cheap — it runs
on every note write — and to be the same gate keeplin-srv applies to line ops, so
a body accepted here is never rejected there for length).

**Used by** — all note traffic in server mode.

**Repeated context** — the suppression check on every write path is the
no-echo invariant; decorators delegating `note_backlinks` is the
indexed-override rule from `storage/backend.md`. Validating **before** the inner
write is the format-limit invariant: local storage must never hold note content
the server has refused.

---

## impl NotebookRepository for CollabBackend

**Identification** — marker `// md:impl NotebookRepository for CollabBackend`.

**Code** — complete and verbatim:

```rust
// md:impl NotebookRepository for CollabBackend
#[async_trait]
impl<B: StorageBackend> NotebookRepository for CollabBackend<B> {
    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        self.inner.create_notebook(notebook).await
    }
    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        self.inner.read_notebook(id).await
    }
    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        self.inner.update_notebook(notebook).await
    }
    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_notebook(id).await
    }
    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        self.inner.list_notebooks(page_size, page_token).await
    }
}
```

**What it does** — Pure delegation for all five notebook methods: notebooks
are not collaborative — they sync over the relay.

**Dependencies** — the inner backend.

**Used by** — notebook traffic.

**Repeated context** — none.

---

## impl TagRepository for CollabBackend

**Identification** — marker `// md:impl TagRepository for CollabBackend`.

**Code** — complete and verbatim:

```rust
// md:impl TagRepository for CollabBackend
#[async_trait]
impl<B: StorageBackend> TagRepository for CollabBackend<B> {
    async fn create_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        self.inner.create_tag(tag).await
    }
    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        self.inner.read_tag(id).await
    }
    async fn update_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        self.inner.update_tag(tag).await
    }
    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_tag(id).await
    }
    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        self.inner.list_tags(page_size, page_token).await
    }
    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        self.inner.add_note_tag(note_tag).await
    }
    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        self.inner.remove_note_tag(note_id, tag_id).await
    }
    async fn list_note_tags(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        self.inner
            .list_note_tags(note_id, page_size, page_token)
            .await
    }
}
```

**What it does** — Pure delegation for all eight tag/association methods (tags
sync over the relay).

**Dependencies** — the inner backend.

**Used by** — tag traffic.

**Repeated context** — none.

---

## impl ResourceRepository for CollabBackend

**Identification** — marker `// md:impl ResourceRepository for CollabBackend`.

**Code** — complete and verbatim:

```rust
// md:impl ResourceRepository for CollabBackend
#[async_trait]
impl<B: StorageBackend> ResourceRepository for CollabBackend<B> {
    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let created = self.inner.create_resource(resource, data.clone()).await?;
        let meta = Change::ResourceCreate {
            resource: created.clone(),
            data: None,
        };
        if let Err(e) = self.inner.send_changes(vec![meta]).await {
            tracing::warn!(error = %e, resource = %created.id, "collab: eager resource metadata push failed");
        }
        self.shared.upload_blob(created.id, data).await;
        Ok(created)
    }
    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let (resource, data) = self.inner.read_resource(id).await?;
        if data.is_empty() && resource.size > 0 {
            if let Some(bytes) = self.shared.download_blob(id).await {
                return Ok((resource, bytes));
            }
            tracing::warn!(resource = %id, "collab: resource blob unavailable from server");
        }
        Ok((resource, data))
    }
    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_resource(id).await
    }
    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        self.inner.list_resources(page_size, page_token).await
    }
    async fn purge_deleted_resources(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, StorageError> {
        self.inner.purge_deleted_resources(older_than).await
    }
}
```

**What it does** —

- `create_resource` — inner create, then **eagerly** push the blob-stripped
  `ResourceCreate` over the relay (`inner.send_changes`) and upload the binary
  out-of-band (`upload_blob`). Rationale: the server accepts a blob only for
  already-materialised metadata, and the *periodic* relay cycle always loses
  that race for a brand-new resource. The periodic cycle may re-send the same
  change later; server materialisation is version-vector-idempotent, so the
  duplicate is harmless.
- `read_resource` — inner read; when the local cache has no bytes but
  `size > 0` (metadata synced from another device), fetch from the server
  (`download_blob`), falling back to the empty local data with a warning.
- `delete_resource`, `list_resources`, `purge_deleted_resources` —
  delegation.

**Dependencies** — the inner backend, `upload_blob`, `download_blob`,
`Change::ResourceCreate`.

**Used by** — resource traffic in server mode.

**Repeated context** — none.

---

## impl SyncBackend for CollabBackend

**Identification** — marker `// md:impl SyncBackend for CollabBackend`.

**Code** — complete and verbatim:

```rust
// md:impl SyncBackend for CollabBackend
#[async_trait]
impl<B: StorageBackend> SyncBackend for CollabBackend<B> {
    async fn get_device_id(&self) -> Result<String, StorageError> {
        self.inner.get_device_id().await
    }
    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
        self.inner.get_last_sync_time().await
    }
    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        self.inner.update_sync_time(ts).await
    }
    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError> {
        let changes = self.inner.get_changes_since(since).await?;
        Ok(changes
            .into_iter()
            .filter(|c| !is_note_change(c))
            .map(strip_resource_blob)
            .collect())
    }
    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        if is_note_change(&change) {
            return Ok(());
        }
        self.inner.apply_change(change).await
    }
    async fn send_changes(&self, changes: Vec<Change>) -> Result<(), StorageError> {
        self.inner.send_changes(changes).await
    }
    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        self.inner.receive_changes().await
    }
    async fn prune_change_journal(&self, older_than: DateTime<Utc>) -> Result<u64, StorageError> {
        self.inner.prune_change_journal(older_than).await
    }
}
```

**What it does** — Delegation for `get_device_id` / `get_last_sync_time` /
`update_sync_time` / `send_changes` / `receive_changes` /
`prune_change_journal`, with the two note filters:

- `get_changes_since` — inner changes, **minus note changes**
  (`is_note_change`) and with resource blobs stripped (`strip_resource_blob`):
  the collab channel owns notes; blobs are uploaded out-of-band and must not
  bloat the relay journal.
- `apply_change` — a note change is dropped (`Ok(())`); everything else
  delegates.

**Dependencies** — `is_note_change`, `strip_resource_blob`, the inner backend.

**Used by** — the relay sync cycle in server mode.

**Repeated context** — this filter is the "notes travel exactly one path"
invariant.

---

## impl HistoryRepository for CollabBackend

**Identification** — marker `// md:impl HistoryRepository for CollabBackend`.

**Code** — complete and verbatim:

```rust
// md:impl HistoryRepository for CollabBackend
#[async_trait]
impl<B: StorageBackend> crate::storage::HistoryRepository for CollabBackend<B> {
    async fn note_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<crate::storage::EntityVersion<Note>>, StorageError> {
        self.inner.note_history(id, limit).await
    }

    async fn notebook_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<crate::storage::EntityVersion<Notebook>>, StorageError> {
        self.inner.notebook_history(id, limit).await
    }
}
```

**What it does** — Pure delegation of `note_history`/`notebook_history`.

**Dependencies** — the inner backend.

**Used by** — history endpoints.

**Repeated context** — none.

---

## fn is_note_change

**Identification** — `fn is_note_change(change: &Change) -> bool`; marker
`// md:fn is_note_change`.

**Code** — complete and verbatim:

```rust
// md:fn is_note_change
fn is_note_change(change: &Change) -> bool {
    matches!(
        change,
        Change::NoteCreate { .. } | Change::NoteUpdate { .. } | Change::NoteDelete { .. }
    )
}
```

**What it does** — `true` for `NoteCreate`/`NoteUpdate`/`NoteDelete`.

**Dependencies** — `Change`.

**Used by** — the `SyncBackend` filters.

**Repeated context** — none.

---

## fn strip_resource_blob

**Identification** — `fn strip_resource_blob(change: Change) -> Change`; marker
`// md:fn strip_resource_blob`.

**Code** — complete and verbatim:

```rust
// md:fn strip_resource_blob
fn strip_resource_blob(change: Change) -> Change {
    match change {
        Change::ResourceCreate { resource, .. } => Change::ResourceCreate {
            resource,
            data: None,
        },
        other => other,
    }
}
```

**What it does** — Drops the inline binary from a `ResourceCreate` before it
is relayed: with keeplin-srv the blob is uploaded out-of-band and served from
the server, so carrying it in the change would duplicate it and bloat the
journal.

**Dependencies** — `Change`.

**Used by** — `get_changes_since`.

**Repeated context** — none.

---

## ServerNote

**Identification** — private
`#[derive(Debug, serde::Deserialize)] struct ServerNote`; marker
`// md:ServerNote`.

**Code** — complete and verbatim:

```rust
// md:ServerNote
#[derive(Debug, serde::Deserialize)]
struct ServerNote {
    id: Uuid,
    title: String,
    notebook_id: Option<Uuid>,
    is_todo: bool,
    todo_due: Option<DateTime<Utc>>,
    todo_completed: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
```

**What it does** — The server's note representation from `GET /api/notes`:
`id`, `title`, `notebook_id: Option<Uuid>`, the todo fields, timestamps. No
body — bodies arrive via `Welcome` snapshots.

**Dependencies** — `serde`, `chrono`, `uuid`.

**Used by** — `discover_and_join`, `ensure_local`.

**Repeated context** — none.

---

## fn run_connection

**Identification** —
`async fn run_connection(shared: Arc<Shared>, out: mpsc::UnboundedReceiver<CollabClientMsg>)`;
marker `// md:fn run_connection`.

**Code** — complete and verbatim:

```rust
// md:fn run_connection
async fn run_connection(shared: Arc<Shared>, mut out: mpsc::UnboundedReceiver<CollabClientMsg>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match connect_once(&shared, &mut out).await {
            Ok(()) => backoff = Duration::from_secs(1),
            Err(e) => {
                tracing::warn!(error = %e, "collab: connection ended; reconnecting");
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}
```

**What it does** — Maintains the WebSocket connection forever: `connect_once`
in a loop, resetting the backoff to 1 s after a clean run and doubling it up
to 30 s after failures. Every reconnect re-discovers and re-joins, so state is
rebuilt from snapshots.

**Dependencies** — `connect_once`, `tokio::time`.

**Used by** — spawned by `CollabBackend::start`.

**Repeated context** — none.

---

## REDISCOVER_EVERY

**Identification** —
`const REDISCOVER_EVERY: Duration = Duration::from_secs(15);` marker
`// md:REDISCOVER_EVERY`.

**Code** — complete and verbatim:

```rust
// md:REDISCOVER_EVERY
const REDISCOVER_EVERY: Duration = Duration::from_secs(15);
```

**What it does** — How often a live connection re-runs note discovery, so
notes created on other devices — or newly shared with this user — get joined
without a reconnect.

**Dependencies** — none.

**Used by** — `connect_once`.

**Repeated context** — none.

---

## fn connect_once

**Identification** —
`async fn connect_once(shared: &Arc<Shared>, out: &mut mpsc::UnboundedReceiver<CollabClientMsg>) -> anyhow::Result<()>`;
marker `// md:fn connect_once`.

**Code** — complete and verbatim:

```rust
// md:fn connect_once
async fn connect_once(
    shared: &Arc<Shared>,
    out: &mut mpsc::UnboundedReceiver<CollabClientMsg>,
) -> anyhow::Result<()> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = shared.cfg.ws_url.as_str().into_client_request()?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", shared.cfg.token).parse()?,
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await?;
    tracing::info!("collab: connected");

    let mut joined: HashSet<Uuid> = HashSet::new();
    discover_and_join(shared, &mut ws, &mut joined).await?;

    let mut rediscover = tokio::time::interval(REDISCOVER_EVERY);
    rediscover.tick().await;

    loop {
        tokio::select! {
            queued = out.recv() => {
                let Some(msg) = queued else { return Ok(()) };
                match &msg {
                    CollabClientMsg::Join { note_id } => {
                        if !joined.insert(*note_id) {
                            continue;
                        }
                    }
                    CollabClientMsg::Leave { note_id } => {
                        joined.remove(note_id);
                    }
                    _ => {}
                }
                ws.send(Message::Text(serde_json::to_string(&msg)?)).await?;
            }
            incoming = ws.next() => {
                let Some(frame) = incoming else { anyhow::bail!("socket closed") };
                if let Message::Text(text) = frame? {
                    if let Ok(msg) = serde_json::from_str::<CollabServerMsg>(&text) {
                        handle_server_msg(shared, msg).await;
                    }
                }
            }
            _ = rediscover.tick() => {
                discover_and_join(shared, &mut ws, &mut joined).await?;
            }
        }
    }
}
```

**What it does** — One connection lifetime: build the WebSocket request with
the token in the **Authorization header, not the URL** (query strings end up
in proxy and access logs), connect, run `discover_and_join`, then a `select!`
loop over three arms:

- **outbound queue** — a queued `Join` for a note this connection already
  joined is dropped (the rediscovery got there first; a duplicate Join would
  fetch a second snapshot that may predate ops just sent, and its `Welcome`
  would clobber the mirror with that stale state); a `Leave` **clears** the note
  from `joined` before being sent, so the `Leave`/`Join` pair `resync_note`
  queues is not swallowed by that de-duplication and does produce a fresh
  `Welcome`; everything else is serialised and sent. A closed queue ends the task
  cleanly.
- **incoming frames** — text frames parsed as `CollabServerMsg` →
  `handle_server_msg`; a closed socket bails (triggering reconnect).
- **rediscovery tick** (every `REDISCOVER_EVERY`; the immediate first tick is
  consumed up front) — `discover_and_join` again.

**Dependencies** — `tokio_tungstenite`, `discover_and_join`,
`handle_server_msg`, `serde_json`.

**Used by** — `run_connection`.

**Repeated context** — none.

---

## DISCOVER_PAGE_SIZE

**Identification** — `const DISCOVER_PAGE_SIZE: &str = "200";` marker
`// md:DISCOVER_PAGE_SIZE`.

**Code** — complete and verbatim:

```rust
// md:DISCOVER_PAGE_SIZE
const DISCOVER_PAGE_SIZE: &str = "200";
```

**What it does** — Page size for note discovery. The server caps `?limit` at
500 (keeplin-srv #29); a smaller page keeps each round-trip bounded on large
accounts.

**Dependencies** — none.

**Used by** — `discover_and_join`.

**Repeated context** — none.

---

## fn discover_and_join

**Identification** —
`async fn discover_and_join(shared, ws: &mut WsStream, joined: &mut HashSet<Uuid>) -> anyhow::Result<()>`;
marker `// md:fn discover_and_join`.

**Code** — complete and verbatim:

```rust
// md:fn discover_and_join
async fn discover_and_join(
    shared: &Arc<Shared>,
    ws: &mut WsStream,
    joined: &mut HashSet<Uuid>,
) -> anyhow::Result<()> {
    let mut cursor: Option<String> = None;
    loop {
        let mut req = shared
            .auth(shared.http.get(format!("{}/api/notes", shared.cfg.api_url)))
            .query(&[("limit", DISCOVER_PAGE_SIZE)]);
        if let Some(c) = &cursor {
            req = req.query(&[("cursor", c.as_str())]);
        }
        let resp = req.send().await?;
        let next = resp
            .headers()
            .get("x-next-cursor")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let listing: Vec<ServerNote> = resp.json().await?;

        for server_note in &listing {
            if joined.contains(&server_note.id) {
                continue;
            }
            ensure_local(shared, server_note).await;
            let join = serde_json::to_string(&CollabClientMsg::Join {
                note_id: server_note.id,
            })?;
            ws.send(Message::Text(join)).await?;
            joined.insert(server_note.id);
        }

        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(())
}
```

**What it does** — Discovers own + shared notes and joins the ones this
connection hasn't joined yet. Pages through `GET /api/notes?limit=&cursor=`,
following the `X-Next-Cursor` header until exhausted, so an account with
thousands of notes never materialises the whole listing at once;
back-compatible with a pre-pagination server (it ignores `?limit`, returns one
array with no cursor header — the loop runs exactly once). Unknown notes are
created locally via `ensure_local` (empty body — the `Welcome` snapshot fills
it in); each new note gets a `Join` sent directly on the socket and its id
recorded in `joined`.

**Dependencies** — `auth`, `reqwest`, `ServerNote`, `ensure_local`,
`CollabClientMsg::Join`.

**Used by** — `connect_once` (at connect and on every rediscovery tick).

**Repeated context** — none.

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

**What it does** — The client-side WebSocket stream type (plain TCP or TLS).

**Dependencies** — `tokio_tungstenite`.

**Used by** — `discover_and_join`'s signature.

**Repeated context** — none.

---

## fn ensure_local

**Identification** —
`async fn ensure_local(shared: &Arc<Shared>, server_note: &ServerNote)`; marker
`// md:fn ensure_local`.

**Code** — complete and verbatim:

```rust
// md:fn ensure_local
async fn ensure_local(shared: &Arc<Shared>, server_note: &ServerNote) {
    let Some(top) = shared.top.get() else { return };
    if top.read_note(server_note.id).await.is_ok() {
        return;
    }
    let note = Note {
        id: server_note.id,
        title: server_note.title.clone(),
        body: String::new(),
        notebook_id: server_note.notebook_id.unwrap_or_else(Uuid::nil),
        is_todo: server_note.is_todo,
        todo_due: server_note.todo_due,
        todo_completed: server_note.todo_completed,
        created_at: server_note.created_at,
        updated_at: server_note.updated_at,
        ..Note::new("", "")
    };
    if let Err(e) = shared.apply_from_server(note, true).await {
        tracing::warn!(error = %e, note = %server_note.id, "collab: local create failed");
    }
}
```

**What it does** — Makes sure a note discovered on the server exists locally:
no-op when `top` is unset or the note already reads; otherwise builds a local
`Note` from the server metadata (empty body; a missing server notebook maps to
the nil UUID — locally the Inbox system notebook) and creates it through
`apply_from_server` (suppressed). Failures logged.

**Dependencies** — `apply_from_server`, `Note`.

**Used by** — `discover_and_join`.

**Repeated context** — none.

---

## fn handle_server_msg

**Identification** —
`async fn handle_server_msg(shared: &Arc<Shared>, msg: CollabServerMsg)`;
marker `// md:fn handle_server_msg`.

**Code** — complete and verbatim:

```rust
// md:fn handle_server_msg
async fn handle_server_msg(shared: &Arc<Shared>, msg: CollabServerMsg) {
    match msg {
        CollabServerMsg::Welcome { note_id, snapshot } => {
            let mut lines = NoteLines::from_snapshot(snapshot);
            if shared.pending_push.lock().await.remove(&note_id) {
                let local_body = match shared.top.get() {
                    Some(top) => top
                        .read_note(note_id)
                        .await
                        .map(|n| n.body)
                        .unwrap_or_default(),
                    None => String::new(),
                };
                match lines.diff_body(&local_body, &shared.device_id) {
                    Ok(ops) => {
                        shared.notes.lock().await.insert(note_id, lines);
                        if !ops.is_empty() {
                            let _ = shared.out.send(CollabClientMsg::Op { note_id, ops });
                        }
                    }
                    Err(violation) => {
                        tracing::warn!(
                            error = %violation,
                            note = %note_id,
                            "collab: local body breaks a format limit; adopting the server snapshot"
                        );
                        let body = lines.materialize();
                        shared.notes.lock().await.insert(note_id, lines);
                        write_body(shared, note_id, body).await;
                    }
                }
            } else {
                let body = lines.materialize();
                shared.notes.lock().await.insert(note_id, lines);
                write_body(shared, note_id, body).await;
            }
        }
        CollabServerMsg::Op { note_id, ops, .. } => {
            let mut notes = shared.notes.lock().await;
            let Some(lines) = notes.get_mut(&note_id) else {
                return;
            };
            for op in &ops {
                lines.apply(op);
            }
            let body = lines.materialize();
            drop(notes);
            write_body(shared, note_id, body).await;
        }
        CollabServerMsg::Error {
            code,
            message,
            note_id,
        } => match note_id.filter(|_| format::is_limit_code(&code)) {
            Some(note_id) => {
                tracing::warn!(
                    code,
                    message,
                    note = %note_id,
                    "collab: server rejected an op over a format limit; resynchronising the note"
                );
                resync_note(shared, note_id).await;
            }
            None => {
                tracing::warn!(code, message, ?note_id, "collab: server error");
            }
        },
        CollabServerMsg::Presence { note_id, users } => {
            shared.presence.lock().await.insert(note_id, users);
        }
    }
}
```

**What it does** — Dispatches one server message:

- `Welcome { note_id, snapshot }` — build the mirror from the snapshot. If the
  note was in `pending_push` (local content the server has not seen — it was
  just created, or edited before this first Welcome): **do not overwrite** —
  read the latest local body from `top`, diff it against the snapshot into
  ops, install the mirror, queue the ops; the local body stays as-is (already
  equal to what was diffed). Otherwise (a note we only cache): install the
  mirror and write the server's materialised body locally via `write_body`.
- `Op { note_id, ops, .. }` — apply each op to the mirror (unknown note:
  ignore), then `write_body` the new materialisation.
- `Error { code, message, note_id }` — a **format-limit rejection** carrying a
  note (`format::is_limit_code(&code)` and `note_id` present) is acted on, not
  merely logged: `resync_note` drops the cached mirror and forces a rejoin, so the
  server's snapshot replaces whatever the local device was showing. Any other code
  (or a limit code with no note attached) stays a warning, as before. This is the
  fix for the divergence audited on issue keeplin#130: an op the server refused
  used to leave the edit sitting on one device, "saved", forever unsynced.
- `Presence { note_id, users }` — replace that note's presence list.

If the pending-push diff itself breaks a format limit — a local body that predates
these limits, or one written by a path that does not revalidate — the client logs
the violation and **adopts the server snapshot** instead of pushing: the note
converges downwards to something both sides accept rather than staying divergent.

**Dependencies** — `NoteLines` (its `diff_body` now returns
`Result<_, LimitViolation>`; expects a failed diff to leave the mirror untouched,
which is what makes the adopt-the-snapshot fallback safe), `format::is_limit_code`
(expects the same code strings keeplin-srv sends), `resync_note`, `write_body`, the
`pending_push`/`notes`/`presence` state.

**Used by** — `connect_once`.

**Repeated context** — the pending-push reconcile is the "late empty Welcome
must not clobber local content" invariant, restated in `create_note`. The
limit-rejection branch adds its mirror image: **a rejected op must not leave local
content the server does not have** — the client resynchronises instead.

---

## fn resync_note

**Identification** — `async fn resync_note(shared: &Arc<Shared>, note_id: Uuid)`;
marker `// md:fn resync_note`.

**Code** — complete and verbatim:

```rust
// md:fn resync_note
async fn resync_note(shared: &Arc<Shared>, note_id: Uuid) {
    shared.notes.lock().await.remove(&note_id);
    shared.pending_push.lock().await.remove(&note_id);
    let _ = shared.out.send(CollabClientMsg::Leave { note_id });
    let _ = shared.out.send(CollabClientMsg::Join { note_id });
}
```

**What it does** — Throws away everything the client believes about a note and
asks the server to state it again. Three steps, in this order: drop the cached
`NoteLines` mirror; clear any `pending_push` marker so the coming `Welcome` takes
the **adopt-the-snapshot** branch rather than the push-local-content branch; queue
`Leave` then `Join`. The queue is FIFO and `connect_once` removes the note from its
`joined` set when it forwards the `Leave`, so the `Join` is genuinely re-sent and a
fresh `Welcome` arrives; `handle_server_msg` then materialises the server's body
and `write_body` writes it locally under `suppress`, which reverts the divergent
local edit without echoing it back as a new op.

Called only when the server rejects an op over a format limit. Termination is by
construction: the resynchronised body is the server's own, so it cannot be
rejected again, and the suppressed local write emits no further ops.

**Dependencies** — `Shared::notes` / `Shared::pending_push` (expects both to be the
only caches of note state — a third cache would survive the reset and keep the
divergence), `Shared::out` (expects FIFO delivery, so `Leave` is processed before
`Join`), `connect_once`'s `Leave` handling (expects it to clear `joined`; without
that the `Join` would be de-duplicated away and no `Welcome` would come).

**Used by** — `handle_server_msg`, on a limit-coded `Error` carrying a `note_id`.

**Repeated context** — writes that originate from the server go through
`apply_from_server`, which sets the `suppress` flag so the decorator does not
re-publish them; that is what keeps a resync from looping.

---

## fn write_body

**Identification** —
`async fn write_body(shared: &Arc<Shared>, note_id: Uuid, body: String)`;
marker `// md:fn write_body`.

**Code** — complete and verbatim:

```rust
// md:fn write_body
async fn write_body(shared: &Arc<Shared>, note_id: Uuid, body: String) {
    let Some(top) = shared.top.get() else { return };
    match top.read_note(note_id).await {
        Ok(mut note) => {
            if note.body != body {
                note.body = body;
                note.updated_at = Utc::now();
                if let Err(e) = shared.apply_from_server(note, false).await {
                    tracing::warn!(error = %e, note = %note_id, "collab: local update failed");
                }
            }
        }
        Err(_) => {
            let mut note = Note::new("", body);
            note.id = note_id;
            if let Err(e) = shared.apply_from_server(note, true).await {
                tracing::warn!(error = %e, note = %note_id, "collab: local create failed");
            }
        }
    }
}
```

**What it does** — Persists a server-derived body locally (suppressed),
keeping metadata: read the note from `top`; if it exists and the body differs,
set body + `updated_at = now` and `apply_from_server(update)`; if it does not
exist, create a fresh note with that id and body. Equal bodies are a no-op
(no version churn). Failures logged.

**Dependencies** — `apply_from_server`, `Note`.

**Used by** — `handle_server_msg` (`Welcome` cache branch and `Op`).

**Repeated context** — none.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `Shared` — defined here (EXTRACTED; 85 cross-file edge(s))
- `CollabBackend<B>` — defined here (EXTRACTED; 6 cross-file edge(s))
- `CollabBackend` — defined here (EXTRACTED; 5 cross-file edge(s))
- `.note_history()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `.notebook_history()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `.apply_from_server()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `CollabHandle` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.start()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.create_note()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.read_note()` — defined here (EXTRACTED; 2 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/collab/protocol.rs` — collaborative channel wire types (EXTRACTED: references×8; e.g. `CollabClientMsg`, `PresenceInfo`, `Cursor`)
- `keeplin-core/src/collab/state.rs` — client line state and body↔lines translation (EXTRACTED: imports_from×1, references×1; e.g. `NoteLines`)
- `keeplin-core/src/error.rs` — error types (EXTRACTED: imports_from×1, references×41; e.g. `StorageError`)
- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: references×31; e.g. `Note`, `Notebook`, `Tag`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: implements×6, references×5; e.g. `StorageBackend`, `NotebookRepository`, `NoteRepository`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/tests/collab_client.rs` — collaborative client tests (state machine + mock server e2e) (EXTRACTED: references×2; e.g. `client()`, `wait_body()`)
- `keeplin-daemon/src/main.rs` — daemon entry point (EXTRACTED: imports_from×1, references×3; e.g. `collab_config()`, `run_server_with()`, `collab_starter()`)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: references×82; e.g. `add_link()`, `add_note_tag()`, `auth_mw()`)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `CollabConfig` | `// md:CollabConfig` |
| 3 | `fn device_id_from_token` | `// md:fn device_id_from_token` |
| 4 | `Shared` | `// md:Shared` |
| 5 | `impl Shared` (container) | `// md:impl Shared` |
| 6 | `fn auth` | `// md:impl Shared > fn auth` |
| 7 | `fn suppressed` | `// md:impl Shared > fn suppressed` |
| 8 | `fn apply_from_server` | `// md:impl Shared > fn apply_from_server` |
| 9 | `fn upload_blob` | `// md:impl Shared > fn upload_blob` |
| 10 | `fn download_blob` | `// md:impl Shared > fn download_blob` |
| 11 | `CollabHandle` | `// md:CollabHandle` |
| 12 | `impl CollabHandle` (container) | `// md:impl CollabHandle` |
| 13 | `fn presence` | `// md:impl CollabHandle > fn presence` |
| 14 | `fn send_cursor` | `// md:impl CollabHandle > fn send_cursor` |
| 15 | `fn proxy_request` | `// md:impl CollabHandle > fn proxy_request` |
| 16 | `CollabBackend` | `// md:CollabBackend` |
| 17 | `impl Clone for CollabBackend` | `// md:impl Clone for CollabBackend` |
| 18 | `impl CollabBackend` (container) | `// md:impl CollabBackend` |
| 19 | `fn new` | `// md:impl CollabBackend > fn new` |
| 20 | `fn handle` | `// md:impl CollabBackend > fn handle` |
| 21 | `fn start` | `// md:impl CollabBackend > fn start` |
| 22 | `fn push_local_edit` | `// md:impl CollabBackend > fn push_local_edit` |
| 23 | `fn patch_meta` | `// md:impl CollabBackend > fn patch_meta` |
| 24 | `impl NoteRepository for CollabBackend` | `// md:impl NoteRepository for CollabBackend` |
| 25 | `impl NotebookRepository for CollabBackend` | `// md:impl NotebookRepository for CollabBackend` |
| 26 | `impl TagRepository for CollabBackend` | `// md:impl TagRepository for CollabBackend` |
| 27 | `impl ResourceRepository for CollabBackend` | `// md:impl ResourceRepository for CollabBackend` |
| 28 | `impl SyncBackend for CollabBackend` | `// md:impl SyncBackend for CollabBackend` |
| 29 | `impl HistoryRepository for CollabBackend` | `// md:impl HistoryRepository for CollabBackend` |
| 30 | `fn is_note_change` | `// md:fn is_note_change` |
| 31 | `fn strip_resource_blob` | `// md:fn strip_resource_blob` |
| 32 | `ServerNote` | `// md:ServerNote` |
| 33 | `fn run_connection` | `// md:fn run_connection` |
| 34 | `REDISCOVER_EVERY` | `// md:REDISCOVER_EVERY` |
| 35 | `fn connect_once` | `// md:fn connect_once` |
| 36 | `DISCOVER_PAGE_SIZE` | `// md:DISCOVER_PAGE_SIZE` |
| 37 | `fn discover_and_join` | `// md:fn discover_and_join` |
| 38 | `WsStream` | `// md:WsStream` |
| 39 | `fn ensure_local` | `// md:fn ensure_local` |
| 40 | `fn handle_server_msg` | `// md:fn handle_server_msg` |
| 41 | `fn resync_note` | `// md:fn resync_note` |
| 42 | `fn write_body` | `// md:fn write_body` |