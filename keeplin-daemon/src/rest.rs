//! REST/JSON API served by [axum] on a separate HTTP port.
//!
//! This module exposes the same operations as the gRPC service over plain HTTP with JSON
//! bodies, serialised straight from the `keeplin-core` domain models (no protobuf). The
//! state holds the backend as a trait object (`Arc<dyn StorageBackend>`) so handlers are
//! not generic over the concrete backend type; the gRPC server shares the same backend
//! instance. Authentication reuses the shared constant-time Basic-Auth check in
//! [`crate::auth`]. The live-change WebSocket feed is added in a follow-up.
//!
//! The HTTP listener is plain HTTP — terminate TLS at a reverse proxy in production, as
//! noted in `SECURITY.md`.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, Request, State},
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE},
        HeaderMap, StatusCode,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use keeplin_core::{
    error::StorageError,
    models::{now, Note, NoteTag, Notebook, Resource, Tag},
    storage::StorageBackend,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

/// Shared state for every HTTP handler.
pub struct AppState {
    /// The storage backend, shared (as a trait object) with the gRPC server.
    pub backend: Arc<dyn StorageBackend>,
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
        .route("/notebooks", get(list_notebooks).post(create_notebook))
        .route(
            "/notebooks/:id",
            get(get_notebook)
                .put(update_notebook)
                .delete(delete_notebook),
        )
        .route("/tags", get(list_tags).post(create_tag))
        .route("/tags/:id", get(get_tag).put(update_tag).delete(delete_tag))
        .route("/resources", get(list_resources).post(create_resource))
        .route("/resources/:id", get(get_resource).delete(delete_resource))
        .route("/resources/:id/data", get(get_resource_data))
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

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let code = match &self.0 {
            StorageError::NotFound(_) => StatusCode::NOT_FOUND,
            StorageError::CorruptedData(_) => StatusCode::UNPROCESSABLE_ENTITY,
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

async fn get_notebook(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Json<Notebook>, ApiError> {
    let nb = s.backend.read_notebook(id).await?;
    if nb.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(Json(nb))
}

async fn update_notebook(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(mut nb): Json<Notebook>,
) -> Result<Json<Notebook>, ApiError> {
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

async fn get_tag(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Tag>, ApiError> {
    let tag = s.backend.read_tag(id).await?;
    if tag.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(Json(tag))
}

async fn update_tag(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(mut tag): Json<Tag>,
) -> Result<Json<Tag>, ApiError> {
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

// `POST /api/sync` is added with the WebSocket feed work, where `run_sync` is wired in.

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
        Arc::new(AppState {
            backend: Arc::new(fs),
            auth_username: auth.map(|a| a.0.to_string()),
            auth_password: auth.map(|a| a.1.to_string()),
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
}
