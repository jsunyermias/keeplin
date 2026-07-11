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
//! Three **operational** endpoints — `GET /api/health` (liveness), `/api/ready` (readiness),
//! and `/api/metrics` (Prometheus, see [`crate::metrics`]) — sit outside the auth middleware
//! and the HTTP-status counter so orchestrator probes and metric scrapers work without
//! credentials and do not inflate the request metrics.
//!
//! The HTTP listener is plain HTTP — terminate TLS at a reverse proxy in production, as
//! noted in `SECURITY.md`.

use std::sync::Arc;

use axum::{
    body::{Body, Bytes},
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        DefaultBodyLimit, Path, Query, Request, State,
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
    history,
    interop::{self, CalendarEvent, Contact},
    linking,
    links::{parse_link_ref, NoteLink},
    models::{now, Change, Note, NoteTag, Notebook, Resource, Tag},
    ordering,
    storage::{EntityVersion, StorageBackend},
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
    /// Presence/cursor view of the collaborative session (server mode with
    /// `collab_api_url` set); `None` when collaboration is disabled.
    pub collab: Option<keeplin_core::collab::CollabHandle>,
    /// Full-text search index query view; `None` when the index could not be
    /// created (search then responds `503`).
    pub search: Option<crate::search::SearchHandle>,
    /// Sender for the live change feed. Each WebSocket connection subscribes to a fresh
    /// receiver; mutations published here by the daemon's `EventBackend` are streamed out.
    pub events: broadcast::Sender<Change>,
    /// Operational counters, shared with the outermost `MetricsBackend` decorator so
    /// `GET /api/metrics` exports the same registry the storage layer writes to.
    pub metrics: Arc<crate::metrics::Metrics>,
    /// Maximum request body size in bytes. Mirrors the gRPC `max_message_size` so a large
    /// resource upload (`POST /api/resources`) is not silently capped at axum's 2 MiB default.
    pub max_body_bytes: usize,
    /// Maximum assembled size, in bytes, of a **streamed** upload (`POST /api/resources/upload`).
    /// That route bypasses `max_body_bytes` and streams the body incrementally up to this cap,
    /// so attachments larger than `max_message_size` can be uploaded. `0` means no limit.
    pub max_upload_bytes: usize,
    /// How many days of change-journal history to retain; `POST /api/sync` prunes older
    /// entries after a successful cycle, exactly like the gRPC `Sync` RPC (both call
    /// [`crate::server::prune_journal_after_sync`]). `0` disables pruning.
    pub journal_retention_days: u64,
    /// After each successful sync, reclaim payloads of resources tombstoned longer than
    /// this many days ago (`0` disables; see `Config::resource_purge_days`).
    pub resource_purge_days: u64,
    /// Basic-Auth credentials; when both are `Some`, every request must authenticate.
    pub auth_username: Option<String>,
    pub auth_password: Option<String>,
}

/// Handler-facing shared state. `Arc` makes it cheaply cloneable for axum's `State`.
pub type Shared = Arc<AppState>;

/// Build the `/api` router: unauthenticated operational probes plus the auth-gated data API.
pub fn router(state: Shared) -> Router {
    // Operational endpoints carry no user data and must be reachable by liveness/readiness
    // probes and metrics scrapers that cannot present Basic-Auth credentials, so they sit
    // **outside** the auth middleware — and outside the HTTP-status counter, so frequent
    // probe/scrape traffic does not drown out the request metrics that matter.
    let ops = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/metrics", get(metrics))
        .with_state(state.clone());

    let api = Router::new()
        .route("/notes", get(list_notes).post(create_note))
        .route(
            "/notes/:id",
            get(get_note).put(update_note).delete(delete_note),
        )
        .route("/notes/:id/presence", get(note_presence))
        .route("/notes/:id/cursor", put(set_cursor))
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
        .route("/notes/:id/history", get(note_history))
        .route("/notes/:id/revert", post(revert_note_ep))
        // Permission management proxies to keeplin-srv (server mode only).
        .route(
            "/notes/:id/share",
            post(proxy_note_share).get(proxy_note_shares),
        )
        .route(
            "/notes/:id/share/:user_id",
            axum::routing::delete(proxy_note_unshare),
        )
        .route("/notes/:id/transfer", post(proxy_note_transfer))
        .route("/notes/starred", get(list_starred_notes))
        .route("/search", get(search_notes))
        .route("/notes/:id/pin", post(pin_note).delete(unpin_note))
        .route("/notes/:id/star", post(star_note).delete(unstar_note))
        .route("/notes/:id/sort-key", put(reorder_note))
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
        .route(
            "/notebooks/:id/share",
            post(proxy_notebook_share).get(proxy_notebook_shares),
        )
        .route(
            "/notebooks/:id/share/:user_id",
            axum::routing::delete(proxy_notebook_unshare),
        )
        .route("/notebooks/:id/transfer", post(proxy_notebook_transfer))
        .route("/notebooks/:id/notes", get(list_notes_in_notebook))
        .route("/notebooks/:id/history", get(notebook_history))
        .route("/notebooks/:id/revert", post(revert_notebook_ep))
        .route(
            "/notebooks/:id/notes/revert",
            post(revert_notebook_notes_ep),
        )
        .route("/history/revert", post(batch_revert_notes_ep))
        .route("/tags", get(list_tags).post(create_tag))
        .route("/tags/:id", get(get_tag).put(update_tag).delete(delete_tag))
        // Standards-format interop (vCard / iCalendar) — see keeplin-core `interop`.
        .route("/contacts", get(list_contacts_ep))
        .route("/contacts/import", post(import_contact_ep))
        .route("/contacts/:uid", axum::routing::delete(delete_contact_ep))
        .route("/contacts/:uid/export", get(export_contact_ep))
        .route("/events", get(list_events_ep))
        .route("/events/import", post(import_event_ep))
        .route("/events/:uid", axum::routing::delete(delete_event_ep))
        .route("/events/:uid/export", get(export_event_ep))
        .route("/todos/import", post(import_todo_ep))
        .route("/profile/vcard", get(profile_vcard_ep))
        .route("/resources", get(list_resources).post(create_resource))
        // Streaming upload for large attachments: the request body is read incrementally up to
        // `max_upload_bytes` instead of being capped at `max_body_bytes`, so this one route
        // disables the router-wide body limit and enforces its own larger cap.
        .route(
            "/resources/upload",
            post(upload_resource).layer(DefaultBodyLimit::disable()),
        )
        .route("/resources/:id", get(get_resource).delete(delete_resource))
        .route("/resources/:id/data", get(get_resource_data))
        .route("/sync", post(sync))
        .route("/ws", get(ws_handler))
        // Raise the request-body cap from axum's 2 MiB default to the configured size so REST
        // resource uploads match what gRPC accepts.
        .layer(axum::extract::DefaultBodyLimit::max(state.max_body_bytes))
        // Layers apply outermost-last: auth runs inside the status counter, so a rejected
        // request is still counted (as a 4xx) by `status_mw`.
        .layer(middleware::from_fn_with_state(state.clone(), auth_mw))
        .layer(middleware::from_fn_with_state(state.clone(), status_mw))
        .with_state(state);

    Router::new().nest("/api", ops.merge(api))
}

// ── Full-text search ─────────────────────────────────────────────────────────────

/// Query parameters for `GET /api/search`. All are optional; timestamps are
/// RFC3339, booleans `true`/`false`.
#[derive(Debug, Deserialize)]
struct SearchParams {
    /// Free text, matched against title, body, tag names and notebook name.
    q: Option<String>,
    notebook: Option<Uuid>,
    todo: Option<bool>,
    /// `true` = open to-dos only, `false` = completed only.
    open: Option<bool>,
    starred: Option<bool>,
    pinned: Option<bool>,
    due_after: Option<DateTime<Utc>>,
    due_before: Option<DateTime<Utc>>,
    updated_after: Option<DateTime<Utc>>,
    updated_before: Option<DateTime<Utc>>,
    limit: Option<usize>,
}

/// `GET /api/search` — full-text search over the daemon's index, returning the
/// matching notes (best match first). `503` when the index is unavailable.
async fn search_notes(State(s): State<Shared>, Query(p): Query<SearchParams>) -> Response {
    let Some(search) = &s.search else {
        return (StatusCode::SERVICE_UNAVAILABLE, "search unavailable").into_response();
    };
    let query = crate::search::SearchQuery {
        text: p.q.unwrap_or_default(),
        notebook_id: p.notebook,
        is_todo: p.todo,
        todo_open: p.open,
        is_starred: p.starred,
        is_pinned: p.pinned,
        due_after: p.due_after,
        due_before: p.due_before,
        updated_after: p.updated_after,
        updated_before: p.updated_before,
        limit: p.limit.unwrap_or(50),
    };
    let ids = match search.search(&query).await {
        Ok(ids) => ids,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    // Resolve ids to full notes through the backend (plaintext); skip any that
    // raced a deletion between the index query and the read.
    let mut notes = Vec::with_capacity(ids.len());
    for id in ids {
        if let Ok(note) = s.backend.read_note(id).await {
            notes.push(note);
        }
    }
    Json(notes).into_response()
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

/// Record every response's status class into the shared metrics registry
/// (`keeplin_http_requests_total`). Applied only to the data API, not the operational
/// probes, so scrape/probe traffic does not inflate the request counts.
async fn status_mw(State(state): State<Shared>, req: Request, next: Next) -> Response {
    let resp = next.run(req).await;
    state.metrics.record_http_status(resp.status().as_u16());
    resp
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
            // Domain-rule rejections (pin an Inbox note, out-of-band sort key, delete the
            // Inbox) are the caller's mistake.
            StorageError::InvalidInput(_) => StatusCode::BAD_REQUEST,
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

/// Liveness probe: the process is up and serving. Always `200 ok`; it does not touch the
/// backend, so it stays green even if storage is momentarily unavailable (that is what
/// `ready` is for).
async fn health() -> &'static str {
    "ok"
}

/// Readiness probe: can the daemon actually serve requests? Issues one cheap backend read
/// (`list_notes` with a page size of 1). `200 ready` when storage answers, `503` with the
/// error otherwise — so an orchestrator stops routing traffic to an instance whose database
/// is locked or unreachable. (This read flows through the metrics decorator, so a busy
/// readiness schedule contributes to the `note`/`list` counter.)
async fn ready(State(s): State<Shared>) -> Response {
    match s.backend.list_notes(1, None).await {
        Ok(_) => (StatusCode::OK, "ready").into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Prometheus metrics exposition (`text/plain; version=0.0.4`). Renders the shared registry
/// the `MetricsBackend` decorator and the HTTP middleware write to. Unauthenticated: the
/// counters carry only fixed-label aggregates, no user content.
async fn metrics(State(s): State<Shared>) -> Response {
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4")],
        s.metrics.render_prometheus(),
    )
        .into_response()
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
    note.notebook_id = req.notebook_id.unwrap_or_else(Uuid::nil);
    note.is_todo = req.is_todo;
    note.todo_due = req.todo_due;
    // Initial manual position: top of the Inbox, or the end of a normal notebook's
    // unpinned band.
    ordering::place_new_note(s.backend.as_ref(), &mut note).await?;
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
    let stored = read_live_note(&s, id).await?;
    note.id = id;
    // Moving the note to a different notebook re-places it (its old position and pinned
    // state belonged to the source notebook); a plain edit keeps its position.
    ordering::reconcile_notebook_move(s.backend.as_ref(), stored.notebook_id, &mut note).await?;
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

/// `GET /api/notebooks/:id/notes` — the notebook's notes in their manual order (pinned
/// band first). Use the nil UUID for the Inbox.
async fn list_notes_in_notebook(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Note>>, ApiError> {
    Ok(page(
        s.backend
            .list_notes_in_notebook(id, p.page_size, p.page_token)
            .await?,
    ))
}

/// `GET /api/notes/starred` — every live starred note, across all notebooks.
async fn list_starred_notes(
    State(s): State<Shared>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Note>>, ApiError> {
    Ok(page(
        s.backend
            .list_starred_notes(p.page_size, p.page_token)
            .await?,
    ))
}

/// `POST /api/notes/:id/pin` — move the note into its notebook's pinned band.
async fn pin_note(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Note>, ApiError> {
    Ok(Json(ordering::pin_note(s.backend.as_ref(), id).await?))
}

/// `DELETE /api/notes/:id/pin` — move the note back to the end of the normal band.
async fn unpin_note(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Note>, ApiError> {
    Ok(Json(ordering::unpin_note(s.backend.as_ref(), id).await?))
}

/// `POST /api/notes/:id/star` — set the global star (never moves the note).
async fn star_note(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Note>, ApiError> {
    Ok(Json(ordering::star_note(s.backend.as_ref(), id).await?))
}

/// `DELETE /api/notes/:id/star` — clear the global star.
async fn unstar_note(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Json<Note>, ApiError> {
    Ok(Json(ordering::unstar_note(s.backend.as_ref(), id).await?))
}

/// `{ "sort_key": … }` body for `PUT /api/notes/:id/sort-key`.
#[derive(Deserialize)]
struct ReorderBody {
    sort_key: u32,
}

/// `PUT /api/notes/:id/sort-key` — give the note a new manual position within its
/// current band (pinned `1..=999`, normal `>= 1000`, Inbox `>= 1`).
async fn reorder_note(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<ReorderBody>,
) -> Result<Json<Note>, ApiError> {
    Ok(Json(
        ordering::reorder_note(s.backend.as_ref(), id, b.sort_key).await?,
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

// ── History & revert ──────────────────────────────────────────────────────────────

/// `?limit=` for the history endpoints; `0` (absent) uses the backend's default cap.
#[derive(Debug, Default, Deserialize)]
struct HistoryQuery {
    #[serde(default)]
    limit: u32,
}

/// One past version of a note, as returned by `GET /notes/:id/history`. `note` is absent when
/// the version is a tombstone (the note was deleted at that point).
#[derive(Debug, Serialize)]
struct NoteVersion {
    timestamp: DateTime<Utc>,
    device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<Note>,
}

/// One past version of a notebook. See [`NoteVersion`].
#[derive(Debug, Serialize)]
struct NotebookVersion {
    timestamp: DateTime<Utc>,
    device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    notebook: Option<Notebook>,
}

/// `{ "at": "<RFC-3339>" }` — the instant to roll an entity back to (the newest version at or
/// before it).
#[derive(Debug, Deserialize)]
struct RevertBody {
    at: DateTime<Utc>,
}

/// `{ "at": …, "note_ids": [ … ] }` — batch forward-revert of the listed notes.
#[derive(Debug, Deserialize)]
struct BatchRevertBody {
    at: DateTime<Utc>,
    #[serde(default)]
    note_ids: Vec<Uuid>,
}

/// `GET /api/notes/:id/history` — a note's past versions, newest first.
async fn note_history(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<NoteVersion>>, ApiError> {
    let versions = s.backend.note_history(id, q.limit).await?;
    Ok(Json(versions.into_iter().map(note_version_dto).collect()))
}

/// `GET /api/notebooks/:id/history` — a notebook's past versions, newest first.
async fn notebook_history(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<NotebookVersion>>, ApiError> {
    let versions = s.backend.notebook_history(id, q.limit).await?;
    Ok(Json(
        versions.into_iter().map(notebook_version_dto).collect(),
    ))
}

/// `POST /api/notes/:id/revert` — forward-revert a note to its state as of `at`.
async fn revert_note_ep(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<RevertBody>,
) -> Result<Json<Note>, ApiError> {
    Ok(Json(
        history::revert_note(s.backend.as_ref(), id, b.at).await?,
    ))
}

/// `POST /api/notebooks/:id/revert` — forward-revert a notebook to its state as of `at`.
async fn revert_notebook_ep(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<RevertBody>,
) -> Result<Json<Notebook>, ApiError> {
    Ok(Json(
        history::revert_notebook(s.backend.as_ref(), id, b.at).await?,
    ))
}

/// `POST /api/notebooks/:id/notes/revert` — batch-revert every note currently in the notebook
/// to its state as of `at` (the roll-back companion to a destructive notebook-wide change).
async fn revert_notebook_notes_ep(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<RevertBody>,
) -> Result<Json<Vec<Note>>, ApiError> {
    Ok(Json(
        history::revert_notebook_notes_to(s.backend.as_ref(), id, b.at).await?,
    ))
}

/// `POST /api/history/revert` — batch forward-revert of an explicit note-id list to `at`.
async fn batch_revert_notes_ep(
    State(s): State<Shared>,
    Json(b): Json<BatchRevertBody>,
) -> Result<Json<Vec<Note>>, ApiError> {
    Ok(Json(
        history::revert_notes_to(s.backend.as_ref(), &b.note_ids, b.at).await?,
    ))
}

fn note_version_dto(v: EntityVersion<Note>) -> NoteVersion {
    NoteVersion {
        timestamp: v.timestamp,
        device_id: v.device_id,
        note: v.entity,
    }
}

fn notebook_version_dto(v: EntityVersion<Notebook>) -> NotebookVersion {
    NotebookVersion {
        timestamp: v.timestamp,
        device_id: v.device_id,
        notebook: v.entity,
    }
}

// ── Permission management (proxied to keeplin-srv) ──────────────────────────────
//
// Permissions are enforced server-side (the authority). The daemon does not store or enforce
// them; it forwards these requests to keeplin-srv over the collab channel's authenticated REST
// client and relays the response, so a frontend can view/manage shares on demand. In fs/offline
// mode there is no server, so these routes return `503`.

async fn proxy_perm(
    s: &Shared,
    method: &str,
    path: String,
    body: Option<serde_json::Value>,
) -> Response {
    let Some(collab) = &s.collab else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "permission management requires server mode" })),
        )
            .into_response();
    };
    match collab.proxy_request(method, &path, body).await {
        Ok((status, body)) => {
            let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
            (code, Json(body)).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn proxy_note_share(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    proxy_perm(&s, "POST", format!("/api/notes/{id}/share"), Some(body)).await
}

async fn proxy_note_shares(State(s): State<Shared>, Path(id): Path<Uuid>) -> Response {
    proxy_perm(&s, "GET", format!("/api/notes/{id}/share"), None).await
}

async fn proxy_note_unshare(
    State(s): State<Shared>,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> Response {
    proxy_perm(
        &s,
        "DELETE",
        format!("/api/notes/{id}/share/{user_id}"),
        None,
    )
    .await
}

async fn proxy_note_transfer(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    proxy_perm(&s, "POST", format!("/api/notes/{id}/transfer"), Some(body)).await
}

async fn proxy_notebook_share(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    proxy_perm(&s, "POST", format!("/api/notebooks/{id}/share"), Some(body)).await
}

async fn proxy_notebook_shares(State(s): State<Shared>, Path(id): Path<Uuid>) -> Response {
    proxy_perm(&s, "GET", format!("/api/notebooks/{id}/share"), None).await
}

async fn proxy_notebook_unshare(
    State(s): State<Shared>,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> Response {
    proxy_perm(
        &s,
        "DELETE",
        format!("/api/notebooks/{id}/share/{user_id}"),
        None,
    )
    .await
}

async fn proxy_notebook_transfer(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    proxy_perm(
        &s,
        "POST",
        format!("/api/notebooks/{id}/transfer"),
        Some(body),
    )
    .await
}

// ── Standards-format interop (vCard / iCalendar) ────────────────────────────────

/// A contact as JSON (a serialisable view of `keeplin_core::interop::Contact`).
#[derive(Debug, Serialize)]
struct ContactDto {
    uid: String,
    formatted_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    family_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    given_name: Option<String>,
    emails: Vec<String>,
    phones: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    org: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

impl From<Contact> for ContactDto {
    fn from(c: Contact) -> Self {
        Self {
            uid: c.uid,
            formatted_name: c.formatted_name,
            family_name: c.family_name,
            given_name: c.given_name,
            emails: c.emails,
            phones: c.phones,
            org: c.org,
            note: c.note,
        }
    }
}

/// A calendar event as JSON.
#[derive(Debug, Serialize)]
struct EventDto {
    uid: String,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

impl From<CalendarEvent> for EventDto {
    fn from(e: CalendarEvent) -> Self {
        Self {
            uid: e.uid,
            summary: e.summary,
            start: e.start,
            end: e.end,
            location: e.location,
            description: e.description,
        }
    }
}

/// Build a raw `text/vcard` or `text/calendar` body response.
fn text_body(mime: &'static str, body: String) -> Response {
    ([(CONTENT_TYPE, mime)], body).into_response()
}

async fn list_contacts_ep(State(s): State<Shared>) -> Result<Json<Vec<ContactDto>>, ApiError> {
    let contacts = interop::list_contacts(s.backend.as_ref()).await?;
    Ok(Json(contacts.into_iter().map(ContactDto::from).collect()))
}

/// `POST /api/contacts/import` — body is a vCard; parse it, store it, return the stored contact.
async fn import_contact_ep(
    State(s): State<Shared>,
    body: String,
) -> Result<Json<ContactDto>, ApiError> {
    let contact = Contact::from_vcard(&body)
        .ok_or_else(|| StorageError::InvalidInput("invalid vCard".into()))?;
    let saved = interop::save_contact(s.backend.as_ref(), contact).await?;
    Ok(Json(saved.into()))
}

/// `GET /api/contacts/:uid/export` — the contact as a `text/vcard` body.
async fn export_contact_ep(
    State(s): State<Shared>,
    Path(uid): Path<String>,
) -> Result<Response, ApiError> {
    let contact = interop::get_contact(s.backend.as_ref(), &uid)
        .await?
        .ok_or_else(|| StorageError::NotFound(uid.clone()))?;
    Ok(text_body(interop::MIME_VCARD, contact.to_vcard()))
}

async fn delete_contact_ep(
    State(s): State<Shared>,
    Path(uid): Path<String>,
) -> Result<StatusCode, ApiError> {
    interop::delete_contact(s.backend.as_ref(), &uid).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_events_ep(State(s): State<Shared>) -> Result<Json<Vec<EventDto>>, ApiError> {
    let events = interop::list_events(s.backend.as_ref()).await?;
    Ok(Json(events.into_iter().map(EventDto::from).collect()))
}

/// `POST /api/events/import` — body is an iCalendar `VEVENT`; parse, store, return it.
async fn import_event_ep(
    State(s): State<Shared>,
    body: String,
) -> Result<Json<EventDto>, ApiError> {
    let event = CalendarEvent::from_ics(&body)
        .ok_or_else(|| StorageError::InvalidInput("no VEVENT in input".into()))?;
    let saved = interop::save_event(s.backend.as_ref(), event).await?;
    Ok(Json(saved.into()))
}

async fn export_event_ep(
    State(s): State<Shared>,
    Path(uid): Path<String>,
) -> Result<Response, ApiError> {
    let event = interop::get_event(s.backend.as_ref(), &uid)
        .await?
        .ok_or_else(|| StorageError::NotFound(uid.clone()))?;
    Ok(text_body(interop::MIME_ICALENDAR, event.to_ics()))
}

async fn delete_event_ep(
    State(s): State<Shared>,
    Path(uid): Path<String>,
) -> Result<StatusCode, ApiError> {
    interop::delete_event(s.backend.as_ref(), &uid).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/todos/import` — body is an iCalendar `VTODO`; create a Keeplin to-do note from it.
async fn import_todo_ep(State(s): State<Shared>, body: String) -> Result<Json<Note>, ApiError> {
    Ok(Json(interop::import_todo(s.backend.as_ref(), &body).await?))
}

/// `?name=&email=` for the profile-vCard endpoint.
#[derive(Debug, Deserialize)]
struct ProfileVcardQuery {
    #[serde(default)]
    name: Option<String>,
    email: String,
}

/// `GET /api/profile/vcard?email=&name=` — render the account owner's profile vCard. The caller
/// supplies the profile (the daemon does not own user identity); `name` defaults to the email's
/// local part.
async fn profile_vcard_ep(Query(q): Query<ProfileVcardQuery>) -> Response {
    let name = q
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| q.email.split('@').next().unwrap_or("").to_string());
    text_body(interop::MIME_VCARD, interop::user_vcard(&name, &q.email))
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
    if ordering::is_inbox(id) {
        return Err(StorageError::InvalidInput(
            "the Inbox system notebook cannot be deleted".to_string(),
        )
        .into());
    }
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

/// Streaming upload: `POST /api/resources/upload?title=&file_name=` with the raw file bytes as
/// the body and `Content-Type` as the MIME type. The body is read incrementally up to
/// `max_upload_bytes` (this route bypasses the router's `max_body_bytes` cap), so an attachment
/// larger than `max_message_size` can be uploaded. A body over the cap is rejected with `413`.
async fn upload_resource(
    State(s): State<Shared>,
    Query(meta): Query<ResourceMeta>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let limit = if s.max_upload_bytes == 0 {
        usize::MAX
    } else {
        s.max_upload_bytes
    };
    // `to_bytes` reads the body incrementally and errors once it passes `limit`, so an
    // oversized upload never fully materialises in memory.
    let data = match axum::body::to_bytes(body, limit).await {
        Ok(bytes) => bytes.to_vec(),
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({
                    "error": format!("upload exceeds max_upload_bytes ({})", s.max_upload_bytes)
                })),
            )
                .into_response()
        }
    };
    let mime = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let resource = Resource::new(meta.title, mime, meta.file_name, data.len() as u64);
    match s.backend.create_resource(resource, data).await {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => ApiError(e).into_response(),
    }
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
    crate::server::purge_resources_after_sync(s.backend.as_ref(), s.resource_purge_days).await;
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
            collab: None,
            search: None,
            backend: Arc::new(fs),
            events,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            max_body_bytes: 32 * 1024 * 1024,
            max_upload_bytes: 1024 * 1024 * 1024,
            journal_retention_days: 30,
            resource_purge_days: 0,
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
            collab: None,
            search: None,
            backend: Arc::new(LinkingBackend::new(fs)),
            events,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            max_body_bytes: 32 * 1024 * 1024,
            max_upload_bytes: 1024 * 1024 * 1024,
            journal_retention_days: 30,
            resource_purge_days: 0,
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
    async fn permission_endpoints_require_server_mode() {
        // In fs/offline mode (collab: None) the permission proxies must 503, not panic.
        let st = state(None).await;
        let (_, body) = call(
            &st,
            "POST",
            "/api/notes",
            Some(r#"{"title":"T","body":""}"#),
            None,
        )
        .await;
        let id = serde_json::from_slice::<Note>(&body).unwrap().id;

        for (method, path, payload) in [
            (
                "POST",
                format!("/api/notes/{id}/share"),
                Some(r#"{"capabilities":1}"#),
            ),
            ("GET", format!("/api/notes/{id}/share"), None),
            (
                "POST",
                format!("/api/notes/{id}/transfer"),
                Some(r#"{"user_email":"x@y.z"}"#),
            ),
        ] {
            let (code, _) = call(&st, method, &path, payload, None).await;
            assert_eq!(
                code,
                StatusCode::SERVICE_UNAVAILABLE,
                "{method} {path} must 503 without a server"
            );
        }
    }

    #[tokio::test]
    async fn contact_import_list_export_delete_endpoints() {
        let st = state(None).await;
        let vcard =
            "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:c1\r\nFN:Ada\r\nEMAIL:a@b.com\r\nEND:VCARD\r\n";

        let (code, body) = call(&st, "POST", "/api/contacts/import", Some(vcard), None).await;
        assert_eq!(code, StatusCode::OK);
        let c: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(c["formatted_name"], "Ada");
        assert_eq!(c["uid"], "c1");

        let (code, body) = call(&st, "GET", "/api/contacts", None, None).await;
        assert_eq!(code, StatusCode::OK);
        let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);

        let (code, body) = call(&st, "GET", "/api/contacts/c1/export", None, None).await;
        assert_eq!(code, StatusCode::OK);
        assert!(String::from_utf8(body).unwrap().contains("FN:Ada"));

        let (code, _) = call(&st, "DELETE", "/api/contacts/c1", None, None).await;
        assert_eq!(code, StatusCode::NO_CONTENT);
        let (_, body) = call(&st, "GET", "/api/contacts", None, None).await;
        assert!(serde_json::from_slice::<serde_json::Value>(&body)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn todo_import_and_profile_vcard_endpoints() {
        let st = state(None).await;
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t1\r\nSUMMARY:Task\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let (code, body) = call(&st, "POST", "/api/todos/import", Some(ics), None).await;
        assert_eq!(code, StatusCode::OK);
        let note: Note = serde_json::from_slice(&body).unwrap();
        assert!(note.is_todo);
        assert_eq!(note.title, "Task");

        let (code, body) = call(&st, "GET", "/api/profile/vcard?email=me@x.com", None, None).await;
        assert_eq!(code, StatusCode::OK);
        let text = String::from_utf8(body).unwrap();
        assert!(
            text.contains("FN:me"),
            "name defaults to the email local part"
        );
        assert!(text.contains("EMAIL:me@x.com"));
    }

    #[tokio::test]
    async fn note_history_and_revert_endpoints() {
        let st = state(None).await;

        // Create, then edit — two versions in history.
        let (_, body) = call(
            &st,
            "POST",
            "/api/notes",
            Some(r#"{"title":"T","body":"v1"}"#),
            None,
        )
        .await;
        let note: Note = serde_json::from_slice(&body).unwrap();
        let id = note.id;
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let mut edited = note.clone();
        edited.body = "v2".into();
        let (code, _) = call(
            &st,
            "PUT",
            &format!("/api/notes/{id}"),
            Some(&serde_json::to_string(&edited).unwrap()),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);

        // History: newest first.
        let (code, body) = call(&st, "GET", &format!("/api/notes/{id}/history"), None, None).await;
        assert_eq!(code, StatusCode::OK);
        let hist: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let versions = hist.as_array().unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0]["note"]["body"], "v2");
        assert_eq!(versions[1]["note"]["body"], "v1");

        // Revert to the first version's instant → body back to v1.
        let at = versions[1]["timestamp"].as_str().unwrap();
        let (code, body) = call(
            &st,
            "POST",
            &format!("/api/notes/{id}/revert"),
            Some(&format!(r#"{{"at":"{at}"}}"#)),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(serde_json::from_slice::<Note>(&body).unwrap().body, "v1");
        // The revert is a new version on top (non-destructive).
        let (_, body) = call(&st, "GET", &format!("/api/notes/{id}/history"), None, None).await;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            3
        );
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
            collab: None,
            search: None,
            backend: Arc::new(db),
            events,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            max_body_bytes: 32 * 1024 * 1024,
            max_upload_bytes: 1024 * 1024 * 1024,
            journal_retention_days: 30,
            resource_purge_days: 0,
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
    async fn operational_endpoints_bypass_auth() {
        // With auth configured, the data API requires credentials but the operational
        // probes must remain reachable without them (orchestrators cannot authenticate).
        let st = state(Some(("alice", "s3cr3t"))).await;

        for path in ["/api/health", "/api/ready", "/api/metrics"] {
            let (code, _) = call(&st, "GET", path, None, None).await;
            assert_eq!(
                code,
                StatusCode::OK,
                "{path} must be reachable without auth"
            );
        }
        let (code, _) = call(&st, "GET", "/api/notes", None, None).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED, "data API still gated");
    }

    /// Build an `AppState` whose backend is a `MetricsBackend` over a fresh `FsBackend`
    /// sharing the state's own `Arc<Metrics>` — mirroring how `main` wires the outermost
    /// decorator — so storage operations issued through the router move the same counters
    /// `GET /api/metrics` renders.
    async fn metrics_state() -> Shared {
        use crate::metrics::{Metrics, MetricsBackend};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let fs = FsBackend::new(&path).await.unwrap();
        let metrics = Arc::new(Metrics::new());
        let (events, _rx) = broadcast::channel(16);
        Arc::new(AppState {
            collab: None,
            search: None,
            backend: Arc::new(MetricsBackend::new(fs, metrics.clone())),
            events,
            metrics,
            max_body_bytes: 32 * 1024 * 1024,
            max_upload_bytes: 1024 * 1024 * 1024,
            journal_retention_days: 30,
            resource_purge_days: 0,
            auth_username: None,
            auth_password: None,
        })
    }

    #[tokio::test]
    async fn metrics_reflect_operations_and_http_status() {
        let st = metrics_state().await;

        // One successful create, then one 404 (GET a missing note).
        let (code, _) = call(
            &st,
            "POST",
            "/api/notes",
            Some(r#"{"title":"T","body":"B"}"#),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let (code, _) = call(
            &st,
            "GET",
            &format!("/api/notes/{}", uuid::Uuid::new_v4()),
            None,
            None,
        )
        .await;
        assert_eq!(code, StatusCode::NOT_FOUND);

        let (code, body) = call(&st, "GET", "/api/metrics", None, None).await;
        assert_eq!(code, StatusCode::OK);
        let text = String::from_utf8(body).unwrap();
        assert!(
            text.contains("keeplin_storage_operations_total{entity=\"note\",op=\"create\"} 1"),
            "create counted:\n{text}"
        );
        // The create was a 2xx and the missing-note GET a 4xx; the /metrics scrape itself is
        // not counted (operational routes bypass the status middleware).
        assert!(
            text.contains("keeplin_http_requests_total{status=\"2xx\"} 1"),
            "one 2xx:\n{text}"
        );
        assert!(
            text.contains("keeplin_http_requests_total{status=\"4xx\"} 1"),
            "one 4xx:\n{text}"
        );
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
    async fn streaming_upload_round_trips() {
        let st = state(None).await;
        let payload = "some large attachment bytes";
        let (code, body) = call(
            &st,
            "POST",
            "/api/resources/upload?title=vid&file_name=v.bin",
            Some(payload),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let res: Resource = serde_json::from_slice(&body).unwrap();
        assert_eq!(res.title, "vid");
        assert_eq!(res.size, payload.len() as u64);

        // The uploaded bytes download intact.
        let (code, data) = call(
            &st,
            "GET",
            &format!("/api/resources/{}/data", res.id),
            None,
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(data, payload.as_bytes());
    }

    #[tokio::test]
    async fn streaming_upload_over_cap_is_413() {
        // A state with a tiny 8-byte upload cap rejects a larger streamed body.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let fs = FsBackend::new(&path).await.unwrap();
        let (events, _rx) = broadcast::channel(16);
        let st: Shared = Arc::new(AppState {
            collab: None,
            search: None,
            backend: Arc::new(fs),
            events,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            max_body_bytes: 32 * 1024 * 1024,
            max_upload_bytes: 8,
            journal_retention_days: 30,
            resource_purge_days: 0,
            auth_username: None,
            auth_password: None,
        });
        let (code, _) = call(
            &st,
            "POST",
            "/api/resources/upload?title=big&file_name=big.bin",
            Some("0123456789"),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::PAYLOAD_TOO_LARGE);
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
        // Aliases live in a real notebook (Inbox notes carry none), so place both notes there.
        let nb = Uuid::from_u128(0xa11a5).to_string();

        // Target note, then give it an alias via the alias endpoint.
        let (_, body) = call(
            &st,
            "POST",
            "/api/notes",
            Some(&format!(
                "{{\"title\":\"target\",\"body\":\"[Anchor](###) here\",\"notebook_id\":\"{nb}\"}}"
            )),
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
            Some(&format!(
                "{{\"title\":\"src\",\"body\":\"see [x](#note3)\",\"notebook_id\":\"{nb}\"}}"
            )),
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
        // Aliases only exist outside the Inbox, so plant both notes in a real notebook — the
        // collision is grouped by (alias, notebook_id), so they must share one.
        let nb = Uuid::from_u128(0xc0111de).to_string();
        let (_, b1) = call(
            &st,
            "POST",
            "/api/notes",
            Some(&format!(
                "{{\"title\":\"a\",\"body\":\"\",\"notebook_id\":\"{nb}\"}}"
            )),
            None,
        )
        .await;
        let (_, b2) = call(
            &st,
            "POST",
            "/api/notes",
            Some(&format!(
                "{{\"title\":\"b\",\"body\":\"\",\"notebook_id\":\"{nb}\"}}"
            )),
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
            collab: None,
            search: None,
            backend,
            events,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            max_body_bytes: 32 * 1024 * 1024,
            max_upload_bytes: 1024 * 1024 * 1024,
            journal_retention_days: 30,
            resource_purge_days: 0,
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

// ── Collaborative presence (design §7.3) ─────────────────────────────────────

/// `GET /api/notes/:id/presence` — who is inside the note's live session and
/// where their caret is. Empty when collaboration is disabled or nobody is in.
async fn note_presence(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Json<Vec<keeplin_core::collab::protocol::PresenceInfo>> {
    match &s.collab {
        Some(collab) => Json(collab.presence(id).await),
        None => Json(Vec::new()),
    }
}

#[derive(Deserialize)]
struct CursorBody {
    line_id: Uuid,
    column: usize,
}

/// `PUT /api/notes/:id/cursor` — publish this device's caret position; the
/// server fans the updated presence out to every participant. 503 when
/// collaboration is disabled.
async fn set_cursor(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(body): Json<CursorBody>,
) -> StatusCode {
    match &s.collab {
        Some(collab) => {
            collab.send_cursor(
                id,
                keeplin_core::collab::protocol::Cursor {
                    line_id: body.line_id,
                    column: body.column,
                },
            );
            StatusCode::NO_CONTENT
        }
        None => StatusCode::SERVICE_UNAVAILABLE,
    }
}
