//! Client side of the keeplin-srv collaborative channel.
//!
//! Architecture: frontends talk to this daemon (gRPC/REST/feed); the daemon —
//! through [`CollabBackend`] — talks to keeplin-srv. The server stores every
//! note (lines + versioned order + metadata) and is the durable source of
//! truth; this client keeps the user's notes (own and shared) in the local
//! database and mirrors line state in memory, rebuilt from each `Welcome`
//! snapshot on (re)connect.
//!
//! - Local note writes are diffed into [`protocol::LineOp`]s (signed with this
//!   *device*'s id, the vv actor) and pushed over the WebSocket; title and
//!   metadata changes go over REST.
//! - Remote ops and snapshots are applied through the daemon's full decorator
//!   stack (so links are re-derived and the live feed fires), with an
//!   in-flight suppression set preventing those writes from echoing back.
//! - Note discovery (own + shared) is a REST `GET /api/notes` at connect.
//! - Note `Change`s are filtered out of the relay sync path: the collab
//!   channel owns notes; notebooks/tags/resources keep syncing via the relay.

pub mod protocol;
pub mod state;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{Change, Note, NoteTag, Notebook, Resource, Tag};
use crate::storage::{
    NoteRepository, NotebookRepository, ResourceRepository, StorageBackend, SyncBackend,
    TagRepository,
};
use protocol::{CollabClientMsg, CollabServerMsg};
use state::NoteLines;

/// Connection settings for the collaborative channel.
#[derive(Debug, Clone)]
pub struct CollabConfig {
    /// HTTP base of keeplin-srv, e.g. `http://host:3000`.
    pub api_url: String,
    /// WebSocket endpoint, e.g. `ws://host:3000/api/ws`.
    pub ws_url: String,
    /// Device token from `POST /api/login` (one per device).
    pub token: String,
}

/// Extract the `device_id` claim from a JWT without verifying it (the server
/// verifies; the client only needs to know its own identity to sign ops).
pub fn device_id_from_token(token: &str) -> Option<String> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    Some(claims.get("device_id")?.as_str()?.to_string())
}

struct Shared {
    cfg: CollabConfig,
    device_id: String,
    http: reqwest::Client,
    /// Per-note in-memory mirror of the server's line entities.
    notes: Mutex<HashMap<Uuid, NoteLines>>,
    /// Note ids currently being written *from* the server, so the decorator
    /// does not diff them back into ops (no echo).
    suppress: Mutex<HashSet<Uuid>>,
    /// Latest presence list per note, as broadcast by the server after every
    /// join/leave/cursor move. Served to frontends via the daemon's REST API.
    presence: Mutex<HashMap<Uuid, Vec<protocol::PresenceInfo>>>,
    /// Outbound queue drained by the connection task.
    out: mpsc::UnboundedSender<CollabClientMsg>,
    /// Notes with local content not yet on the server (freshly created, or
    /// edited before the note's first `Welcome`). Their body is reconciled
    /// against the join snapshot instead of being pushed eagerly, so a late
    /// (empty) `Welcome` cannot clobber the local content.
    pending_push: Mutex<HashSet<Uuid>>,
    /// Top of the daemon's decorator stack, set once after construction, used
    /// to apply remote state so linking/eventing run on those writes too.
    top: OnceLock<Arc<dyn StorageBackend>>,
}

impl Shared {
    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.bearer_auth(&self.cfg.token)
    }

    async fn suppressed(&self, id: Uuid) -> bool {
        self.suppress.lock().await.contains(&id)
    }

    /// Write `note` through the top of the stack with echo suppression.
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

    /// Upload a resource's binary payload to keeplin-srv out-of-band, so the
    /// bytes never ride the relay journal (they are stripped from the relayed
    /// `ResourceCreate`). Best effort: a failure is logged and the metadata
    /// still syncs; the blob can be re-uploaded by a later read/retry.
    async fn upload_blob(&self, id: Uuid, data: Vec<u8>) {
        let url = format!("{}/api/resources/{}/data", self.cfg.api_url, id);
        if let Err(e) = self.auth(self.http.put(url)).body(data).send().await {
            tracing::warn!(error = %e, resource = %id, "collab: resource blob upload failed");
        }
    }

    /// Fetch a resource's binary payload from keeplin-srv (the source of truth
    /// for blobs). `None` on any failure, so the caller falls back to whatever
    /// the local cache holds.
    async fn download_blob(&self, id: Uuid) -> Option<Vec<u8>> {
        let url = format!("{}/api/resources/{}/data", self.cfg.api_url, id);
        let resp = self.auth(self.http.get(url)).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.bytes().await.ok().map(|b| b.to_vec())
    }
}

/// An ungeneric, cloneable view of the collaborative session for the daemon's
/// HTTP/gRPC surfaces: read presence and publish this device's cursor without
/// knowing the storage type behind [`CollabBackend`].
#[derive(Clone)]
pub struct CollabHandle {
    shared: Arc<Shared>,
}

impl CollabHandle {
    /// The latest presence list the server broadcast for `note_id` (empty if
    /// the note has no live session or is unknown).
    pub async fn presence(&self, note_id: Uuid) -> Vec<protocol::PresenceInfo> {
        self.shared
            .presence
            .lock()
            .await
            .get(&note_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Queue this device's caret position for `note_id`; the server fans the
    /// updated presence out to every participant.
    pub fn send_cursor(&self, note_id: Uuid, cursor: protocol::Cursor) {
        let _ = self
            .shared
            .out
            .send(CollabClientMsg::Cursor { note_id, cursor });
    }

    /// Forward a **permission-management** request (share, transfer, list, revoke) to
    /// keeplin-srv — the authority for permissions in server mode — and return the server's
    /// status code and JSON body. The daemon's REST surface proxies its own permission
    /// endpoints through this, so a frontend can view/manage shares on demand without the
    /// client re-implementing (or locally enforcing) the model. `method` is an HTTP verb
    /// (`"GET"`, `"POST"`, `"DELETE"`); `path` is a server path like `/api/notes/<id>/share`.
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
}

/// Storage decorator that turns local note writes into collaborative ops and
/// REST calls. Sits *below* `LinkingBackend`/`EventBackend` in the stack.
pub struct CollabBackend<B> {
    inner: Arc<B>,
    shared: Arc<Shared>,
    out_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<CollabClientMsg>>>>,
}

// Manual impl: `B` itself need not be `Clone` (everything is behind `Arc`s).
impl<B> Clone for CollabBackend<B> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            shared: self.shared.clone(),
            out_rx: self.out_rx.clone(),
        }
    }
}

impl<B: StorageBackend> CollabBackend<B> {
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

    /// A cloneable presence/cursor view for the daemon's surfaces.
    pub fn handle(&self) -> CollabHandle {
        CollabHandle {
            shared: self.shared.clone(),
        }
    }

    /// Start the connection task. `top` must be the outermost backend of the
    /// daemon's stack (this backend wrapped by linking/eventing) so remote
    /// writes flow through every decorator exactly once.
    pub async fn start(&self, top: Arc<dyn StorageBackend>) {
        let _ = self.shared.top.set(top);
        let rx = self
            .out_rx
            .lock()
            .await
            .take()
            .expect("collab task started twice");
        tokio::spawn(run_connection(self.shared.clone(), rx));
    }

    /// Diff `note.body` against the mirror and queue the resulting ops.
    async fn push_local_edit(&self, note: &Note) {
        let mut notes = self.shared.notes.lock().await;
        let lines = notes.entry(note.id).or_default();
        let ops = lines.diff_body(&note.body, &self.shared.device_id);
        drop(notes);
        if !ops.is_empty() {
            let _ = self.shared.out.send(CollabClientMsg::Op {
                note_id: note.id,
                ops,
            });
        }
    }

    /// Mirror title/metadata to the server; failures are logged and retried
    /// implicitly by the next edit (the server holds the durable truth, the
    /// local row already has the change).
    async fn patch_meta(&self, note: &Note) {
        let url = format!("{}/api/notes/{}", self.shared.cfg.api_url, note.id);
        let body = serde_json::json!({
            "title": note.title,
            "notebook_id": note.notebook_id,
            "is_todo": note.is_todo,
            "todo_due": note.todo_due,
            "todo_completed": note.todo_completed,
        });
        if let Err(e) = self
            .shared
            .auth(self.shared.http.patch(url))
            .json(&body)
            .send()
            .await
        {
            tracing::warn!(error = %e, note = %note.id, "collab: PATCH note failed");
        }
    }
}

#[async_trait]
impl<B: StorageBackend> NoteRepository for CollabBackend<B> {
    async fn create_note(&self, note: Note) -> Result<Note, StorageError> {
        let created = self.inner.create_note(note).await?;
        if self.shared.suppressed(created.id).await {
            return Ok(created);
        }
        // Register the note on the server keeping the local id. A 409 means it
        // already exists there (e.g. created on another device); the body still
        // converges through resolution.
        let url = format!("{}/api/notes", self.shared.cfg.api_url);
        let body = serde_json::json!({ "id": created.id, "title": created.title });
        if let Err(e) = self
            .shared
            .auth(self.shared.http.post(url))
            .json(&body)
            .send()
            .await
        {
            tracing::warn!(error = %e, note = %created.id, "collab: POST note failed");
        }
        self.patch_meta(&created).await;
        // Mark the body as pending and Join. The body is pushed when the Join's
        // `Welcome` arrives (reconciled against the snapshot), NOT eagerly:
        // pushing before the `Welcome` lets a late empty snapshot clobber the
        // local content. See `handle_server_msg`.
        self.shared.pending_push.lock().await.insert(created.id);
        let _ = self.shared.out.send(CollabClientMsg::Join {
            note_id: created.id,
        });
        Ok(created)
    }

    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        self.inner.read_note(id).await
    }

    async fn update_note(&self, note: Note) -> Result<Note, StorageError> {
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
        // If the note is still pending its first `Welcome`, leave the push to
        // that reconcile (which reads the latest local body) rather than
        // diffing against a mirror that has not been initialised yet.
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

#[async_trait]
impl<B: StorageBackend> ResourceRepository for CollabBackend<B> {
    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let created = self.inner.create_resource(resource, data.clone()).await?;
        // Upload the binary to keeplin-srv out-of-band; the metadata syncs over
        // the relay (with the blob stripped, see `get_changes_since`).
        self.shared.upload_blob(created.id, data).await;
        Ok(created)
    }
    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let (resource, data) = self.inner.read_resource(id).await?;
        // A blob that never arrived over the relay (metadata synced from another
        // device) is not in the local cache; fetch it from the server, the
        // source of truth for resource binaries.
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
    /// Note changes are owned by the collaborative channel; the relay only
    /// carries notebooks, tags and resources. Resource binaries are stripped:
    /// they are uploaded out-of-band (`create_resource` → `PUT …/data`) and the
    /// server holds them, so they must not bloat the relay journal.
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

fn is_note_change(change: &Change) -> bool {
    matches!(
        change,
        Change::NoteCreate { .. } | Change::NoteUpdate { .. } | Change::NoteDelete { .. }
    )
}

/// Drop the inline binary from a `ResourceCreate` before it is relayed: with
/// keeplin-srv the blob is uploaded out-of-band and served from the server, so
/// carrying it in the change would duplicate it and bloat the journal.
fn strip_resource_blob(change: Change) -> Change {
    match change {
        Change::ResourceCreate { resource, .. } => Change::ResourceCreate {
            resource,
            data: None,
        },
        other => other,
    }
}

// ── Connection task ──────────────────────────────────────────────────────────

/// Server note representation from `GET /api/notes`.
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

/// Maintain the WebSocket connection forever: discover notes, join them, and
/// pump ops in both directions. Reconnects with a capped backoff; every
/// reconnect re-discovers and re-joins, so state is rebuilt from snapshots.
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

/// How often a live connection re-runs note discovery, so notes created on
/// other devices — or newly shared with this user — get joined without a
/// reconnect.
const REDISCOVER_EVERY: Duration = Duration::from_secs(15);

async fn connect_once(
    shared: &Arc<Shared>,
    out: &mut mpsc::UnboundedReceiver<CollabClientMsg>,
) -> anyhow::Result<()> {
    // The token travels in the Authorization header, not the URL: query
    // strings end up in proxy and access logs.
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
    rediscover.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            queued = out.recv() => {
                let Some(msg) = queued else { return Ok(()) };
                if let CollabClientMsg::Join { note_id } = &msg {
                    joined.insert(*note_id);
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

/// Discover own + shared notes and join the ones this connection has not
/// joined yet. Unknown notes are created locally (empty body — the Welcome
/// snapshot fills it in).
async fn discover_and_join(
    shared: &Arc<Shared>,
    ws: &mut WsStream,
    joined: &mut HashSet<Uuid>,
) -> anyhow::Result<()> {
    let listing: Vec<ServerNote> = shared
        .auth(shared.http.get(format!("{}/api/notes", shared.cfg.api_url)))
        .send()
        .await?
        .json()
        .await?;
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
    Ok(())
}

/// The client-side WebSocket stream type (plain TCP or TLS).
type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Make sure a note discovered on the server exists locally.
async fn ensure_local(shared: &Arc<Shared>, server_note: &ServerNote) {
    let Some(top) = shared.top.get() else { return };
    if top.read_note(server_note.id).await.is_ok() {
        return;
    }
    let note = Note {
        id: server_note.id,
        title: server_note.title.clone(),
        body: String::new(),
        // The server may have no notebook for the note; locally the nil UUID
        // is the Inbox system notebook.
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

async fn handle_server_msg(shared: &Arc<Shared>, msg: CollabServerMsg) {
    match msg {
        CollabServerMsg::Welcome { note_id, snapshot } => {
            let mut lines = NoteLines::from_snapshot(snapshot);
            if shared.pending_push.lock().await.remove(&note_id) {
                // This note has local content the server has not seen yet (it
                // was just created, or edited before this first Welcome). Rather
                // than overwrite it with the snapshot, diff the local body
                // against the snapshot and push the difference — so the content
                // is preserved and correctly transformed into ops against the
                // server's actual state.
                let local_body = match shared.top.get() {
                    Some(top) => top
                        .read_note(note_id)
                        .await
                        .map(|n| n.body)
                        .unwrap_or_default(),
                    None => String::new(),
                };
                let ops = lines.diff_body(&local_body, &shared.device_id);
                shared.notes.lock().await.insert(note_id, lines);
                if !ops.is_empty() {
                    let _ = shared.out.send(CollabClientMsg::Op { note_id, ops });
                }
                // The local body already equals `local_body`; nothing to write.
            } else {
                // A note we only cache: take the server's body as-is.
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
        CollabServerMsg::Error { code, message } => {
            tracing::warn!(code, message, "collab: server error");
        }
        CollabServerMsg::Presence { note_id, users } => {
            shared.presence.lock().await.insert(note_id, users);
        }
    }
}

/// Persist a server-derived body locally (suppressed), keeping metadata.
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
