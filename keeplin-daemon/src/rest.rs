// md:Overview

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

// md:AppState
pub struct AppState {
    pub backend: Arc<dyn StorageBackend>,
    pub collab: Option<keeplin_core::collab::CollabHandle>,
    pub search: Option<crate::search::SearchHandle>,
    pub events: broadcast::Sender<Change>,
    pub metrics: Arc<crate::metrics::Metrics>,
    pub max_body_bytes: usize,
    pub max_upload_bytes: usize,
    pub journal_retention_days: u64,
    pub resource_purge_days: u64,
    pub auth_username: Option<String>,
    pub auth_password: Option<String>,
}

// md:Shared
pub type Shared = Arc<AppState>;

// md:fn router
pub fn router(state: Shared) -> Router {
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
        .route(
            "/resources/upload",
            post(upload_resource).layer(DefaultBodyLimit::disable()),
        )
        .route("/resources/:id", get(get_resource).delete(delete_resource))
        .route("/resources/:id/data", get(get_resource_data))
        .route("/sync", post(sync))
        .route("/ws", get(ws_handler))
        .layer(axum::extract::DefaultBodyLimit::max(state.max_body_bytes))
        .layer(middleware::from_fn_with_state(state.clone(), auth_mw))
        .layer(middleware::from_fn_with_state(state.clone(), status_mw))
        .with_state(state);

    Router::new().nest("/api", ops.merge(api))
}

// md:SearchParams
#[derive(Debug, Deserialize)]
struct SearchParams {
    q: Option<String>,
    notebook: Option<Uuid>,
    todo: Option<bool>,
    open: Option<bool>,
    starred: Option<bool>,
    pinned: Option<bool>,
    due_after: Option<DateTime<Utc>>,
    due_before: Option<DateTime<Utc>>,
    updated_after: Option<DateTime<Utc>>,
    updated_before: Option<DateTime<Utc>>,
    limit: Option<usize>,
}

// md:fn search_notes
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
    let mut notes = Vec::with_capacity(ids.len());
    for id in ids {
        if let Ok(note) = s.backend.read_note(id).await {
            notes.push(note);
        }
    }
    Json(notes).into_response()
}

// md:fn auth_mw
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

// md:fn status_mw
async fn status_mw(State(state): State<Shared>, req: Request, next: Next) -> Response {
    let resp = next.run(req).await;
    state.metrics.record_http_status(resp.status().as_u16());
    resp
}

// md:ApiError
struct ApiError(StorageError);

// md:impl From StorageError for ApiError
impl From<StorageError> for ApiError {
    fn from(e: StorageError) -> Self {
        ApiError(e)
    }
}

// md:impl From SyncError for ApiError
impl From<SyncError> for ApiError {
    fn from(e: SyncError) -> Self {
        match e {
            SyncError::Storage(s) => ApiError(s),
            other => ApiError(StorageError::InvalidState(other.to_string())),
        }
    }
}

// md:impl IntoResponse for ApiError
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let code = match &self.0 {
            StorageError::NotFound(_) => StatusCode::NOT_FOUND,
            StorageError::CorruptedData(_) => StatusCode::UNPROCESSABLE_ENTITY,
            StorageError::Conflict(_) => StatusCode::CONFLICT,
            StorageError::InvalidInput(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (code, Json(json!({ "error": self.0.to_string() }))).into_response()
    }
}

// md:Pagination
#[derive(Debug, Deserialize)]
struct Pagination {
    #[serde(default)]
    page_size: u32,
    #[serde(default)]
    page_token: Option<String>,
}

// md:Page
#[derive(Debug, Serialize)]
struct Page<T> {
    items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_page_token: Option<String>,
}

// md:fn page
fn page<T>((items, next): (Vec<T>, Option<String>)) -> Json<Page<T>> {
    Json(Page {
        items,
        next_page_token: next,
    })
}

// md:fn health
async fn health() -> &'static str {
    "ok"
}

// md:fn ready
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

// md:fn metrics
async fn metrics(State(s): State<Shared>) -> Response {
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4")],
        s.metrics.render_prometheus(),
    )
        .into_response()
}

// md:CreateNote
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

// md:fn list_notes
async fn list_notes(
    State(s): State<Shared>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Note>>, ApiError> {
    Ok(page(s.backend.list_notes(p.page_size, p.page_token).await?))
}

// md:fn create_note
async fn create_note(
    State(s): State<Shared>,
    Json(req): Json<CreateNote>,
) -> Result<Json<Note>, ApiError> {
    let mut note = Note::new(req.title, req.body);
    note.notebook_id = req.notebook_id.unwrap_or_else(Uuid::nil);
    note.is_todo = req.is_todo;
    note.todo_due = req.todo_due;
    ordering::place_new_note(s.backend.as_ref(), &mut note).await?;
    Ok(Json(s.backend.create_note(note).await?))
}

// md:fn get_note
async fn get_note(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Note>, ApiError> {
    let note = s.backend.read_note(id).await?;
    if note.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(Json(note))
}

// md:fn update_note
async fn update_note(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(mut note): Json<Note>,
) -> Result<Json<Note>, ApiError> {
    let stored = read_live_note(&s, id).await?;
    note.id = id;
    ordering::reconcile_notebook_move(s.backend.as_ref(), stored.notebook_id, &mut note).await?;
    note.updated_at = now();
    Ok(Json(s.backend.update_note(note).await?))
}

// md:fn delete_note
async fn delete_note(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.backend.delete_note(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// md:fn list_note_tags
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

// md:fn add_note_tag
async fn add_note_tag(
    State(s): State<Shared>,
    Path((note_id, tag_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    s.backend.add_note_tag(NoteTag { note_id, tag_id }).await?;
    Ok(StatusCode::NO_CONTENT)
}

// md:fn remove_note_tag
async fn remove_note_tag(
    State(s): State<Shared>,
    Path((note_id, tag_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    s.backend.remove_note_tag(note_id, tag_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// md:AliasBody
#[derive(Debug, Deserialize)]
struct AliasBody {
    #[serde(default)]
    alias: Option<String>,
}

// md:fn read_live_note
async fn read_live_note(s: &Shared, id: Uuid) -> Result<Note, ApiError> {
    let note = s.backend.read_note(id).await?;
    if note.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(note)
}

// md:fn set_note_alias
async fn set_note_alias(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<AliasBody>,
) -> Result<Json<Note>, ApiError> {
    Ok(Json(
        linking::set_note_alias(s.backend.as_ref(), id, b.alias).await?,
    ))
}

// md:fn list_links
async fn list_links(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<NoteLink>>, ApiError> {
    Ok(Json(read_live_note(&s, id).await?.links))
}

// md:AddLinkBody
#[derive(Debug, Deserialize)]
struct AddLinkBody {
    raw: String,
}

// md:fn add_link
async fn add_link(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<AddLinkBody>,
) -> Result<Json<Note>, ApiError> {
    if parse_link_ref(&b.raw).is_none() {
        return Err(
            StorageError::CorruptedData(format!("invalid link reference '{}'", b.raw)).into(),
        );
    }
    Ok(Json(
        linking::add_manual_link(s.backend.as_ref(), id, &b.raw).await?,
    ))
}

// md:fn remove_link
async fn remove_link(
    State(s): State<Shared>,
    Path((id, index)): Path<(Uuid, usize)>,
) -> Result<Json<Note>, ApiError> {
    Ok(Json(
        linking::remove_link(s.backend.as_ref(), id, index).await?,
    ))
}

// md:fn list_backlinks
async fn list_backlinks(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Note>>, ApiError> {
    Ok(page(
        linking::backlinks(s.backend.as_ref(), id, p.page_size, p.page_token).await?,
    ))
}

// md:fn list_notes_in_notebook
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

// md:fn list_starred_notes
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

// md:fn pin_note
async fn pin_note(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Note>, ApiError> {
    Ok(Json(ordering::pin_note(s.backend.as_ref(), id).await?))
}

// md:fn unpin_note
async fn unpin_note(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Note>, ApiError> {
    Ok(Json(ordering::unpin_note(s.backend.as_ref(), id).await?))
}

// md:fn star_note
async fn star_note(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Note>, ApiError> {
    Ok(Json(ordering::star_note(s.backend.as_ref(), id).await?))
}

// md:fn unstar_note
async fn unstar_note(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Json<Note>, ApiError> {
    Ok(Json(ordering::unstar_note(s.backend.as_ref(), id).await?))
}

// md:ReorderBody
#[derive(Deserialize)]
struct ReorderBody {
    sort_key: u32,
}

// md:fn reorder_note
async fn reorder_note(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<ReorderBody>,
) -> Result<Json<Note>, ApiError> {
    Ok(Json(
        ordering::reorder_note(s.backend.as_ref(), id, b.sort_key).await?,
    ))
}

// md:ResolveQuery
#[derive(Debug, Deserialize)]
struct ResolveQuery {
    #[serde(rename = "ref")]
    reference: String,
}

// md:fn resolve_reference
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

// md:fn list_alias_conflicts
async fn list_alias_conflicts(
    State(s): State<Shared>,
) -> Result<Json<linking::AliasConflicts>, ApiError> {
    Ok(Json(linking::alias_conflicts(s.backend.as_ref()).await?))
}

// md:HistoryQuery
#[derive(Debug, Default, Deserialize)]
struct HistoryQuery {
    #[serde(default)]
    limit: u32,
}

// md:NoteVersion
#[derive(Debug, Serialize)]
struct NoteVersion {
    timestamp: DateTime<Utc>,
    device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<Note>,
}

// md:NotebookVersion
#[derive(Debug, Serialize)]
struct NotebookVersion {
    timestamp: DateTime<Utc>,
    device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    notebook: Option<Notebook>,
}

// md:RevertBody
#[derive(Debug, Deserialize)]
struct RevertBody {
    at: DateTime<Utc>,
}

// md:BatchRevertBody
#[derive(Debug, Deserialize)]
struct BatchRevertBody {
    at: DateTime<Utc>,
    #[serde(default)]
    note_ids: Vec<Uuid>,
}

// md:fn note_history
async fn note_history(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<NoteVersion>>, ApiError> {
    let versions = s.backend.note_history(id, q.limit).await?;
    Ok(Json(versions.into_iter().map(note_version_dto).collect()))
}

// md:fn notebook_history
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

// md:fn revert_note_ep
async fn revert_note_ep(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<RevertBody>,
) -> Result<Json<Note>, ApiError> {
    Ok(Json(
        history::revert_note(s.backend.as_ref(), id, b.at).await?,
    ))
}

// md:fn revert_notebook_ep
async fn revert_notebook_ep(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<RevertBody>,
) -> Result<Json<Notebook>, ApiError> {
    Ok(Json(
        history::revert_notebook(s.backend.as_ref(), id, b.at).await?,
    ))
}

// md:fn revert_notebook_notes_ep
async fn revert_notebook_notes_ep(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<RevertBody>,
) -> Result<Json<Vec<Note>>, ApiError> {
    Ok(Json(
        history::revert_notebook_notes_to(s.backend.as_ref(), id, b.at).await?,
    ))
}

// md:fn batch_revert_notes_ep
async fn batch_revert_notes_ep(
    State(s): State<Shared>,
    Json(b): Json<BatchRevertBody>,
) -> Result<Json<Vec<Note>>, ApiError> {
    Ok(Json(
        history::revert_notes_to(s.backend.as_ref(), &b.note_ids, b.at).await?,
    ))
}

// md:fn note_version_dto
fn note_version_dto(v: EntityVersion<Note>) -> NoteVersion {
    NoteVersion {
        timestamp: v.timestamp,
        device_id: v.device_id,
        note: v.entity,
    }
}

// md:fn notebook_version_dto
fn notebook_version_dto(v: EntityVersion<Notebook>) -> NotebookVersion {
    NotebookVersion {
        timestamp: v.timestamp,
        device_id: v.device_id,
        notebook: v.entity,
    }
}

// md:fn proxy_perm
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

// md:fn proxy_note_share
async fn proxy_note_share(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    proxy_perm(&s, "POST", format!("/api/notes/{id}/share"), Some(body)).await
}

// md:fn proxy_note_shares
async fn proxy_note_shares(State(s): State<Shared>, Path(id): Path<Uuid>) -> Response {
    proxy_perm(&s, "GET", format!("/api/notes/{id}/share"), None).await
}

// md:fn proxy_note_unshare
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

// md:fn proxy_note_transfer
async fn proxy_note_transfer(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    proxy_perm(&s, "POST", format!("/api/notes/{id}/transfer"), Some(body)).await
}

// md:fn proxy_notebook_share
async fn proxy_notebook_share(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    proxy_perm(&s, "POST", format!("/api/notebooks/{id}/share"), Some(body)).await
}

// md:fn proxy_notebook_shares
async fn proxy_notebook_shares(State(s): State<Shared>, Path(id): Path<Uuid>) -> Response {
    proxy_perm(&s, "GET", format!("/api/notebooks/{id}/share"), None).await
}

// md:fn proxy_notebook_unshare
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

// md:fn proxy_notebook_transfer
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

// md:ContactDto
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

// md:impl From Contact for ContactDto
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

// md:EventDto
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

// md:impl From CalendarEvent for EventDto
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

// md:fn text_body
fn text_body(mime: &'static str, body: String) -> Response {
    ([(CONTENT_TYPE, mime)], body).into_response()
}

// md:fn list_contacts_ep
async fn list_contacts_ep(State(s): State<Shared>) -> Result<Json<Vec<ContactDto>>, ApiError> {
    let contacts = interop::list_contacts(s.backend.as_ref()).await?;
    Ok(Json(contacts.into_iter().map(ContactDto::from).collect()))
}

// md:fn import_contact_ep
async fn import_contact_ep(
    State(s): State<Shared>,
    body: String,
) -> Result<Json<ContactDto>, ApiError> {
    let contact = Contact::from_vcard(&body)
        .ok_or_else(|| StorageError::InvalidInput("invalid vCard".into()))?;
    let saved = interop::save_contact(s.backend.as_ref(), contact).await?;
    Ok(Json(saved.into()))
}

// md:fn export_contact_ep
async fn export_contact_ep(
    State(s): State<Shared>,
    Path(uid): Path<String>,
) -> Result<Response, ApiError> {
    let contact = interop::get_contact(s.backend.as_ref(), &uid)
        .await?
        .ok_or_else(|| StorageError::NotFound(uid.clone()))?;
    Ok(text_body(interop::MIME_VCARD, contact.to_vcard()))
}

// md:fn delete_contact_ep
async fn delete_contact_ep(
    State(s): State<Shared>,
    Path(uid): Path<String>,
) -> Result<StatusCode, ApiError> {
    interop::delete_contact(s.backend.as_ref(), &uid).await?;
    Ok(StatusCode::NO_CONTENT)
}

// md:fn list_events_ep
async fn list_events_ep(State(s): State<Shared>) -> Result<Json<Vec<EventDto>>, ApiError> {
    let events = interop::list_events(s.backend.as_ref()).await?;
    Ok(Json(events.into_iter().map(EventDto::from).collect()))
}

// md:fn import_event_ep
async fn import_event_ep(
    State(s): State<Shared>,
    body: String,
) -> Result<Json<Vec<EventDto>>, ApiError> {
    let events = CalendarEvent::from_ics_all(&body);
    if events.is_empty() {
        return Err(StorageError::InvalidInput("no VEVENT in input".into()).into());
    }
    let mut saved = Vec::with_capacity(events.len());
    for event in events {
        saved.push(EventDto::from(
            interop::save_event(s.backend.as_ref(), event).await?,
        ));
    }
    Ok(Json(saved))
}

// md:fn export_event_ep
async fn export_event_ep(
    State(s): State<Shared>,
    Path(uid): Path<String>,
) -> Result<Response, ApiError> {
    let event = interop::get_event(s.backend.as_ref(), &uid)
        .await?
        .ok_or_else(|| StorageError::NotFound(uid.clone()))?;
    Ok(text_body(interop::MIME_ICALENDAR, event.to_ics()))
}

// md:fn delete_event_ep
async fn delete_event_ep(
    State(s): State<Shared>,
    Path(uid): Path<String>,
) -> Result<StatusCode, ApiError> {
    interop::delete_event(s.backend.as_ref(), &uid).await?;
    Ok(StatusCode::NO_CONTENT)
}

// md:fn import_todo_ep
async fn import_todo_ep(
    State(s): State<Shared>,
    body: String,
) -> Result<Json<Vec<Note>>, ApiError> {
    Ok(Json(
        interop::import_todos(s.backend.as_ref(), &body).await?,
    ))
}

// md:ProfileVcardQuery
#[derive(Debug, Deserialize)]
struct ProfileVcardQuery {
    #[serde(default)]
    name: Option<String>,
    email: String,
}

// md:fn profile_vcard_ep
async fn profile_vcard_ep(Query(q): Query<ProfileVcardQuery>) -> Response {
    let name = q
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| q.email.split('@').next().unwrap_or("").to_string());
    text_body(interop::MIME_VCARD, interop::user_vcard(&name, &q.email))
}

// md:TitleOnly
#[derive(Debug, Deserialize)]
struct TitleOnly {
    title: String,
}

// md:fn list_notebooks
async fn list_notebooks(
    State(s): State<Shared>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Notebook>>, ApiError> {
    Ok(page(
        s.backend.list_notebooks(p.page_size, p.page_token).await?,
    ))
}

// md:fn create_notebook
async fn create_notebook(
    State(s): State<Shared>,
    Json(req): Json<TitleOnly>,
) -> Result<Json<Notebook>, ApiError> {
    Ok(Json(
        s.backend.create_notebook(Notebook::new(req.title)).await?,
    ))
}

// md:fn read_live_notebook
async fn read_live_notebook(s: &Shared, id: Uuid) -> Result<Notebook, ApiError> {
    let nb = s.backend.read_notebook(id).await?;
    if nb.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(nb)
}

// md:fn get_notebook
async fn get_notebook(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Json<Notebook>, ApiError> {
    Ok(Json(read_live_notebook(&s, id).await?))
}

// md:fn update_notebook
async fn update_notebook(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(mut nb): Json<Notebook>,
) -> Result<Json<Notebook>, ApiError> {
    read_live_notebook(&s, id).await?;
    nb.id = id;
    nb.updated_at = now();
    Ok(Json(s.backend.update_notebook(nb).await?))
}

// md:fn delete_notebook
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

// md:fn set_notebook_alias
async fn set_notebook_alias(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(b): Json<AliasBody>,
) -> Result<Json<Notebook>, ApiError> {
    Ok(Json(
        linking::set_notebook_alias(s.backend.as_ref(), id, b.alias).await?,
    ))
}

// md:fn list_tags
async fn list_tags(
    State(s): State<Shared>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Tag>>, ApiError> {
    Ok(page(s.backend.list_tags(p.page_size, p.page_token).await?))
}

// md:fn create_tag
async fn create_tag(
    State(s): State<Shared>,
    Json(req): Json<TitleOnly>,
) -> Result<Json<Tag>, ApiError> {
    Ok(Json(s.backend.create_tag(Tag::new(req.title)).await?))
}

// md:fn read_live_tag
async fn read_live_tag(s: &Shared, id: Uuid) -> Result<Tag, ApiError> {
    let tag = s.backend.read_tag(id).await?;
    if tag.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(tag)
}

// md:fn get_tag
async fn get_tag(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Tag>, ApiError> {
    Ok(Json(read_live_tag(&s, id).await?))
}

// md:fn update_tag
async fn update_tag(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(mut tag): Json<Tag>,
) -> Result<Json<Tag>, ApiError> {
    read_live_tag(&s, id).await?;
    tag.id = id;
    tag.updated_at = now();
    Ok(Json(s.backend.update_tag(tag).await?))
}

// md:fn delete_tag
async fn delete_tag(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    s.backend.delete_tag(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// md:ResourceMeta
#[derive(Debug, Deserialize)]
struct ResourceMeta {
    #[serde(default)]
    title: String,
    #[serde(default)]
    file_name: String,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

// md:fn list_resources
async fn list_resources(
    State(s): State<Shared>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Resource>>, ApiError> {
    Ok(page(
        s.backend.list_resources(p.page_size, p.page_token).await?,
    ))
}

// md:fn create_resource
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
    let mut resource = Resource::new(meta.title, mime, meta.file_name, data.len() as u64);
    resource.duration_ms = meta.duration_ms;
    resource.dimensions = match (meta.width, meta.height) {
        (Some(w), Some(h)) => Some((w, h)),
        _ => None,
    };
    Ok(Json(s.backend.create_resource(resource, data).await?))
}

// md:fn upload_resource
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
    let mut resource = Resource::new(meta.title, mime, meta.file_name, data.len() as u64);
    resource.duration_ms = meta.duration_ms;
    resource.dimensions = match (meta.width, meta.height) {
        (Some(w), Some(h)) => Some((w, h)),
        _ => None,
    };
    match s.backend.create_resource(resource, data).await {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => ApiError(e).into_response(),
    }
}

// md:fn get_resource
async fn get_resource(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Json<Resource>, ApiError> {
    let (meta, _data) = s.backend.read_resource(id).await?;
    Ok(Json(meta))
}

// md:fn get_resource_data
async fn get_resource_data(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let (meta, data) = s.backend.read_resource(id).await?;
    Ok(([(CONTENT_TYPE, meta.mime_type)], data).into_response())
}

// md:fn delete_resource
async fn delete_resource(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.backend.delete_resource(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// md:SyncSummary
#[derive(Debug, Serialize)]
struct SyncSummary {
    applied: usize,
}

// md:fn sync
async fn sync(State(s): State<Shared>) -> Result<Json<SyncSummary>, ApiError> {
    let applied = run_sync(s.backend.as_ref(), |_stage, _count| {}).await?;
    crate::server::prune_journal_after_sync(s.backend.as_ref(), s.journal_retention_days).await;
    crate::server::purge_resources_after_sync(s.backend.as_ref(), s.resource_purge_days).await;
    Ok(Json(SyncSummary {
        applied: applied.len(),
    }))
}

// md:fn ws_handler
async fn ws_handler(State(s): State<Shared>, ws: WebSocketUpgrade) -> Response {
    let rx = s.events.subscribe();
    ws.on_upgrade(move |socket| stream_changes(socket, rx))
}

// md:fn stream_changes
async fn stream_changes(mut socket: WebSocket, mut rx: broadcast::Receiver<Change>) {
    loop {
        tokio::select! {
            received = rx.recv() => match received {
                Ok(change) => {
                    let text = serde_json::to_string(&change)
                        .unwrap_or_else(|_| r#"{"type":"error"}"#.to_string());
                    if socket.send(Message::Text(text)).await.is_err() {
                        break;
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
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
        }
    }
}

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use keeplin_core::storage::fs::FsBackend;
    use tower::ServiceExt;

    // md:mod tests > fn state
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

    // md:mod tests > fn linking_state
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

    // md:mod tests > fn call
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

    // md:mod tests > fn note_crud_round_trip
    #[tokio::test]
    async fn note_crud_round_trip() {
        let st = state(None).await;

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

        let (code, body) = call(&st, "GET", &format!("/api/notes/{id}"), None, None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(serde_json::from_slice::<Note>(&body).unwrap().body, "B");

        let (code, body) = call(&st, "GET", "/api/notes", None, None).await;
        assert_eq!(code, StatusCode::OK);
        let pagev: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(pagev["items"].as_array().unwrap().len(), 1);

        let (code, _) = call(&st, "DELETE", &format!("/api/notes/{id}"), None, None).await;
        assert_eq!(code, StatusCode::NO_CONTENT);
        let (code, _) = call(&st, "GET", &format!("/api/notes/{id}"), None, None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    // md:mod tests > fn permission_endpoints_require_server_mode
    #[tokio::test]
    async fn permission_endpoints_require_server_mode() {
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

    // md:mod tests > fn contact_import_list_export_delete_endpoints
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

    // md:mod tests > fn todo_import_and_profile_vcard_endpoints
    #[tokio::test]
    async fn todo_import_and_profile_vcard_endpoints() {
        let st = state(None).await;
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t1\r\nSUMMARY:Task\r\nEND:VTODO\r\n\
                   BEGIN:VTODO\r\nUID:t2\r\nSUMMARY:Other\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let (code, body) = call(&st, "POST", "/api/todos/import", Some(ics), None).await;
        assert_eq!(code, StatusCode::OK);
        let notes: Vec<Note> = serde_json::from_slice(&body).unwrap();
        assert_eq!(notes.len(), 2, "every VTODO in the calendar imports");
        assert!(notes[0].is_todo);
        assert_eq!(notes[0].title, "Task");
        assert_eq!(notes[1].title, "Other");

        let (code, body) = call(&st, "GET", "/api/profile/vcard?email=me@x.com", None, None).await;
        assert_eq!(code, StatusCode::OK);
        let text = String::from_utf8(body).unwrap();
        assert!(
            text.contains("FN:me"),
            "name defaults to the email local part"
        );
        assert!(text.contains("EMAIL:me@x.com"));
    }

    // md:mod tests > fn note_history_and_revert_endpoints
    #[tokio::test]
    async fn note_history_and_revert_endpoints() {
        let st = state(None).await;

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

        let (code, body) = call(&st, "GET", &format!("/api/notes/{id}/history"), None, None).await;
        assert_eq!(code, StatusCode::OK);
        let hist: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let versions = hist.as_array().unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0]["note"]["body"], "v2");
        assert_eq!(versions[1]["note"]["body"], "v1");

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

    // md:mod tests > fn updates_on_deleted_entities_are_404
    #[tokio::test]
    async fn updates_on_deleted_entities_are_404() {
        let st = state(None).await;

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

        let update = serde_json::to_string(&note).unwrap();
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
        let (code, _) = call(&st, "GET", &format!("/api/notes/{}", note.id), None, None).await;
        assert_eq!(code, StatusCode::NOT_FOUND, "note must remain deleted");

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

    // md:mod tests > fn sync_endpoint_prunes_journal_within_retention
    #[tokio::test]
    async fn sync_endpoint_prunes_journal_within_retention() {
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

        let epoch = chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap();
        let journal = st.backend.get_changes_since(epoch).await.unwrap();
        assert_eq!(journal.len(), 1, "recent journal rows survive the prune");
    }

    // md:mod tests > fn operational_endpoints_bypass_auth
    #[tokio::test]
    async fn operational_endpoints_bypass_auth() {
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

    // md:mod tests > fn metrics_state
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

    // md:mod tests > fn metrics_reflect_operations_and_http_status
    #[tokio::test]
    async fn metrics_reflect_operations_and_http_status() {
        let st = metrics_state().await;

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
        assert!(
            text.contains("keeplin_http_requests_total{status=\"2xx\"} 1"),
            "one 2xx:\n{text}"
        );
        assert!(
            text.contains("keeplin_http_requests_total{status=\"4xx\"} 1"),
            "one 4xx:\n{text}"
        );
    }

    // md:mod tests > fn invalid_uuid_is_bad_request
    #[tokio::test]
    async fn invalid_uuid_is_bad_request() {
        let st = state(None).await;
        let (code, _) = call(&st, "GET", "/api/notes/not-a-uuid", None, None).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    // md:mod tests > fn auth_is_enforced_when_configured
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

    // md:mod tests > fn resource_upload_and_download
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

    // md:mod tests > fn resource_upload_above_axum_default_limit
    #[tokio::test]
    async fn resource_upload_above_axum_default_limit() {
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

    // md:mod tests > fn streaming_upload_round_trips
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

    // md:mod tests > fn streaming_upload_over_cap_is_413
    #[tokio::test]
    async fn streaming_upload_over_cap_is_413() {
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

    // md:mod tests > fn alias_and_links_endpoints
    #[tokio::test]
    async fn alias_and_links_endpoints() {
        let st = linking_state().await;

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

        assert_eq!(note.bookmarks.len(), 1);
        assert_eq!(note.bookmarks[0].number, 1);
        assert_eq!(note.bookmarks[0].text, "Anchor1");
        assert_eq!(note.bookmarks[0].alias, "Custom");

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

    // md:mod tests > fn alias_backlinks_and_resolve_endpoints
    #[tokio::test]
    async fn alias_backlinks_and_resolve_endpoints() {
        let st = linking_state().await;
        let nb = Uuid::from_u128(0xa11a5).to_string();

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

    // md:mod tests > fn alias_conflicts_endpoint
    #[tokio::test]
    async fn alias_conflicts_endpoint() {
        let st = state(None).await;
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

    use crate::event_backend::EventBackend;
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    // md:mod tests > fn state_with_events
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

    // md:mod tests > fn websocket_streams_note_create
    #[tokio::test]
    async fn websocket_streams_note_create() {
        let st = state_with_events().await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = router(st.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let (mut ws, _resp) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws"))
            .await
            .expect("ws connect");

        let (code, _) = call(
            &st,
            "POST",
            "/api/notes",
            Some(r#"{"title":"hello","body":"world"}"#),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);

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

// md:fn note_presence
async fn note_presence(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Json<Vec<keeplin_core::collab::protocol::PresenceInfo>> {
    match &s.collab {
        Some(collab) => Json(collab.presence(id).await),
        None => Json(Vec::new()),
    }
}

// md:CursorBody
#[derive(Deserialize)]
struct CursorBody {
    line_id: Uuid,
    column: usize,
}

// md:fn set_cursor
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
