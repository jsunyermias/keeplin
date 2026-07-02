//! REST/JSON API served by [axum] on a separate HTTP port.
//!
//! This module exposes the same operations as the gRPC service over plain HTTP with JSON
//! bodies, serialised straight from the `keeplin-core` domain models (no protobuf). The
//! state holds the backend as a trait object (`Arc<dyn StorageBackend>`) so handlers are
//! not generic over the concrete backend type; the gRPC server shares the same backend
//! instance. Authentication reuses the shared constant-time Basic-Auth check in
//! [`crate::auth`]. `GET /api/ws` upgrades to a WebSocket that streams every [`Change`]
//! published by the daemon's `EventBackend`, and `POST /api/sync` runs one sync cycle.
//!
//! The HTTP listener is plain HTTP — terminate TLS at a reverse proxy in production, as
//! noted in `SECURITY.md`.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, Request, State,
    },
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE},
        HeaderMap, StatusCode,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use keeplin_core::{
    error::{StorageError, SyncError},
    linking,
    links::{parse_link_ref, NoteLink},
    models::{now, Change, Note, NoteTag, Notebook, Resource, Tag},
    storage::StorageBackend,
    sync::run_sync,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Shared state for every HTTP handler.
pub struct AppState {
    /// The storage backend, shared (as a trait object) with the gRPC server.
    pub backend: Arc<dyn StorageBackend>,
    /// Sender for the live change feed. Each WebSocket connection subscribes to a fresh
    /// receiver; mutations published here by the daemon's `EventBackend` are streamed out.
    pub events: broadcast::Sender<Change>,
    /// Maximum request body size in bytes. Mirrors the gRPC `max_message_size` so a large
    /// resource upload (`POST /api/resources`) is not silently capped at axum's 2 MiB default.
    pub max_body_bytes: usize,
    /// How many days of change-journal history to retain; `POST /api/sync` prunes older
    /// entries after a successful cycle, exactly like the gRPC `Sync` RPC (both call
    /// [`crate::server::prune_journal_after_sync`]). `0` disables pruning.
    pub journal_retention_days: u64,
    /// Basic-Auth credentials; when both are `Some`, every request must authenticate.
    pub auth_username: Option<String>,
    pub auth_password: Option<String>,
}

/// Handler-facing shared state. `Arc` makes it cheaply cloneable for axum's `State`.
pub type Shared = Arc<AppState>;

/// Build the `/api` router (REST endpoints) with the auth middleware applied.
pub fn router(state: Shared) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/notes", get(list_notes).post(create_note))
        .route(
            "/notes/:id",
            get(get_note).put(update_note).delete(delete_note),
        )
        .route("/notes/:id/tags", get(list_note_tags))
        .route(
            "/notes/:note_id/tags/:tag_id",
            put(add_note_tag).delete(remove_note_tag),
        )
        .route("/notes/:id/alias", put(set_note_alias))
        .route("/notes/:id/links", get(list_links).post(add_link))
        .route(
            "/notes/:id/links/:index",
            axum::routing::delete(remove_link),
        )
        .route("/notes/:id/backlinks", get(list_backlinks))
        .route("/links/resolve", get(resolve_reference))
        .route("/aliases/conflicts", get(list_alias_conflicts))
        .route("/notebooks", get(list_notebooks).post(create_notebook))
        .route(
            "/notebooks/:id",
            get(get_notebook)
                .put(update_notebook)
                .delete(delete_notebook),
        )
        .route("/notebooks/:id/alias", put(set_notebook_alias))
        .route("/tags", get(list_tags).post(create_tag))
        .route("/tags/:id", get(get_tag).put(update_tag).delete(delete_tag))
        .route("/resources", get(list_resources).post(create_resource))
        .route("/resources/:id", get(get_resource).delete(delete_resource))
        .route("/resources/:id/data", get(get_resource_data))
        .route("/sync", post(sync))
        .route("/ws", get(ws_handler))
        // Raise the request-body cap from axum's 2 MiB default to the configured size so REST
        // resource uploads match what gRPC accepts.
        .layer(axum::extract::DefaultBodyLimit::max(state.max_body_bytes))
        .layer(middleware::from_fn_with_state(state.clone(), auth_mw))
        .with_state(state);
    Router::new().nest("/api", api)
}

// ── Auth middleware ─────────────────────────────────────────────────────────────

/// Reject requests that fail Basic Auth when credentials are configured; a no-op
/// otherwise (mirrors the gRPC interceptor).
async fn auth_mw(State(state): State<Shared>, req: Request, next: Next) -> Response {
    if let (Some(user), Some(pass)) = (&state.auth_username, &state.auth_password) {
        let header = req
            .headers()
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        if !crate::auth::verify_basic(header, user, pass) {
            return (
                StatusCode::UNAUTHORIZED,
                [(WWW_AUTHENTICATE, "Basic")],
                Json(json!({ "error": "invalid credentials" })),
            )
                .into_response();
        }
    }
    next.run(req).await
}

// ── Error mapping ───────────────────────────────────────────────────────────────

/// Wraps a [`StorageError`] so it can be returned from a handler as an HTTP response.
struct ApiError(StorageError);

impl From<StorageError> for ApiError {
    fn from(e: StorageError) -> Self {
        ApiError(e)
    }
}

impl From<SyncError> for ApiError {
    fn from(e: SyncError) -> Self {
        match e {
            // A storage failure during sync keeps its precise mapping (e.g. NotFound → 404).
            SyncError::Storage(s) => ApiError(s),
            // Conflict/Failed are transport- or protocol-level sync failures; surface them
            // as a 500 with the underlying message rather than inventing a finer status.
            other => ApiError(StorageError::InvalidState(other.to_string())),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let code = match &self.0 {
            StorageError::NotFound(_) => StatusCode::NOT_FOUND,
            StorageError::CorruptedData(_) => StatusCode::UNPROCESSABLE_ENTITY,
            // A duplicate alias (uniqueness violation) is a client conflict, not a server bug.
            StorageError::Conflict(_) => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (code, Json(json!({ "error": self.0.to_string() }))).into_response()
    }
}

// ── Shared request/response shapes ──────────────────────────────────────────────

/// `?page_size=&page_token=` for every list endpoint.
#[derive(Debug, Deserialize)]
struct Pagination {
    #[serde(default)]
    page_size: u32,
    #[serde(default)]
    page_token: Option<String>,
}

/// A page of results: the items plus the opaque cursor for the next page.
#[derive(Debug, Serialize)]
struct Page<T> {
    items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_page_token: Option<String>,
}

fn page<T>((items, next): (Vec<T>, Option<String>)) -> Json<Page<T>> {
    Json(Page {
        items,
        next_page_token: next,
    })
}

// ── Health ──────────────────────────────────────────────────────────────────────

async fn health() -> &'static str {
    "ok"
}

// ── Notes ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateNote {
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    notebook_id: Option<Uuid>,
    #[serde(default)]
    is_todo: bool,
    #[serde(default)]
    todo_due: Option<DateTime<Utc>>,
}

async fn list_notes(
    State(s): State<Shared>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Note>>, ApiError> {
    Ok(page(s.backend.list_notes(p.page_size, p.page_token).await?))
}

async fn create_note(
    State(s): State<Shared>,
    Json(req): Json<CreateNote>,
) -> Result<Json<Note>, ApiError> {
    let mut note = Note::new(req.title, req.body);
    note.notebook_id = req.notebook_id;
    note.is_todo = req.is_todo;
    note.todo_due = req.todo_due;
    Ok(Json(s.backend.create_note(note).await?))
}

async fn get_note(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Note>, ApiError> {
    let note = s.backend.read_note(id).await?;
    // The backend retains soft-deleted entities as tombstones (for sync); the REST surface
    // presents a clean lifecycle, so a deleted note reads as 404.
    if note.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(Json(note))
}

async fn update_note(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(mut note): Json<Note>,
) -> Result<Json<Note>, ApiError> {
    // A tombstoned note reads as 404 on this surface, so updating one is a 404 too —
    // otherwise a PUT (whose body defaults `deleted_at` to null) would silently revive
    // it. Revival is reserved for the sync path (`apply_change`).
    read_live_note(&s, id).await?;
    note.id = id;
    note.updated_at = now();
    Ok(Json(s.backend.update_note(note).await?))
}

async fn delete_note(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.backend.delete_note(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_note_tags(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Tag>>, ApiError> {
    Ok(page(
        s.backend
            .list_note_tags(id, p.page_size, p.page_token)
            .await?,
    ))
}

async fn add_note_tag(
    State(s): State<Shared>,
    Path((note_id, tag_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    s.backend.add_note_tag(NoteTag { note_id, tag_id }).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_note_tag(
    State(s): State<Shared>,
    Path((note_id, tag_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    s.backend.remove_note_tag(note_id, tag_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Aliases & links ───────────────────────────────────────────────────────────────

/// `{ "alias": "…" | null }` body shared by the alias-setting endpoints.
#[derive(Debug, Deserialize)]
struct AliasBody {
    #[serde(default)]
    alias: Option<String>,
}

/// Read a live note or return 404 for a missing or soft-deleted one (mirrors `get_note`).
async fn read_live_note(s: &Shared, id: Uuid) -> Result<Note, ApiError> {
    let note = s.backend.read_note(id).await?;
    if note.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(note)
}

async fn set_note_alias(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<AliasBody>,
) -> Result<Json<Note>, ApiError> {
    Ok(Json(
        linking::set_note_alias(s.backend.as_ref(), id, b.alias).await?,
    ))
}

async fn list_links(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<NoteLink>>, ApiError> {
    Ok(Json(read_live_note(&s, id).await?.links))
}

/// `{ "raw": "#notebook1#note3#5" }` body for adding a manual (global) link.
#[derive(Debug, Deserialize)]
struct AddLinkBody {
    raw: String,
}

async fn add_link(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<AddLinkBody>,
) -> Result<Json<Note>, ApiError> {
    // Validate the reference syntax up front so a bad body is a 422, not a 500.
    if parse_link_ref(&b.raw).is_none() {
        return Err(
            StorageError::CorruptedData(format!("invalid link reference '{}'", b.raw)).into(),
        );
    }
    Ok(Json(
        linking::add_manual_link(s.backend.as_ref(), id, &b.raw).await?,
    ))
}

async fn remove_link(
    State(s): State<Shared>,
    Path((id, index)): Path<(Uuid, usize)>,
) -> Result<Json<Note>, ApiError> {
    Ok(Json(
        linking::remove_link(s.backend.as_ref(), id, index).await?,
    ))
}

async fn list_backlinks(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Note>>, ApiError> {
    Ok(page(
        linking::backlinks(s.backend.as_ref(), id, p.page_size, p.page_token).await?,
    ))
}

/// `?ref=#notebook1#note3#5` query for resolving a reference to a note (+ bookmark number).
#[derive(Debug, Deserialize)]
struct ResolveQuery {
    #[serde(rename = "ref")]
    reference: String,
}

async fn resolve_reference(
    State(s): State<Shared>,
    Query(q): Query<ResolveQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let resolved = linking::resolve(s.backend.as_ref(), &q.reference).await?;
    Ok(Json(match resolved {
        Some(r) => json!({
            "note_id": r.note_id,
            "bookmark_number": r.bookmark_number,
        }),
        None => json!({ "note_id": null, "bookmark_number": null }),
    }))
}

/// `GET /api/aliases/conflicts` — list note/notebook aliases shared by two or more live
/// entities (the residue of a cross-device alias collision), so a human can rename one side.
async fn list_alias_conflicts(
    State(s): State<Shared>,
) -> Result<Json<linking::AliasConflicts>, ApiError> {
    Ok(Json(linking::alias_conflicts(s.backend.as_ref()).await?))
}

// ── Notebooks ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TitleOnly {
    title: String,
}

async fn list_notebooks(
    State(s): State<Shared>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Notebook>>, ApiError> {
    Ok(page(
        s.backend.list_notebooks(p.page_size, p.page_token).await?,
    ))
}

async fn create_notebook(
    State(s): State<Shared>,
    Json(req): Json<TitleOnly>,
) -> Result<Json<Notebook>, ApiError> {
    Ok(Json(
        s.backend.create_notebook(Notebook::new(req.title)).await?,
    ))
}

/// Read a live notebook or return 404 for a missing or soft-deleted one.
async fn read_live_notebook(s: &Shared, id: Uuid) -> Result<Notebook, ApiError> {
    let nb = s.backend.read_notebook(id).await?;
    if nb.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(nb)
}

async fn get_notebook(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Json<Notebook>, ApiError> {
    Ok(Json(read_live_notebook(&s, id).await?))
}

async fn update_notebook(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(mut nb): Json<Notebook>,
) -> Result<Json<Notebook>, ApiError> {
    // Updating a tombstoned notebook is a 404, like reading one (see `update_note`).
    read_live_notebook(&s, id).await?;
    nb.id = id;
    nb.updated_at = now();
    Ok(Json(s.backend.update_notebook(nb).await?))
}

async fn delete_notebook(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.backend.delete_notebook(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_notebook_alias(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<AliasBody>,
) -> Result<Json<Notebook>, ApiError> {
    Ok(Json(
        linking::set_notebook_alias(s.backend.as_ref(), id, b.alias).await?,
    ))
}

// ── Tags ────────────────────────────────────────────────────────────────────────

async fn list_tags(
    State(s): State<Shared>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Tag>>, ApiError> {
    Ok(page(s.backend.list_tags(p.page_size, p.page_token).await?))
}

async fn create_tag(
    State(s): State<Shared>,
    Json(req): Json<TitleOnly>,
) -> Result<Json<Tag>, ApiError> {
    Ok(Json(s.backend.create_tag(Tag::new(req.title)).await?))
}

/// Read a live tag or return 404 for a missing or soft-deleted one.
async fn read_live_tag(s: &Shared, id: Uuid) -> Result<Tag, ApiError> {
    let tag = s.backend.read_tag(id).await?;
    if tag.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(tag)
}

async fn get_tag(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Tag>, ApiError> {
    Ok(Json(read_live_tag(&s, id).await?))
}

async fn update_tag(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(mut tag): Json<Tag>,
) -> Result<Json<Tag>, ApiError> {
    // Updating a tombstoned tag is a 404, like reading one (see `update_note`).
    read_live_tag(&s, id).await?;
    tag.id = id;
    tag.updated_at = now();
    Ok(Json(s.backend.update_tag(tag).await?))
}

async fn delete_tag(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    s.backend.delete_tag(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Resources ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ResourceMeta {
    #[serde(default)]
    title: String,
    #[serde(default)]
    file_name: String,
}

async fn list_resources(
    State(s): State<Shared>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Resource>>, ApiError> {
    Ok(page(
        s.backend.list_resources(p.page_size, p.page_token).await?,
    ))
}

/// Upload a resource: raw request body is the payload, `?title=&file_name=` carry the
/// metadata, and the `Content-Type` header is recorded as the MIME type.
async fn create_resource(
    State(s): State<Shared>,
    Query(meta): Query<ResourceMeta>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Resource>, ApiError> {
    let mime = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let data = body.to_vec();
    let resource = Resource::new(meta.title, mime, meta.file_name, data.len() as u64);
    Ok(Json(s.backend.create_resource(resource, data).await?))
}

async fn get_resource(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Json<Resource>, ApiError> {
    let (meta, _data) = s.backend.read_resource(id).await?;
    Ok(Json(meta))
}

/// Download the raw bytes of a resource, served with its stored MIME type.
async fn get_resource_data(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let (meta, data) = s.backend.read_resource(id).await?;
    Ok(([(CONTENT_TYPE, meta.mime_type)], data).into_response())
}

async fn delete_resource(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.backend.delete_resource(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Sync ────────────────────────────────────────────────────────────────────────

/// JSON summary returned by `POST /api/sync`: how many remote changes were applied.
#[derive(Debug, Serialize)]
struct SyncSummary {
    applied: usize,
}

/// Run one synchronisation cycle on the shared backend and report how many remote
/// changes were applied. Mirrors the gRPC `Sync` RPC, minus the streaming progress —
/// including the post-sync journal prune, so `journal_retention_days` is honoured no
/// matter which surface drives the sync.
///
/// The backend is passed as `&dyn StorageBackend`; `run_sync` accepts it because
/// `dyn StorageBackend` itself satisfies `StorageBackend` (see the `?Sized` blanket impl
/// in `keeplin-core`).
async fn sync(State(s): State<Shared>) -> Result<Json<SyncSummary>, ApiError> {
    let applied = run_sync(s.backend.as_ref(), |_stage, _count| {}).await?;
    crate::server::prune_journal_after_sync(s.backend.as_ref(), s.journal_retention_days).await;
    Ok(Json(SyncSummary {
        applied: applied.len(),
    }))
}

// ── WebSocket live-change feed ────────────────────────────────────────────────────

/// `GET /api/ws` — upgrade to a WebSocket and stream every [`Change`] as a JSON text
/// frame. The upgrade request passes through the same Basic-Auth middleware as the REST
/// routes. Each connection gets its own broadcast receiver created at upgrade time, so it
/// sees changes from the moment it connects onward.
async fn ws_handler(State(s): State<Shared>, ws: WebSocketUpgrade) -> Response {
    let rx = s.events.subscribe();
    ws.on_upgrade(move |socket| stream_changes(socket, rx))
}

/// Forward broadcast changes to one connected client until it disconnects or the channel
/// closes. Serialises each [`Change`] to JSON; on `Lagged` (the client fell behind the
/// channel capacity) it sends a `{"type":"resync"}` hint so the client can reload state
/// rather than silently miss events.
async fn stream_changes(mut socket: WebSocket, mut rx: broadcast::Receiver<Change>) {
    loop {
        tokio::select! {
            received = rx.recv() => match received {
                Ok(change) => {
                    let text = serde_json::to_string(&change)
                        .unwrap_or_else(|_| r#"{"type":"error"}"#.to_string());
                    if socket.send(Message::Text(text)).await.is_err() {
                        break; // client went away
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let hint = Message::Text(r#"{"type":"resync"}"#.to_string());
                    if socket.send(hint).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            // Drive the receive side so client pings get pongs and a close frame ends the
            // loop promptly instead of waiting for the next failed send.
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(_)) => break,
                Some(Ok(_)) => {} // ignore data/ping/pong frames from the client
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use keeplin_core::storage::fs::FsBackend;
    use tower::ServiceExt;

    /// Build an `AppState` over a fresh `FsBackend` in a leaked temp dir (kept alive for
    /// the test), optionally with Basic-Auth credentials configured.
    async fn state(auth: Option<(&str, &str)>) -> Shared {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let fs = FsBackend::new(&path).await.unwrap();
        let (events, _rx) = broadcast::channel(16);
        Arc::new(AppState {
            backend: Arc::new(fs),
            events,
            max_body_bytes: 32 * 1024 * 1024,
            journal_retention_days: 30,
            auth_username: auth.map(|a| a.0.to_string()),
            auth_password: auth.map(|a| a.1.to_string()),
        })
    }

    /// Like [`state`] but wraps the backend in `LinkingBackend`, so writes derive bookmarks
    /// and links and resolve references — required by the bookmark/link endpoint tests.
    async fn linking_state() -> Shared {
        use keeplin_core::linking::LinkingBackend;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let fs = FsBackend::new(&path).await.unwrap();
        let (events, _rx) = broadcast::channel(16);
        Arc::new(AppState {
            backend: Arc::new(LinkingBackend::new(fs)),
            events,
            max_body_bytes: 32 * 1024 * 1024,
            journal_retention_days: 30,
            auth_username: None,
            auth_password: None,
        })
    }

    /// Issue one request against a fresh router over the shared state and return
    /// `(status, body bytes)`.
    async fn call(
        st: &Shared,
        method: &str,
        uri: &str,
        body: Option<&str>,
        auth_header: Option<&str>,
    ) -> (StatusCode, Vec<u8>) {
        let mut b = Request::builder().method(method).uri(uri);
        if body.is_some() {
            b = b.header(CONTENT_TYPE, "application/json");
        }
        if let Some(a) = auth_header {
            b = b.header(AUTHORIZATION, a);
        }
        let req = b
            .body(
                body.map(|s| Body::from(s.to_owned()))
                    .unwrap_or(Body::empty()),
            )
            .unwrap();
        let resp = router(st.clone()).oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        (status, bytes)
    }

    #[tokio::test]
    async fn note_crud_round_trip() {
        let st = state(None).await;

        // Create.
        let (code, body) = call(
            &st,
            "POST",
            "/api/notes",
            Some(r#"{"title":"T","body":"B"}"#),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let note: Note = serde_json::from_slice(&body).unwrap();
        assert_eq!(note.title, "T");
        let id = note.id;

        // Read.
        let (code, body) = call(&st, "GET", &format!("/api/notes/{id}"), None, None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(serde_json::from_slice::<Note>(&body).unwrap().body, "B");

        // List.
        let (code, body) = call(&st, "GET", "/api/notes", None, None).await;
        assert_eq!(code, StatusCode::OK);
        let pagev: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(pagev["items"].as_array().unwrap().len(), 1);

        // Delete, then 404.
        let (code, _) = call(&st, "DELETE", &format!("/api/notes/{id}"), None, None).await;
        assert_eq!(code, StatusCode::NO_CONTENT);
        let (code, _) = call(&st, "GET", &format!("/api/notes/{id}"), None, None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn updates_on_deleted_entities_are_404() {
        let st = state(None).await;

        // Note: create → delete → PUT must be a 404, not a silent revival.
        let (_, body) = call(
            &st,
            "POST",
            "/api/notes",
            Some(r#"{"title":"T","body":"B"}"#),
            None,
        )
        .await;
        let note: Note = serde_json::from_slice(&body).unwrap();
        call(
            &st,
            "DELETE",
            &format!("/api/notes/{}", note.id),
            None,
            None,
        )
        .await;

        let update = serde_json::to_string(&note).unwrap(); // deleted_at: null
        let (code, _) = call(
            &st,
            "PUT",
            &format!("/api/notes/{}", note.id),
            Some(&update),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::NOT_FOUND, "PUT on deleted note");
        let (code, _) = call(
            &st,
            "PUT",
            &format!("/api/notes/{}/alias", note.id),
            Some(r#"{"alias":"ghost"}"#),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::NOT_FOUND, "alias PUT on deleted note");
        // Still deleted afterwards.
        let (code, _) = call(&st, "GET", &format!("/api/notes/{}", note.id), None, None).await;
        assert_eq!(code, StatusCode::NOT_FOUND, "note must remain deleted");

        // Notebook.
        let (_, body) = call(
            &st,
            "POST",
            "/api/notebooks",
            Some(r#"{"title":"NB"}"#),
            None,
        )
        .await;
        let nb: Notebook = serde_json::from_slice(&body).unwrap();
        call(
            &st,
            "DELETE",
            &format!("/api/notebooks/{}", nb.id),
            None,
            None,
        )
        .await;
        let (code, _) = call(
            &st,
            "PUT",
            &format!("/api/notebooks/{}", nb.id),
            Some(&serde_json::to_string(&nb).unwrap()),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::NOT_FOUND, "PUT on deleted notebook");

        // Tag.
        let (_, body) = call(&st, "POST", "/api/tags", Some(r#"{"title":"t"}"#), None).await;
        let tag: Tag = serde_json::from_slice(&body).unwrap();
        call(&st, "DELETE", &format!("/api/tags/{}", tag.id), None, None).await;
        let (code, _) = call(
            &st,
            "PUT",
            &format!("/api/tags/{}", tag.id),
            Some(&serde_json::to_string(&tag).unwrap()),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::NOT_FOUND, "PUT on deleted tag");
    }

    #[tokio::test]
    async fn sync_endpoint_prunes_journal_within_retention() {
        // A DbBackend state (empty server_url → local-only sync) exercises the pruning
        // path that FsBackend no-ops: after POST /api/sync, fresh journal rows must
        // survive a 30-day retention window (the prune ran, and respected the window).
        use keeplin_core::storage::db::DbBackend;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rest.db");
        std::mem::forget(dir);
        let db = DbBackend::new(path, "", "").await.unwrap();
        let (events, _rx) = broadcast::channel(16);
        let st: Shared = Arc::new(AppState {
            backend: Arc::new(db),
            events,
            max_body_bytes: 32 * 1024 * 1024,
            journal_retention_days: 30,
            auth_username: None,
            auth_password: None,
        });

        let (code, _) = call(
            &st,
            "POST",
            "/api/notes",
            Some(r#"{"title":"kept","body":""}"#),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);

        let (code, body) = call(&st, "POST", "/api/sync", None, None).await;
        assert_eq!(code, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["applied"], 0, "no relay → nothing applied");

        // The fresh create is younger than the retention cutoff, so it must survive.
        let epoch = chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap();
        let journal = st.backend.get_changes_since(epoch).await.unwrap();
        assert_eq!(journal.len(), 1, "recent journal rows survive the prune");
    }

    #[tokio::test]
    async fn invalid_uuid_is_bad_request() {
        let st = state(None).await;
        let (code, _) = call(&st, "GET", "/api/notes/not-a-uuid", None, None).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn auth_is_enforced_when_configured() {
        let st = state(Some(("alice", "s3cr3t"))).await;
        let good = format!("Basic {}", STANDARD.encode("alice:s3cr3t"));
        let bad = format!("Basic {}", STANDARD.encode("alice:wrong"));

        let (code, _) = call(&st, "GET", "/api/notes", None, None).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED, "missing credentials → 401");

        let (code, _) = call(&st, "GET", "/api/notes", None, Some(&bad)).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED, "wrong credentials → 401");

        let (code, _) = call(&st, "GET", "/api/notes", None, Some(&good)).await;
        assert_eq!(code, StatusCode::OK, "valid credentials → 200");
    }

    #[tokio::test]
    async fn resource_upload_and_download() {
        let st = state(None).await;
        let (code, body) = call(
            &st,
            "POST",
            "/api/resources?title=pic&file_name=p.png",
            Some("not really json but raw bytes"),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let res: Resource = serde_json::from_slice(&body).unwrap();
        assert_eq!(res.title, "pic");
        assert_eq!(res.file_name, "p.png");

        let (code, data) = call(
            &st,
            "GET",
            &format!("/api/resources/{}/data", res.id),
            None,
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(data, b"not really json but raw bytes");
    }

    #[tokio::test]
    async fn resource_upload_above_axum_default_limit() {
        // A 3 MiB body exceeds axum's 2 MiB default; it must succeed because the router
        // raises the limit to `max_body_bytes` (32 MiB in the test state).
        let st = state(None).await;
        let big = "x".repeat(3 * 1024 * 1024);
        let (code, body) = call(
            &st,
            "POST",
            "/api/resources?title=big&file_name=big.bin",
            Some(&big),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let res: Resource = serde_json::from_slice(&body).unwrap();
        assert_eq!(res.size, big.len() as u64);
    }

    #[tokio::test]
    async fn alias_and_links_endpoints() {
        let st = linking_state().await;

        // Create a note whose body declares a bookmark (with an inline alias) and a link.
        let (code, body) = call(
            &st,
            "POST",
            "/api/notes",
            Some(r#"{"title":"T","body":"intro [Anchor1](### \"Custom\") and [l](#other)"}"#),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let note: Note = serde_json::from_slice(&body).unwrap();
        let id = note.id;

        // The bookmark was derived from the body and is returned inline on the note
        // (there is no dedicated bookmark endpoint — the body is the source of truth).
        assert_eq!(note.bookmarks.len(), 1);
        assert_eq!(note.bookmarks[0].number, 1);
        assert_eq!(note.bookmarks[0].text, "Anchor1");
        assert_eq!(note.bookmarks[0].alias, "Custom");

        // Content link present; add a manual link and then remove it.
        let (code, body) = call(&st, "GET", &format!("/api/notes/{id}/links"), None, None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<Vec<NoteLink>>(&body)
                .unwrap()
                .len(),
            1
        );

        let (code, body) = call(
            &st,
            "POST",
            &format!("/api/notes/{id}/links"),
            Some(r##"{"raw":"#manualtarget"}"##),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<Note>(&body).unwrap().links.len(),
            2
        );

        // A malformed link reference is rejected (422).
        let (code, _) = call(
            &st,
            "POST",
            &format!("/api/notes/{id}/links"),
            Some(r#"{"raw":"not-a-ref"}"#),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn alias_backlinks_and_resolve_endpoints() {
        let st = linking_state().await;

        // Target note, then give it an alias via the alias endpoint.
        let (_, body) = call(
            &st,
            "POST",
            "/api/notes",
            Some("{\"title\":\"target\",\"body\":\"[Anchor](###) here\"}"),
            None,
        )
        .await;
        let target: Note = serde_json::from_slice(&body).unwrap();
        let (code, body) = call(
            &st,
            "PUT",
            &format!("/api/notes/{}/alias", target.id),
            Some(r#"{"alias":"note3"}"#),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<Note>(&body)
                .unwrap()
                .alias
                .as_deref(),
            Some("note3")
        );

        // Source note links to the target by alias.
        let (_, body) = call(
            &st,
            "POST",
            "/api/notes",
            Some(r#"{"title":"src","body":"see [x](#note3)"}"#),
            None,
        )
        .await;
        let src: Note = serde_json::from_slice(&body).unwrap();

        // Backlinks of the target include the source.
        let (code, body) = call(
            &st,
            "GET",
            &format!("/api/notes/{}/backlinks", target.id),
            None,
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let backv: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let back = backv["items"].as_array().unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0]["id"], serde_json::json!(src.id.to_string()));

        // Resolve a 3-segment reference to the target note + bookmark number 1.
        let (code, body) = call(
            &st,
            "GET",
            &format!("/api/links/resolve?ref=%23nb%23{}%23Anchor", target.id),
            None,
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["note_id"], serde_json::json!(target.id.to_string()));
        assert_eq!(v["bookmark_number"], serde_json::json!(1));
    }

    #[tokio::test]
    async fn alias_conflicts_endpoint() {
        // A plain FsBackend state (no LinkingBackend) has no write-time uniqueness check, so
        // the same alias can be planted on two notes — the way a cross-device sync collision
        // would appear.
        let st = state(None).await;
        let (_, b1) = call(
            &st,
            "POST",
            "/api/notes",
            Some(r#"{"title":"a","body":""}"#),
            None,
        )
        .await;
        let (_, b2) = call(
            &st,
            "POST",
            "/api/notes",
            Some(r#"{"title":"b","body":""}"#),
            None,
        )
        .await;
        let n1: Note = serde_json::from_slice(&b1).unwrap();
        let n2: Note = serde_json::from_slice(&b2).unwrap();

        for id in [n1.id, n2.id] {
            let (code, _) = call(
                &st,
                "PUT",
                &format!("/api/notes/{id}/alias"),
                Some(r#"{"alias":"dup"}"#),
                None,
            )
            .await;
            assert_eq!(code, StatusCode::OK);
        }

        let (code, body) = call(&st, "GET", "/api/aliases/conflicts", None, None).await;
        assert_eq!(code, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["notes"].as_array().unwrap().len(), 1);
        assert_eq!(v["notes"][0]["alias"], "dup");
        assert_eq!(v["notes"][0]["entities"].as_array().unwrap().len(), 2);
        assert!(v["notebooks"].as_array().unwrap().is_empty());
    }

    // ── WebSocket feed (real socket, end to end) ─────────────────────────────────

    use crate::event_backend::EventBackend;
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    /// Build an `AppState` whose backend is an `EventBackend` over a fresh `FsBackend`, so
    /// mutations made through the router publish to the same `events` channel the WebSocket
    /// route subscribes to. Returns the state and a clone of the sender.
    async fn state_with_events() -> Shared {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let fs = FsBackend::new(&path).await.unwrap();
        let (events, _rx) = broadcast::channel(16);
        let backend = Arc::new(EventBackend::new(fs, events.clone()));
        Arc::new(AppState {
            backend,
            events,
            max_body_bytes: 32 * 1024 * 1024,
            journal_retention_days: 30,
            auth_username: None,
            auth_password: None,
        })
    }

    #[tokio::test]
    async fn websocket_streams_note_create() {
        let st = state_with_events().await;

        // Serve the real router on an ephemeral port.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = router(st.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Connect a WebSocket client. The handler subscribes synchronously before the
        // upgrade response is sent, so no event created after this point can be missed.
        let (mut ws, _resp) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws"))
            .await
            .expect("ws connect");

        // Create a note through the same shared backend (in-process is fine; it still flows
        // through the EventBackend and publishes to the broadcast channel).
        let (code, _) = call(
            &st,
            "POST",
            "/api/notes",
            Some(r#"{"title":"hello","body":"world"}"#),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);

        // The client should receive a NoteCreate frame whose note matches.
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("timed out waiting for change frame")
            .expect("stream ended")
            .expect("ws error");
        let text = match frame {
            WsMessage::Text(t) => t,
            other => panic!("expected text frame, got {other:?}"),
        };
        let change: Change = serde_json::from_str(&text).unwrap();
        match change {
            Change::NoteCreate { note } => {
                assert_eq!(note.title, "hello");
                assert_eq!(note.body, "world");
            }
            other => panic!("expected NoteCreate, got {other:?}"),
        }
    }
}
