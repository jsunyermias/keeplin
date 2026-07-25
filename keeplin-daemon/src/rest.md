# `rest.rs` — the REST/JSON + WebSocket surface

Self-contained companion for `keeplin-daemon/src/rest.rs`. It documents **every
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

**Identification** — file-level block: the module doc and the imports. Marker
`// md:Overview`.

**Code** — complete and verbatim:

```rust
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
    format, history,
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
```

**What it does** — The REST/JSON API served by axum on a separate HTTP port
(`http_addr`). It exposes the same operations as the gRPC service over plain HTTP
with JSON bodies serialised **straight from the `keeplin-core` domain models** (no
protobuf, no DTO layer for entities). The state holds the backend as a trait
object (`Arc<dyn StorageBackend>`) so handlers are not generic; the gRPC server
shares the same backend instance. Authentication reuses the shared constant-time
Basic-Auth check in `crate::auth`. `GET /api/ws` upgrades to a WebSocket streaming
every `Change` published by the daemon's `EventBackend`; `POST /api/sync` runs one
sync cycle. Three **operational** endpoints — `/api/health`, `/api/ready`,
`/api/metrics` — sit outside the auth middleware and the HTTP-status counter so
probes and scrapers work without credentials and do not inflate request metrics.
The listener is plain HTTP — terminate TLS at a reverse proxy (SECURITY.md).

**Used by** — `main.rs` (`AppState` construction + `router`).

**Repeated context** — soft-delete convention: backends retain deleted entities as
tombstones (sync needs them); this surface presents a clean lifecycle, so a
tombstone reads and updates as `404`. Ordering convention: pinned band
`1..=999`, normal band from `NORMAL_START = 1000`; the Inbox is the nil-UUID
system notebook.

---

## AppState

**Identification** — `pub struct AppState`. Marker `// md:AppState`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Shared state for every HTTP handler: `backend:
Arc<dyn StorageBackend>` (shared with the gRPC server), `collab:
Option<CollabHandle>` (presence/cursor + permission proxying; `None` outside
server mode), `search: Option<SearchHandle>` (`None` → search responds 503),
`events: broadcast::Sender<Change>` (each WebSocket connection subscribes a fresh
receiver), `metrics: Arc<Metrics>` (same registry the outermost `MetricsBackend`
decorator writes), `max_body_bytes` (request-body cap mirroring gRPC
`max_message_size`), `max_upload_bytes` (cap for the **streaming** upload route,
which bypasses `max_body_bytes`; `0` = unlimited), `journal_retention_days` and
`resource_purge_days` (post-sync maintenance, same semantics as gRPC), and the
optional `auth_username`/`auth_password` pair.

**Used by** — `main.rs::run_server_with` (construction); every handler below.

---

## Shared

**Identification** — `pub type Shared = Arc<AppState>`. Marker `// md:Shared`.

**Code** — complete and verbatim:

```rust
// md:Shared
pub type Shared = Arc<AppState>;
```

**What it does** — Handler-facing state alias; `Arc` makes it cheaply cloneable
for axum's `State` extractor.

---

## fn router

**Identification** — `pub fn router(state: Shared) -> Router`. Marker
`// md:fn router`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Builds the `/api` router as `ops.merge(api)` nested under
`/api`. `ops` (`/health`, `/ready`, `/metrics`) sits **outside** the auth
middleware and the status counter — probes and scrapers cannot present
credentials, and their traffic must not drown the request metrics. `api` carries
the full data surface:

| Area | Routes |
|---|---|
| Notes | `/notes` GET/POST; `/notes/:id` GET/PUT/DELETE; `/notes/starred`; `/notes/:id/pin` POST/DELETE; `…/star` POST/DELETE; `…/sort-key` PUT |
| Tags on notes | `/notes/:id/tags` GET; `/notes/:note_id/tags/:tag_id` PUT/DELETE |
| Aliases & links | `/notes/:id/alias` PUT; `…/links` GET/POST; `…/links/:index` DELETE; `…/backlinks`; `/links/resolve`; `/aliases/conflicts` |
| History | `/notes/:id/history`, `/notes/:id/revert`, `/notebooks/:id/history`, `/notebooks/:id/revert`, `/notebooks/:id/notes/revert`, `/history/revert` |
| Collab | `/notes/:id/presence` GET; `/notes/:id/cursor` PUT |
| Permission proxies | `/notes/:id/share` POST/GET; `…/share/:user_id` DELETE; `…/transfer` POST; same four under `/notebooks/:id/…` |
| Search | `/search` GET |
| Notebooks | `/notebooks` GET/POST; `/notebooks/:id` GET/PUT/DELETE; `…/alias` PUT; `…/notes` GET |
| Tags | `/tags` GET/POST; `/tags/:id` GET/PUT/DELETE |
| Interop | `/contacts`, `/contacts/import`, `/contacts/:uid` DELETE, `…/export`; `/events…` likewise; `/todos/import`; `/profile/vcard` |
| Resources | `/resources` GET/POST; `/resources/upload` POST (body limit **disabled** — it enforces its own `max_upload_bytes` cap); `/resources/:id` GET/DELETE; `…/data` GET |
| Sync & feed | `/sync` POST; `/ws` GET |

Layer order matters: `DefaultBodyLimit::max(max_body_bytes)` raises axum's 2 MiB
default so REST uploads match gRPC; layers apply outermost-last, so **auth runs
inside the status counter** — a rejected request is still counted (as a 4xx) by
`status_mw`.

**Used by** — `main.rs::run_server_with`; the tests' `call` helper.

---

## SearchParams

**Identification** — `#[derive(Debug, Deserialize)] struct SearchParams`. Marker
`// md:SearchParams`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Query parameters for `GET /api/search`, all optional: `q`
(free text over title/body/tag names/notebook name), `notebook` (UUID), `todo`,
`open` (`true` = open to-dos only, `false` = completed only), `starred`,
`pinned`, `due_after`/`due_before`, `updated_after`/`updated_before` (RFC-3339),
`limit`.

**Used by** — `search_notes`.

---

## fn search_notes

**Identification** — `async fn search_notes(State, Query<SearchParams>) ->
Response`. Marker `// md:fn search_notes`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `GET /api/search`: full-text search over the daemon's
in-memory FTS index, best match first. `503` when `s.search` is `None` (index
unavailable); `500` on a query error. Maps params into
`crate::search::SearchQuery` (`limit` defaults to 50), gets matching ids, then
resolves each id to a full note through the **backend** (plaintext), skipping
any that raced a deletion between index query and read. Returns the notes as
JSON.

**Used by** — routed from `router`. No other callers.

---

## fn auth_mw

**Identification** — `async fn auth_mw(State, Request, Next) -> Response`.
Marker `// md:fn auth_mw`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Middleware: when both credentials are configured, extracts
the `Authorization` header and checks it with `crate::auth::verify_basic`
(RFC 7617 parse, first-colon split, `subtle::ConstantTimeEq` comparison —
mirroring the gRPC interceptor). Failure → `401` with `WWW-Authenticate: Basic`
and a JSON error. No-op when auth is not configured.

**Used by** — layered onto the data API in `router` (not the ops routes).

---

## fn status_mw

**Identification** — `async fn status_mw(State, Request, Next) -> Response`.
Marker `// md:fn status_mw`.

**Code** — complete and verbatim:

```rust
// md:fn status_mw
async fn status_mw(State(state): State<Shared>, req: Request, next: Next) -> Response {
    let resp = next.run(req).await;
    state.metrics.record_http_status(resp.status().as_u16());
    resp
}
```

**What it does** — Middleware: runs the request, then records the response's
status class into the shared registry (`keeplin_http_requests_total` via
`Metrics::record_http_status`). Applied only to the data API, so probe/scrape
traffic does not inflate the counts.

**Used by** — layered onto the data API in `router`.

---

## ApiError

**Identification** — `struct ApiError(StorageError)`. Marker `// md:ApiError`.

**Code** — complete and verbatim:

```rust
// md:ApiError
struct ApiError(StorageError);
```

**What it does** — Newtype letting handlers return `Result<_, ApiError>` with
`?` on backend calls; the `IntoResponse` impl below does the HTTP mapping.

**Used by** — every fallible handler in this file.

---

## impl From StorageError for ApiError

**Identification** — marker `// md:impl From StorageError for ApiError`.

**Code** — complete and verbatim:

```rust
// md:impl From StorageError for ApiError
impl From<StorageError> for ApiError {
    fn from(e: StorageError) -> Self {
        ApiError(e)
    }
}
```

**What it does** — Plain wrap, enabling `?` on storage results.

---

## impl From SyncError for ApiError

**Identification** — marker `// md:impl From SyncError for ApiError`.

**Code** — complete and verbatim:

```rust
// md:impl From SyncError for ApiError
impl From<SyncError> for ApiError {
    fn from(e: SyncError) -> Self {
        match e {
            SyncError::Storage(s) => ApiError(s),
            other => ApiError(StorageError::InvalidState(other.to_string())),
        }
    }
}
```

**What it does** — `SyncError::Storage(s)` keeps its precise mapping (e.g.
NotFound → 404); other variants (Conflict/Failed — transport- or protocol-level
sync failures) become `StorageError::InvalidState` and surface as a 500 with the
underlying message rather than inventing a finer status.

**Used by** — the `sync` handler's `?`.

---

## impl IntoResponse for ApiError

**Identification** — marker `// md:impl IntoResponse for ApiError`.

**Code** — complete and verbatim:

```rust
// md:impl IntoResponse for ApiError
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let code = match &self.0 {
            StorageError::NotFound(_) => StatusCode::NOT_FOUND,
            StorageError::CorruptedData(_) => StatusCode::UNPROCESSABLE_ENTITY,
            StorageError::Conflict(_) => StatusCode::CONFLICT,
            StorageError::InvalidInput(_) => StatusCode::BAD_REQUEST,
            StorageError::TooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (code, Json(json!({ "error": self.0.to_string() }))).into_response()
    }
}
```

**What it does** — The single `StorageError` → HTTP status mapping:

| `StorageError` | HTTP | Why |
|---|---|---|
| `NotFound` | 404 | missing or tombstoned entity |
| `CorruptedData` | 422 | undecryptable/unparsable payload |
| `Conflict` | 409 | duplicate alias — a client conflict, not a server bug |
| `InvalidInput` | 400 | domain-rule rejection (pin an Inbox note, out-of-band sort key, delete the Inbox) |
| `TooLarge` | 413 | a hard format limit was exceeded — a line over 4096 bytes, a note over 65 536 lines, or a notebook already holding 2²⁴ notes (`keeplin_core::format`) |
| everything else | 500 | |

Body is `{"error": "<message>"}`.

---

## Pagination

**Identification** — `#[derive(Debug, Deserialize)] struct Pagination`. Marker
`// md:Pagination`.

**Code** — complete and verbatim:

```rust
// md:Pagination
#[derive(Debug, Deserialize)]
struct Pagination {
    #[serde(default)]
    page_size: u32,
    #[serde(default)]
    page_token: Option<String>,
}
```

**What it does** — `?page_size=&page_token=` for every list endpoint (both
defaulted; `page_size` `0` lets the backend choose).

---

## Page

**Identification** — `#[derive(Debug, Serialize)] struct Page<T>`. Marker
`// md:Page`.

**Code** — complete and verbatim:

```rust
// md:Page
#[derive(Debug, Serialize)]
struct Page<T> {
    items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_page_token: Option<String>,
}
```

**What it does** — A page of results: `items` plus `next_page_token` (omitted
from the JSON when `None` — no more pages).

---

## fn page

**Identification** — `fn page<T>((items, next): (Vec<T>, Option<String>)) ->
Json<Page<T>>`. Marker `// md:fn page`.

**Code** — complete and verbatim:

```rust
// md:fn page
fn page<T>((items, next): (Vec<T>, Option<String>)) -> Json<Page<T>> {
    Json(Page {
        items,
        next_page_token: next,
    })
}
```

**What it does** — Adapter from the backend's `(Vec<T>, Option<String>)` list
result to the JSON page shape; used with `?` in every list handler.

---

## fn health

**Identification** — `async fn health() -> &'static str`. Marker
`// md:fn health`.

**Code** — complete and verbatim:

```rust
// md:fn health
async fn health() -> &'static str {
    "ok"
}
```

**What it does** — Liveness probe: always `200 ok`, never touches the backend —
stays green even if storage is momentarily unavailable (that is what `ready` is
for).

---

## fn ready

**Identification** — `async fn ready(State) -> Response`. Marker
`// md:fn ready`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Readiness probe: one cheap backend read (`list_notes` page
size 1). `200 ready` when storage answers; `503` with the error otherwise, so an
orchestrator stops routing to an instance whose database is locked or
unreachable. (The read flows through the metrics decorator, so a busy readiness
schedule contributes to the `note`/`list` counter.)

---

## fn metrics

**Identification** — `async fn metrics(State) -> Response`. Marker
`// md:fn metrics`.

**Code** — complete and verbatim:

```rust
// md:fn metrics
async fn metrics(State(s): State<Shared>) -> Response {
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4")],
        s.metrics.render_prometheus(),
    )
        .into_response()
}
```

**What it does** — Prometheus exposition (`text/plain; version=0.0.4`) of the
shared registry. Unauthenticated: counters carry only fixed-label aggregates, no
user content.

---

## CreateNote

**Identification** — `#[derive(Debug, Deserialize)] struct CreateNote`. Marker
`// md:CreateNote`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `POST /api/notes` body: `title` (required), `body`,
`notebook_id` (absent → Inbox), `is_todo`, `todo_due` (all defaulted).

---

## fn list_notes

**Identification** — marker `// md:fn list_notes`.

**Code** — complete and verbatim:

```rust
// md:fn list_notes
async fn list_notes(
    State(s): State<Shared>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Note>>, ApiError> {
    Ok(page(s.backend.list_notes(p.page_size, p.page_token).await?))
}
```

**What it does** — `GET /api/notes`: paginated `backend.list_notes`.

---

## fn create_note

**Identification** — marker `// md:fn create_note`.

**Code** — complete and verbatim:

```rust
// md:fn create_note
async fn create_note(
    State(s): State<Shared>,
    Json(req): Json<CreateNote>,
) -> Result<Json<Note>, ApiError> {
    let mut note = Note::new(req.title, req.body);
    note.notebook_id = req.notebook_id.unwrap_or_else(Uuid::nil);
    note.is_todo = req.is_todo;
    note.todo_due = req.todo_due;
    format::check_body(&note.body).map_err(StorageError::from)?;
    ordering::place_new_note(s.backend.as_ref(), &mut note).await?;
    Ok(Json(s.backend.create_note(note).await?))
}
```

**What it does** — `POST /api/notes`: builds `Note::new(title, body)`; absent
`notebook_id` → nil UUID (the Inbox); applies `is_todo`/`todo_due`;
`format::check_body` enforces the hard format limits (≤ 4096 bytes per line,
≤ 65 536 lines) and answers **413** rather than storing an over-sized note; then
`ordering::place_new_note` gives the initial manual position (top of the Inbox,
or the end of a normal notebook's unpinned band) — and, since keeplin#130, also
refuses the note with 413 if the destination notebook already holds
`format::MAX_NOTES_PER_NOTEBOOK` live notes — before `create_note`. Validating
here means the limits hold for a local-only daemon too, not only when the collab
decorator is in the stack.

---

## fn get_note

**Identification** — marker `// md:fn get_note`.

**Code** — complete and verbatim:

```rust
// md:fn get_note
async fn get_note(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Note>, ApiError> {
    let note = s.backend.read_note(id).await?;
    if note.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(Json(note))
}
```

**What it does** — `GET /api/notes/:id`. The backend retains soft-deleted
entities as tombstones (for sync); this surface presents a clean lifecycle, so a
deleted note reads as **404** (checked via `deleted_at`).

---

## fn update_note

**Identification** — marker `// md:fn update_note`.

**Code** — complete and verbatim:

```rust
// md:fn update_note
async fn update_note(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(mut note): Json<Note>,
) -> Result<Json<Note>, ApiError> {
    let stored = read_live_note(&s, id).await?;
    note.id = id;
    format::check_body(&note.body).map_err(StorageError::from)?;
    ordering::reconcile_notebook_move(s.backend.as_ref(), stored.notebook_id, &mut note).await?;
    note.updated_at = now();
    Ok(Json(s.backend.update_note(note).await?))
}
```

**What it does** — `PUT /api/notes/:id` with a full `Note` JSON body. A
tombstoned note is 404 (via `read_live_note`) — otherwise a PUT (whose body
defaults `deleted_at` to null) would silently revive it; revival is reserved for
sync's `apply_change`. The path id overrides the body id. Then
`ordering::reconcile_notebook_move`: moving to a different notebook re-places
the note (its old position and pinned state belonged to the source notebook); a
plain edit keeps its position — and re-places it only if the destination notebook
has room, since `place_new_note` enforces the notes-per-notebook cap.
`format::check_body` runs first, so an over-limit body is a **413** and never
reaches storage. `updated_at = now()` server-side.

---

## fn delete_note

**Identification** — marker `// md:fn delete_note`.

**Code** — complete and verbatim:

```rust
// md:fn delete_note
async fn delete_note(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.backend.delete_note(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

**What it does** — `DELETE /api/notes/:id` → `204` (soft delete).

---

## fn list_note_tags

**Identification** — marker `// md:fn list_note_tags`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `GET /api/notes/:id/tags`: paginated tags on one note.

---

## fn add_note_tag

**Identification** — marker `// md:fn add_note_tag`.

**Code** — complete and verbatim:

```rust
// md:fn add_note_tag
async fn add_note_tag(
    State(s): State<Shared>,
    Path((note_id, tag_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    s.backend.add_note_tag(NoteTag { note_id, tag_id }).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

**What it does** — `PUT /api/notes/:note_id/tags/:tag_id` → `204`
(`backend.add_note_tag`, idempotent at the storage layer).

---

## fn remove_note_tag

**Identification** — marker `// md:fn remove_note_tag`.

**Code** — complete and verbatim:

```rust
// md:fn remove_note_tag
async fn remove_note_tag(
    State(s): State<Shared>,
    Path((note_id, tag_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    s.backend.remove_note_tag(note_id, tag_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

**What it does** — `DELETE /api/notes/:note_id/tags/:tag_id` → `204`.

---

## AliasBody

**Identification** — `#[derive(Debug, Deserialize)] struct AliasBody`. Marker
`// md:AliasBody`.

**Code** — complete and verbatim:

```rust
// md:AliasBody
#[derive(Debug, Deserialize)]
struct AliasBody {
    #[serde(default)]
    alias: Option<String>,
}
```

**What it does** — `{ "alias": "…" | null }` body shared by the two
alias-setting endpoints (`null`/absent clears the alias).

---

## fn read_live_note

**Identification** — `async fn read_live_note(s: &Shared, id: Uuid) ->
Result<Note, ApiError>`. Marker `// md:fn read_live_note`.

**Code** — complete and verbatim:

```rust
// md:fn read_live_note
async fn read_live_note(s: &Shared, id: Uuid) -> Result<Note, ApiError> {
    let note = s.backend.read_note(id).await?;
    if note.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(note)
}
```

**What it does** — Read a live note or 404 for a missing or soft-deleted one
(mirrors `get_note`); the shared tombstone guard for note handlers.

**Used by** — `update_note`, `list_links`.

---

## fn set_note_alias

**Identification** — marker `// md:fn set_note_alias`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `PUT /api/notes/:id/alias`: `linking::set_note_alias`
(uniqueness enforced by the linking layer; duplicate → 409).

---

## fn list_links

**Identification** — marker `// md:fn list_links`.

**Code** — complete and verbatim:

```rust
// md:fn list_links
async fn list_links(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<NoteLink>>, ApiError> {
    Ok(Json(read_live_note(&s, id).await?.links))
}
```

**What it does** — `GET /api/notes/:id/links`: the live note's `links` array
(content-derived + manual).

---

## AddLinkBody

**Identification** — `#[derive(Debug, Deserialize)] struct AddLinkBody`. Marker
`// md:AddLinkBody`.

**Code** — complete and verbatim:

```rust
// md:AddLinkBody
#[derive(Debug, Deserialize)]
struct AddLinkBody {
    raw: String,
}
```

**What it does** — `{ "raw": "#notebook1#note3#5" }` body for adding a manual
(global) link.

---

## fn add_link

**Identification** — marker `// md:fn add_link`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `POST /api/notes/:id/links`: validates the reference syntax
up front with `parse_link_ref` so a bad body is a **422** (`CorruptedData`), not
a 500; then `linking::add_manual_link`.

---

## fn remove_link

**Identification** — marker `// md:fn remove_link`.

**Code** — complete and verbatim:

```rust
// md:fn remove_link
async fn remove_link(
    State(s): State<Shared>,
    Path((id, index)): Path<(Uuid, usize)>,
) -> Result<Json<Note>, ApiError> {
    Ok(Json(
        linking::remove_link(s.backend.as_ref(), id, index).await?,
    ))
}
```

**What it does** — `DELETE /api/notes/:id/links/:index`:
`linking::remove_link` by index into the note's links array.

---

## fn list_backlinks

**Identification** — marker `// md:fn list_backlinks`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `GET /api/notes/:id/backlinks`: paginated
`linking::backlinks` — the notes whose links resolve to this one.

---

## fn list_notes_in_notebook

**Identification** — marker `// md:fn list_notes_in_notebook`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `GET /api/notebooks/:id/notes`: the notebook's notes in
manual order (pinned band first). Use the nil UUID for the Inbox.

---

## fn list_starred_notes

**Identification** — marker `// md:fn list_starred_notes`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `GET /api/notes/starred`: every live starred note, across all
notebooks.

---

## fn pin_note

**Identification** — marker `// md:fn pin_note`.

**Code** — complete and verbatim:

```rust
// md:fn pin_note
async fn pin_note(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Note>, ApiError> {
    Ok(Json(ordering::pin_note(s.backend.as_ref(), id).await?))
}
```

**What it does** — `POST /api/notes/:id/pin`: `ordering::pin_note` — into the
notebook's pinned band (`1..=999`; Inbox notes reject with 400, full band 409).

---

## fn unpin_note

**Identification** — marker `// md:fn unpin_note`.

**Code** — complete and verbatim:

```rust
// md:fn unpin_note
async fn unpin_note(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Note>, ApiError> {
    Ok(Json(ordering::unpin_note(s.backend.as_ref(), id).await?))
}
```

**What it does** — `DELETE /api/notes/:id/pin`: back to the end of the normal
band.

---

## fn star_note

**Identification** — marker `// md:fn star_note`.

**Code** — complete and verbatim:

```rust
// md:fn star_note
async fn star_note(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Note>, ApiError> {
    Ok(Json(ordering::star_note(s.backend.as_ref(), id).await?))
}
```

**What it does** — `POST /api/notes/:id/star`: sets the global star (never moves
the note).

---

## fn unstar_note

**Identification** — marker `// md:fn unstar_note`.

**Code** — complete and verbatim:

```rust
// md:fn unstar_note
async fn unstar_note(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Json<Note>, ApiError> {
    Ok(Json(ordering::unstar_note(s.backend.as_ref(), id).await?))
}
```

**What it does** — `DELETE /api/notes/:id/star`.

---

## ReorderBody

**Identification** — `#[derive(Deserialize)] struct ReorderBody`. Marker
`// md:ReorderBody`.

**Code** — complete and verbatim:

```rust
// md:ReorderBody
#[derive(Deserialize)]
struct ReorderBody {
    sort_key: u32,
}
```

**What it does** — `{ "sort_key": … }` body for the reorder endpoint.

---

## fn reorder_note

**Identification** — marker `// md:fn reorder_note`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `PUT /api/notes/:id/sort-key`: `ordering::reorder_note` — a
new manual position within the note's current band (pinned `1..=999`, normal
`>= 1000`, Inbox `>= 1`); out-of-band keys are 400.

---

## ResolveQuery

**Identification** — `#[derive(Debug, Deserialize)] struct ResolveQuery` with
`#[serde(rename = "ref")]`. Marker `// md:ResolveQuery`.

**Code** — complete and verbatim:

```rust
// md:ResolveQuery
#[derive(Debug, Deserialize)]
struct ResolveQuery {
    #[serde(rename = "ref")]
    reference: String,
}
```

**What it does** — `?ref=#notebook1#note3#5` query for reference resolution.

---

## fn resolve_reference

**Identification** — marker `// md:fn resolve_reference`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `GET /api/links/resolve?ref=…`: `linking::resolve` → JSON
`{note_id, bookmark_number}`; an unresolvable reference is `{null, null}`, not
an error.

---

## fn list_alias_conflicts

**Identification** — marker `// md:fn list_alias_conflicts`.

**Code** — complete and verbatim:

```rust
// md:fn list_alias_conflicts
async fn list_alias_conflicts(
    State(s): State<Shared>,
) -> Result<Json<linking::AliasConflicts>, ApiError> {
    Ok(Json(linking::alias_conflicts(s.backend.as_ref()).await?))
}
```

**What it does** — `GET /api/aliases/conflicts`: note/notebook aliases shared by
two or more live entities (the residue of a cross-device alias collision), so a
human can rename one side. Serialises `linking::AliasConflicts` directly.

---

## HistoryQuery

**Identification** — `#[derive(Debug, Default, Deserialize)] struct
HistoryQuery`. Marker `// md:HistoryQuery`.

**Code** — complete and verbatim:

```rust
// md:HistoryQuery
#[derive(Debug, Default, Deserialize)]
struct HistoryQuery {
    #[serde(default)]
    limit: u32,
}
```

**What it does** — `?limit=` for the history endpoints; `0` (absent) uses the
backend's default cap.

---

## NoteVersion

**Identification** — `#[derive(Debug, Serialize)] struct NoteVersion`. Marker
`// md:NoteVersion`.

**Code** — complete and verbatim:

```rust
// md:NoteVersion
#[derive(Debug, Serialize)]
struct NoteVersion {
    timestamp: DateTime<Utc>,
    device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<Note>,
}
```

**What it does** — One past version of a note (`timestamp`, `device_id`,
optional `note` — absent when the version is a tombstone).

---

## NotebookVersion

**Identification** — `#[derive(Debug, Serialize)] struct NotebookVersion`.
Marker `// md:NotebookVersion`.

**Code** — complete and verbatim:

```rust
// md:NotebookVersion
#[derive(Debug, Serialize)]
struct NotebookVersion {
    timestamp: DateTime<Utc>,
    device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    notebook: Option<Notebook>,
}
```

**What it does** — The notebook counterpart of `NoteVersion`.

---

## RevertBody

**Identification** — `#[derive(Debug, Deserialize)] struct RevertBody`. Marker
`// md:RevertBody`.

**Code** — complete and verbatim:

```rust
// md:RevertBody
#[derive(Debug, Deserialize)]
struct RevertBody {
    at: DateTime<Utc>,
}
```

**What it does** — `{ "at": "<RFC-3339>" }` — the instant to roll an entity back
to (the newest version at or before it).

---

## BatchRevertBody

**Identification** — `#[derive(Debug, Deserialize)] struct BatchRevertBody`.
Marker `// md:BatchRevertBody`.

**Code** — complete and verbatim:

```rust
// md:BatchRevertBody
#[derive(Debug, Deserialize)]
struct BatchRevertBody {
    at: DateTime<Utc>,
    #[serde(default)]
    note_ids: Vec<Uuid>,
}
```

**What it does** — `{ "at": …, "note_ids": [ … ] }` — batch forward-revert of
the listed notes.

---

## fn note_history

**Identification** — marker `// md:fn note_history`.

**Code** — complete and verbatim:

```rust
// md:fn note_history
async fn note_history(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<NoteVersion>>, ApiError> {
    let versions = s.backend.note_history(id, q.limit).await?;
    Ok(Json(versions.into_iter().map(note_version_dto).collect()))
}
```

**What it does** — `GET /api/notes/:id/history`: `backend.note_history`, newest
first, mapped through `note_version_dto`.

---

## fn notebook_history

**Identification** — marker `// md:fn notebook_history`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `GET /api/notebooks/:id/history`, mapped through
`notebook_version_dto`.

---

## fn revert_note_ep

**Identification** — marker `// md:fn revert_note_ep`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `POST /api/notes/:id/revert`: `history::revert_note` — a
**forward** revert (writes the old state as a new version; non-destructive).

---

## fn revert_notebook_ep

**Identification** — marker `// md:fn revert_notebook_ep`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `POST /api/notebooks/:id/revert`:
`history::revert_notebook`.

---

## fn revert_notebook_notes_ep

**Identification** — marker `// md:fn revert_notebook_notes_ep`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `POST /api/notebooks/:id/notes/revert`:
`history::revert_notebook_notes_to` — batch-revert every note currently in the
notebook to its state as of `at` (the roll-back companion to a destructive
notebook-wide change).

---

## fn batch_revert_notes_ep

**Identification** — marker `// md:fn batch_revert_notes_ep`.

**Code** — complete and verbatim:

```rust
// md:fn batch_revert_notes_ep
async fn batch_revert_notes_ep(
    State(s): State<Shared>,
    Json(b): Json<BatchRevertBody>,
) -> Result<Json<Vec<Note>>, ApiError> {
    Ok(Json(
        history::revert_notes_to(s.backend.as_ref(), &b.note_ids, b.at).await?,
    ))
}
```

**What it does** — `POST /api/history/revert`: `history::revert_notes_to` over
an explicit note-id list.

---

## fn note_version_dto

**Identification** — `fn note_version_dto(v: EntityVersion<Note>) ->
NoteVersion`. Marker `// md:fn note_version_dto`.

**Code** — complete and verbatim:

```rust
// md:fn note_version_dto
fn note_version_dto(v: EntityVersion<Note>) -> NoteVersion {
    NoteVersion {
        timestamp: v.timestamp,
        device_id: v.device_id,
        note: v.entity,
    }
}
```

**What it does** — Field map from the storage-layer version record.

---

## fn notebook_version_dto

**Identification** — `fn notebook_version_dto(v: EntityVersion<Notebook>) ->
NotebookVersion`. Marker `// md:fn notebook_version_dto`.

**Code** — complete and verbatim:

```rust
// md:fn notebook_version_dto
fn notebook_version_dto(v: EntityVersion<Notebook>) -> NotebookVersion {
    NotebookVersion {
        timestamp: v.timestamp,
        device_id: v.device_id,
        notebook: v.entity,
    }
}
```

**What it does** — Notebook counterpart of `note_version_dto`.

---

## fn proxy_perm

**Identification** — `async fn proxy_perm(s: &Shared, method: &str, path:
String, body: Option<serde_json::Value>) -> Response`. Marker
`// md:fn proxy_perm`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The shared permission-proxy core. Permissions are enforced
**server-side** (keeplin-srv is the authority); the daemon does not store or
enforce them — it forwards these requests over the collab channel's
authenticated REST client (`collab.proxy_request`) and relays the response
`(status, body)`, so a frontend can view/manage shares on demand. In fs/offline
mode (`collab: None`) → `503`; a transport failure or unmappable status →
`502 Bad Gateway`.

**Used by** — the eight `proxy_*` handlers below.

---

## fn proxy_note_share

**Identification** — marker `// md:fn proxy_note_share`.

**Code** — complete and verbatim:

```rust
// md:fn proxy_note_share
async fn proxy_note_share(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    proxy_perm(&s, "POST", format!("/api/notes/{id}/share"), Some(body)).await
}
```

**What it does** — `POST /api/notes/:id/share` → forwards body to the server's
same path.

---

## fn proxy_note_shares

**Identification** — marker `// md:fn proxy_note_shares`.

**Code** — complete and verbatim:

```rust
// md:fn proxy_note_shares
async fn proxy_note_shares(State(s): State<Shared>, Path(id): Path<Uuid>) -> Response {
    proxy_perm(&s, "GET", format!("/api/notes/{id}/share"), None).await
}
```

**What it does** — `GET /api/notes/:id/share` → forwarded.

---

## fn proxy_note_unshare

**Identification** — marker `// md:fn proxy_note_unshare`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `DELETE /api/notes/:id/share/:user_id` → forwarded.

---

## fn proxy_note_transfer

**Identification** — marker `// md:fn proxy_note_transfer`.

**Code** — complete and verbatim:

```rust
// md:fn proxy_note_transfer
async fn proxy_note_transfer(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    proxy_perm(&s, "POST", format!("/api/notes/{id}/transfer"), Some(body)).await
}
```

**What it does** — `POST /api/notes/:id/transfer` → forwarded with body.

---

## fn proxy_notebook_share

**Identification** — marker `// md:fn proxy_notebook_share`.

**Code** — complete and verbatim:

```rust
// md:fn proxy_notebook_share
async fn proxy_notebook_share(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    proxy_perm(&s, "POST", format!("/api/notebooks/{id}/share"), Some(body)).await
}
```

**What it does** — `POST /api/notebooks/:id/share` → forwarded with body.

---

## fn proxy_notebook_shares

**Identification** — marker `// md:fn proxy_notebook_shares`.

**Code** — complete and verbatim:

```rust
// md:fn proxy_notebook_shares
async fn proxy_notebook_shares(State(s): State<Shared>, Path(id): Path<Uuid>) -> Response {
    proxy_perm(&s, "GET", format!("/api/notebooks/{id}/share"), None).await
}
```

**What it does** — `GET /api/notebooks/:id/share` → forwarded.

---

## fn proxy_notebook_unshare

**Identification** — marker `// md:fn proxy_notebook_unshare`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `DELETE /api/notebooks/:id/share/:user_id` → forwarded.

---

## fn proxy_notebook_transfer

**Identification** — marker `// md:fn proxy_notebook_transfer`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `POST /api/notebooks/:id/transfer` → forwarded with body.

---

## ContactDto

**Identification** — `#[derive(Debug, Serialize)] struct ContactDto`. Marker
`// md:ContactDto`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — JSON view of `keeplin_core::interop::Contact` (`uid`,
`formatted_name`, optional name parts, `emails`, `phones`, optional
`org`/`note`; `None`s omitted).

---

## impl From Contact for ContactDto

**Identification** — marker `// md:impl From Contact for ContactDto`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Field-for-field map.

---

## EventDto

**Identification** — `#[derive(Debug, Serialize)] struct EventDto`. Marker
`// md:EventDto`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — JSON view of a calendar event (`uid`, `summary`, optional
`start`/`end`/`location`/`description`).

---

## impl From CalendarEvent for EventDto

**Identification** — marker `// md:impl From CalendarEvent for EventDto`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Field-for-field map.

---

## fn text_body

**Identification** — `fn text_body(mime: &'static str, body: String) ->
Response`. Marker `// md:fn text_body`.

**Code** — complete and verbatim:

```rust
// md:fn text_body
fn text_body(mime: &'static str, body: String) -> Response {
    ([(CONTENT_TYPE, mime)], body).into_response()
}
```

**What it does** — Builds a raw `text/vcard` or `text/calendar` body response.

**Used by** — `export_contact_ep`, `export_event_ep`, `profile_vcard_ep`.

---

## fn list_contacts_ep

**Identification** — marker `// md:fn list_contacts_ep`.

**Code** — complete and verbatim:

```rust
// md:fn list_contacts_ep
async fn list_contacts_ep(State(s): State<Shared>) -> Result<Json<Vec<ContactDto>>, ApiError> {
    let contacts = interop::list_contacts(s.backend.as_ref()).await?;
    Ok(Json(contacts.into_iter().map(ContactDto::from).collect()))
}
```

**What it does** — `GET /api/contacts`: `interop::list_contacts` → `ContactDto`
list.

---

## fn import_contact_ep

**Identification** — marker `// md:fn import_contact_ep`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `POST /api/contacts/import`: the raw body is a vCard;
`Contact::from_vcard` (invalid → 400 `InvalidInput`), stored with
`interop::save_contact`, stored contact returned.

---

## fn export_contact_ep

**Identification** — marker `// md:fn export_contact_ep`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `GET /api/contacts/:uid/export`: the contact as a
`text/vcard` body (`interop::MIME_VCARD`); unknown uid → 404.

---

## fn delete_contact_ep

**Identification** — marker `// md:fn delete_contact_ep`.

**Code** — complete and verbatim:

```rust
// md:fn delete_contact_ep
async fn delete_contact_ep(
    State(s): State<Shared>,
    Path(uid): Path<String>,
) -> Result<StatusCode, ApiError> {
    interop::delete_contact(s.backend.as_ref(), &uid).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

**What it does** — `DELETE /api/contacts/:uid` → `204`.

---

## fn list_events_ep

**Identification** — marker `// md:fn list_events_ep`.

**Code** — complete and verbatim:

```rust
// md:fn list_events_ep
async fn list_events_ep(State(s): State<Shared>) -> Result<Json<Vec<EventDto>>, ApiError> {
    let events = interop::list_events(s.backend.as_ref()).await?;
    Ok(Json(events.into_iter().map(EventDto::from).collect()))
}
```

**What it does** — `GET /api/events`: `interop::list_events` → `EventDto` list.

---

## fn import_event_ep

**Identification** — marker `// md:fn import_event_ep`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `POST /api/events/import`: the raw body is an iCalendar file;
**every** `VEVENT` is parsed (`CalendarEvent::from_ics_all`) and stored, and the
stored events are returned in document order — a whole exported calendar imports
in one call. No `VEVENT` at all → 400.

---

## fn export_event_ep

**Identification** — marker `// md:fn export_event_ep`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `GET /api/events/:uid/export`: the event as a
`text/calendar` body (`interop::MIME_ICALENDAR`); unknown uid → 404.

---

## fn delete_event_ep

**Identification** — marker `// md:fn delete_event_ep`.

**Code** — complete and verbatim:

```rust
// md:fn delete_event_ep
async fn delete_event_ep(
    State(s): State<Shared>,
    Path(uid): Path<String>,
) -> Result<StatusCode, ApiError> {
    interop::delete_event(s.backend.as_ref(), &uid).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

**What it does** — `DELETE /api/events/:uid` → `204`.

---

## fn import_todo_ep

**Identification** — marker `// md:fn import_todo_ep`.

**Code** — complete and verbatim:

```rust
// md:fn import_todo_ep
async fn import_todo_ep(
    State(s): State<Shared>,
    body: String,
) -> Result<Json<Vec<Note>>, ApiError> {
    Ok(Json(
        interop::import_todos(s.backend.as_ref(), &body).await?,
    ))
}
```

**What it does** — `POST /api/todos/import`: a Keeplin to-do note is created
from **every** `VTODO` in the iCalendar body (`interop::import_todos`), returned
in document order.

---

## ProfileVcardQuery

**Identification** — `#[derive(Debug, Deserialize)] struct ProfileVcardQuery`.
Marker `// md:ProfileVcardQuery`.

**Code** — complete and verbatim:

```rust
// md:ProfileVcardQuery
#[derive(Debug, Deserialize)]
struct ProfileVcardQuery {
    #[serde(default)]
    name: Option<String>,
    email: String,
}
```

**What it does** — `?name=&email=` (email required) for the profile-vCard
endpoint.

---

## fn profile_vcard_ep

**Identification** — marker `// md:fn profile_vcard_ep`.

**Code** — complete and verbatim:

```rust
// md:fn profile_vcard_ep
async fn profile_vcard_ep(Query(q): Query<ProfileVcardQuery>) -> Response {
    let name = q
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| q.email.split('@').next().unwrap_or("").to_string());
    text_body(interop::MIME_VCARD, interop::user_vcard(&name, &q.email))
}
```

**What it does** — `GET /api/profile/vcard?email=&name=`: renders the account
owner's profile vCard (`interop::user_vcard`). The caller supplies the profile —
the daemon does not own user identity; a blank `name` defaults to the email's
local part.

---

## TitleOnly

**Identification** — `#[derive(Debug, Deserialize)] struct TitleOnly`. Marker
`// md:TitleOnly`.

**Code** — complete and verbatim:

```rust
// md:TitleOnly
#[derive(Debug, Deserialize)]
struct TitleOnly {
    title: String,
}
```

**What it does** — `{ "title": … }` body shared by notebook and tag creation.

---

## fn list_notebooks

**Identification** — marker `// md:fn list_notebooks`.

**Code** — complete and verbatim:

```rust
// md:fn list_notebooks
async fn list_notebooks(
    State(s): State<Shared>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Notebook>>, ApiError> {
    Ok(page(
        s.backend.list_notebooks(p.page_size, p.page_token).await?,
    ))
}
```

**What it does** — `GET /api/notebooks`: paginated.

---

## fn create_notebook

**Identification** — marker `// md:fn create_notebook`.

**Code** — complete and verbatim:

```rust
// md:fn create_notebook
async fn create_notebook(
    State(s): State<Shared>,
    Json(req): Json<TitleOnly>,
) -> Result<Json<Notebook>, ApiError> {
    Ok(Json(
        s.backend.create_notebook(Notebook::new(req.title)).await?,
    ))
}
```

**What it does** — `POST /api/notebooks`: `Notebook::new(title)`.

---

## fn read_live_notebook

**Identification** — `async fn read_live_notebook(s: &Shared, id: Uuid) ->
Result<Notebook, ApiError>`. Marker `// md:fn read_live_notebook`.

**Code** — complete and verbatim:

```rust
// md:fn read_live_notebook
async fn read_live_notebook(s: &Shared, id: Uuid) -> Result<Notebook, ApiError> {
    let nb = s.backend.read_notebook(id).await?;
    if nb.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(nb)
}
```

**What it does** — Live-or-404 tombstone guard for notebooks.

**Used by** — `get_notebook`, `update_notebook`.

---

## fn get_notebook

**Identification** — marker `// md:fn get_notebook`.

**Code** — complete and verbatim:

```rust
// md:fn get_notebook
async fn get_notebook(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Json<Notebook>, ApiError> {
    Ok(Json(read_live_notebook(&s, id).await?))
}
```

**What it does** — `GET /api/notebooks/:id` via the live guard (tombstone →
404).

---

## fn update_notebook

**Identification** — marker `// md:fn update_notebook`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `PUT /api/notebooks/:id`: tombstone → 404 (no revival); path
id wins; `updated_at = now()` server-side.

---

## fn delete_notebook

**Identification** — marker `// md:fn delete_notebook`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `DELETE /api/notebooks/:id`: **the Inbox system notebook
(nil UUID) cannot be deleted** (`ordering::is_inbox` → 400); otherwise soft
delete → `204`.

---

## fn set_notebook_alias

**Identification** — marker `// md:fn set_notebook_alias`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `PUT /api/notebooks/:id/alias`:
`linking::set_notebook_alias`.

---

## fn list_tags

**Identification** — marker `// md:fn list_tags`.

**Code** — complete and verbatim:

```rust
// md:fn list_tags
async fn list_tags(
    State(s): State<Shared>,
    Query(p): Query<Pagination>,
) -> Result<Json<Page<Tag>>, ApiError> {
    Ok(page(s.backend.list_tags(p.page_size, p.page_token).await?))
}
```

**What it does** — `GET /api/tags`: paginated.

---

## fn create_tag

**Identification** — marker `// md:fn create_tag`.

**Code** — complete and verbatim:

```rust
// md:fn create_tag
async fn create_tag(
    State(s): State<Shared>,
    Json(req): Json<TitleOnly>,
) -> Result<Json<Tag>, ApiError> {
    Ok(Json(s.backend.create_tag(Tag::new(req.title)).await?))
}
```

**What it does** — `POST /api/tags`: `Tag::new(title)`.

---

## fn read_live_tag

**Identification** — `async fn read_live_tag(s: &Shared, id: Uuid) ->
Result<Tag, ApiError>`. Marker `// md:fn read_live_tag`.

**Code** — complete and verbatim:

```rust
// md:fn read_live_tag
async fn read_live_tag(s: &Shared, id: Uuid) -> Result<Tag, ApiError> {
    let tag = s.backend.read_tag(id).await?;
    if tag.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()).into());
    }
    Ok(tag)
}
```

**What it does** — Live-or-404 tombstone guard for tags.

**Used by** — `get_tag`, `update_tag`.

---

## fn get_tag

**Identification** — marker `// md:fn get_tag`.

**Code** — complete and verbatim:

```rust
// md:fn get_tag
async fn get_tag(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<Json<Tag>, ApiError> {
    Ok(Json(read_live_tag(&s, id).await?))
}
```

**What it does** — `GET /api/tags/:id` via the live guard.

---

## fn update_tag

**Identification** — marker `// md:fn update_tag`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `PUT /api/tags/:id`: tombstone → 404; path id wins;
`updated_at = now()` server-side.

---

## fn delete_tag

**Identification** — marker `// md:fn delete_tag`.

**Code** — complete and verbatim:

```rust
// md:fn delete_tag
async fn delete_tag(State(s): State<Shared>, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    s.backend.delete_tag(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

**What it does** — `DELETE /api/tags/:id` → `204`.

---

## ResourceMeta

**Identification** — `#[derive(Debug, Deserialize)] struct ResourceMeta`. Marker
`// md:ResourceMeta`.

**Code** — complete and verbatim:

```rust
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
    #[serde(default)]
    note_id: Option<Uuid>,
}
```

**What it does** — `?title=&file_name=&note_id=` query metadata for the two upload
routes. `note_id` is `Option` at parse time but **required by the handlers** (a missing
`note_id` is a `400`, via `StorageError::InvalidInput`) — every attachment must name its
owning note (issue #125).

---

## ListResourcesQuery

**Identification** — `#[derive(Debug, Deserialize)] struct ListResourcesQuery`. Marker
`// md:ListResourcesQuery`.

**Code** — complete and verbatim:

```rust
// md:ListResourcesQuery
#[derive(Debug, Deserialize)]
struct ListResourcesQuery {
    #[serde(default)]
    page_size: u32,
    #[serde(default)]
    page_token: Option<String>,
    #[serde(default)]
    note_id: Option<Uuid>,
}
```

**What it does** — Query for `GET /api/resources`: the pagination fields (mirroring
`Pagination`) plus an **optional** `note_id` filter. When `note_id` is present the handler
delegates to `list_resources_for_note`; when absent it lists every resource. Explicit fields
(rather than `#[serde(flatten)]` of `Pagination`) because `serde_urlencoded` does not support
flattening.

**Dependencies** —
- `serde` derive — urlencoded query parsing; expects `#[serde(default)]` so each field is
  optional in the query string.

**Used by** — `list_resources`.

**Repeated context** — none.

---

## fn list_resources

**Identification** — marker `// md:fn list_resources`.

**Code** — complete and verbatim:

```rust
// md:fn list_resources
async fn list_resources(
    State(s): State<Shared>,
    Query(p): Query<ListResourcesQuery>,
) -> Result<Json<Page<Resource>>, ApiError> {
    let listed = match p.note_id {
        Some(note_id) => {
            s.backend
                .list_resources_for_note(note_id, p.page_size, p.page_token)
                .await?
        }
        None => s.backend.list_resources(p.page_size, p.page_token).await?,
    };
    Ok(page(listed))
}
```

**What it does** — `GET /api/resources`: paginated metadata. With `?note_id=<uuid>` it returns
just that note's attachments (via `list_resources_for_note`); without it, all resources.

---

## fn create_resource

**Identification** — marker `// md:fn create_resource`.

**Code** — complete and verbatim:

```rust
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
    let note_id = meta
        .note_id
        .ok_or_else(|| StorageError::InvalidInput("note_id is required".into()))?;
    let data = body.to_vec();
    let mut resource = Resource::new(note_id, meta.title, mime, meta.file_name, data.len() as u64);
    resource.duration_ms = meta.duration_ms;
    resource.dimensions = match (meta.width, meta.height) {
        (Some(w), Some(h)) => Some((w, h)),
        _ => None,
    };
    Ok(Json(s.backend.create_resource(resource, data).await?))
}
```

**What it does** — `POST /api/resources?title=&file_name=&note_id=`: the raw request body
is the payload (`Bytes`, bounded by the router's `max_body_bytes` layer), the
`Content-Type` header is recorded as the MIME type
(`application/octet-stream` default). `note_id` is **required** — a missing one is a `400`
(`StorageError::InvalidInput`) — and becomes the attachment's owning note.

---

## fn upload_resource

**Identification** — `async fn upload_resource(State, Query<ResourceMeta>,
HeaderMap, Body) -> Response`. Marker `// md:fn upload_resource`.

**Code** — complete and verbatim:

```rust
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
    let note_id = match meta.note_id {
        Some(id) => id,
        None => {
            return ApiError(StorageError::InvalidInput("note_id is required".into()))
                .into_response()
        }
    };
    let mut resource = Resource::new(note_id, meta.title, mime, meta.file_name, data.len() as u64);
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
```

**What it does** — `POST /api/resources/upload?title=&file_name=`, the
**streaming** upload: this route disables the router-wide body limit and reads
the body incrementally with `axum::body::to_bytes(body, limit)` where `limit` is
`max_upload_bytes` (`0` → `usize::MAX`) — `to_bytes` errors once past the limit,
so an oversized upload never fully materialises in memory; over the cap → `413`
with a JSON error. Then stores like `create_resource`.

---

## fn get_resource

**Identification** — marker `// md:fn get_resource`.

**Code** — complete and verbatim:

```rust
// md:fn get_resource
async fn get_resource(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Json<Resource>, ApiError> {
    let (meta, _data) = s.backend.read_resource(id).await?;
    Ok(Json(meta))
}
```

**What it does** — `GET /api/resources/:id`: metadata only (the payload read is
discarded).

---

## fn get_resource_data

**Identification** — marker `// md:fn get_resource_data`.

**Code** — complete and verbatim:

```rust
// md:fn get_resource_data
async fn get_resource_data(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let (meta, data) = s.backend.read_resource(id).await?;
    Ok(([(CONTENT_TYPE, meta.mime_type)], data).into_response())
}
```

**What it does** — `GET /api/resources/:id/data`: the raw bytes, served with the
stored MIME type.

---

## fn delete_resource

**Identification** — marker `// md:fn delete_resource`.

**Code** — complete and verbatim:

```rust
// md:fn delete_resource
async fn delete_resource(
    State(s): State<Shared>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.backend.delete_resource(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

**What it does** — `DELETE /api/resources/:id` → `204` (payload reclaimed later
by the post-sync purge).

---

## SyncSummary

**Identification** — `#[derive(Debug, Serialize)] struct SyncSummary`. Marker
`// md:SyncSummary`.

**Code** — complete and verbatim:

```rust
// md:SyncSummary
#[derive(Debug, Serialize)]
struct SyncSummary {
    applied: usize,
}
```

**What it does** — `{ "applied": n }` — how many remote changes were applied.

---

## fn sync

**Identification** — `async fn sync(State) -> Result<Json<SyncSummary>,
ApiError>`. Marker `// md:fn sync`.

**Code** — complete and verbatim:

```rust
// md:fn sync
async fn sync(State(s): State<Shared>) -> Result<Json<SyncSummary>, ApiError> {
    let applied = run_sync(s.backend.as_ref(), |_stage, _count| {}).await?;
    crate::server::prune_journal_after_sync(s.backend.as_ref(), s.journal_retention_days).await;
    crate::server::purge_resources_after_sync(s.backend.as_ref(), s.resource_purge_days).await;
    Ok(Json(SyncSummary {
        applied: applied.len(),
    }))
}
```

**What it does** — `POST /api/sync`: one `keeplin_core::sync::run_sync` cycle on
the shared backend (no-op progress callback), then
`crate::server::prune_journal_after_sync` and
`crate::server::purge_resources_after_sync` — the same maintenance the gRPC
`Sync` RPC runs, so retention/purge settings are honoured no matter which
surface drives the sync. The backend is `&dyn StorageBackend`; `run_sync`
accepts it because `dyn StorageBackend` itself satisfies the trait (the `?Sized`
blanket impl in keeplin-core).

---

## fn ws_handler

**Identification** — `async fn ws_handler(State, WebSocketUpgrade) -> Response`.
Marker `// md:fn ws_handler`.

**Code** — complete and verbatim:

```rust
// md:fn ws_handler
async fn ws_handler(State(s): State<Shared>, ws: WebSocketUpgrade) -> Response {
    let rx = s.events.subscribe();
    ws.on_upgrade(move |socket| stream_changes(socket, rx))
}
```

**What it does** — `GET /api/ws`: subscribes a fresh broadcast receiver
**before** the upgrade response is sent (so no event created after the connect
can be missed) and upgrades to a WebSocket driven by `stream_changes`. The
upgrade request passes through the same Basic-Auth middleware as the REST
routes.

---

## fn stream_changes

**Identification** — `async fn stream_changes(mut socket: WebSocket, mut rx:
broadcast::Receiver<Change>)`. Marker `// md:fn stream_changes`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Forwards broadcast changes to one client until it disconnects
or the channel closes. Each `Change` is serialised to a JSON text frame
(serialisation failure sends `{"type":"error"}`). On
`RecvError::Lagged` (the client fell behind the channel capacity) it sends a
`{"type":"resync"}` hint so the client can reload state rather than silently
miss events; `Closed` ends the loop. A `tokio::select!` also drives the receive
side so client pings get pongs and a close frame ends the loop promptly instead
of waiting for the next failed send; data/ping/pong frames from the client are
ignored.

---

## mod tests

**Identification** — `#[cfg(test)] mod tests`. Marker `// md:mod tests`. Uses
`tower::ServiceExt` for `oneshot` router calls, plus (mid-module) the
WebSocket-test imports (`EventBackend`, `futures_util::StreamExt`,
`tokio_tungstenite`).

**Code** — container: members documented as sub-blocks below: fn state, fn linking_state, fn call, fn note_crud_round_trip, fn permission_endpoints_require_server_mode, fn contact_import_list_export_delete_endpoints, fn todo_import_and_profile_vcard_endpoints, fn note_history_and_revert_endpoints, fn updates_on_deleted_entities_are_404, fn sync_endpoint_prunes_journal_within_retention, fn operational_endpoints_bypass_auth, fn metrics_state, fn metrics_reflect_operations_and_http_status, fn invalid_uuid_is_bad_request, fn auth_is_enforced_when_configured, fn resource_upload_and_download, fn resource_upload_above_axum_default_limit, fn streaming_upload_round_trips, fn streaming_upload_over_cap_is_413, fn alias_and_links_endpoints, fn alias_backlinks_and_resolve_endpoints, fn alias_conflicts_endpoint, fn state_with_events, fn websocket_streams_note_create.

The explicit `imports` leaf below preserves the test-module dependency preamble
verbatim.

### imports

**Identification** — test-module dependencies; marker `// md:mod tests > imports`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > imports
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use keeplin_core::storage::fs::FsBackend;
    use tower::ServiceExt;
```

**What it does** — Brings the parent module API and test-only dependencies into
scope.

**Dependencies** —

- `super::*` — the items under test from the parent module; expects: the parent keeps them at module scope; a rename or a move into a submodule breaks these tests at compile time, which is the intended early signal.
- `axum::body::Body` — builds the request bodies driven through the router; expects: it stays the body type the router is generic over.
- `axum::http::Request` — builds the requests driven through the router; expects: header and URI construction stay infallible for the fixed inputs used here.
- `base64::{engine::general_purpose::STANDARD, Engine}` — encodes and decodes the payloads exchanged with the daemon; expects: `STANDARD` stays the padded alphabet the rest of the code uses; an unpadded engine would round-trip here and fail in production.
- `keeplin_core::storage::fs::FsBackend` — a real filesystem-backed store built over a temporary directory; expects: it honours the same repository traits as production, so what passes here says something about the real backend.
- `tower::ServiceExt` — provides `oneshot`, which drives the router without binding a socket; expects: `oneshot` keeps consuming the service and returning the full response, so the tests exercise the real routing stack rather than a handler call.

**Used by** — every block of `mod tests` in this file: `fn state`, `fn linking_state`, `fn call`, `fn note_crud_round_trip`, `fn permission_endpoints_require_server_mode`, `fn contact_import_list_export_delete_endpoints`, `fn todo_import_and_profile_vcard_endpoints`, `fn note_history_and_revert_endpoints`, `fn updates_on_deleted_entities_are_404`, `fn sync_endpoint_prunes_journal_within_retention`, `fn operational_endpoints_bypass_auth`, `fn metrics_state`, `fn metrics_reflect_operations_and_http_status`, `fn invalid_uuid_is_bad_request`, `fn auth_is_enforced_when_configured`, `fn resource_upload_and_download`, `fn resource_upload_above_axum_default_limit`, `fn streaming_upload_round_trips`, `fn streaming_upload_over_cap_is_413`, `fn alias_and_links_endpoints`, `fn alias_backlinks_and_resolve_endpoints`, `fn alias_conflicts_endpoint`, `fn state_with_events`, `fn websocket_streams_note_create`. Nothing outside the module can use it: the preamble is private to `mod tests`.

**Repeated context** — This preamble is a leaf block, not scaffolding: only the `mod` declaration, its attributes and its braces are exempt from coverage, so these `use` lines carry their own marker and are verified verbatim against the source (template v2.5.0, RULE 6). Changing an import here without updating this fence fails `scripts/check-docs.sh`.

### fn state

**Identification** — helper; marker `// md:mod tests > fn state`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — An `AppState` over a fresh `FsBackend` in a leaked temp dir,
optionally with Basic-Auth credentials; generous body/upload limits, 30-day
journal retention.

### fn linking_state

**Identification** — helper; marker `// md:mod tests > fn linking_state`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Like `state` but wraps the backend in `LinkingBackend`, so
writes derive bookmarks/links and resolve references — required by the
bookmark/link endpoint tests.

### fn call

**Identification** — helper; marker `// md:mod tests > fn call`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — One request against a fresh `router` over the shared state
via `oneshot`; returns `(status, body bytes)`. Sets `Content-Type:
application/json` when a body is given and an `Authorization` header when
provided.

### fn note_crud_round_trip

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn note_crud_round_trip`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — POST → GET → list (1 item) → DELETE (204) → GET is 404.

### fn permission_endpoints_require_server_mode

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn permission_endpoints_require_server_mode`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — With `collab: None`, share/list-shares/transfer all answer
`503`, not a panic.

### fn contact_import_list_export_delete_endpoints

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn contact_import_list_export_delete_endpoints`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — vCard import round-trips (name/uid), lists once, exports
containing `FN:Ada`, deletes to an empty list.

### fn todo_import_and_profile_vcard_endpoints

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn todo_import_and_profile_vcard_endpoints`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Two `VTODO`s in one calendar import as two to-do notes in
document order; `/api/profile/vcard?email=me@x.com` defaults the name to the
email local part.

### fn note_history_and_revert_endpoints

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn note_history_and_revert_endpoints`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Create + edit → history has two versions newest-first;
revert to the first instant restores `v1` and adds a **third** version
(non-destructive forward revert).

### fn updates_on_deleted_entities_are_404

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn updates_on_deleted_entities_are_404`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — For note (including the alias PUT), notebook, and tag:
create → delete → PUT is 404, no silent revival; the note stays deleted.

### fn sync_endpoint_prunes_journal_within_retention

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn sync_endpoint_prunes_journal_within_retention`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — On a `DbBackend` state (empty `server_url` → local-only sync;
exercises the prune path `FsBackend` no-ops): `POST /api/sync` applies 0 remote
changes and fresh journal rows survive a 30-day retention window (the prune ran
and respected the window).

### fn operational_endpoints_bypass_auth

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn operational_endpoints_bypass_auth`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — With auth configured, `/api/health`, `/api/ready`,
`/api/metrics` remain reachable without credentials while `/api/notes` is 401.

### fn metrics_state

**Identification** — helper; marker `// md:mod tests > fn metrics_state`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — An `AppState` whose backend is a `MetricsBackend` over
`FsBackend` sharing the state's own `Arc<Metrics>` — mirroring `main`'s wiring —
so router operations move the same counters `GET /api/metrics` renders.

### fn metrics_reflect_operations_and_http_status

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn metrics_reflect_operations_and_http_status`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — One successful create + one 404 GET → the exposition shows
`keeplin_storage_operations_total{entity="note",op="create"} 1`, one `2xx`, one
`4xx`; the `/metrics` scrape itself is not counted (operational routes bypass
the status middleware).

### fn invalid_uuid_is_bad_request

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn invalid_uuid_is_bad_request`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn invalid_uuid_is_bad_request
    #[tokio::test]
    async fn invalid_uuid_is_bad_request() {
        let st = state(None).await;
        let (code, _) = call(&st, "GET", "/api/notes/not-a-uuid", None, None).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }
```

**What it does** — `GET /api/notes/not-a-uuid` → 400 (axum path-extractor
rejection).

### fn auth_is_enforced_when_configured

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn auth_is_enforced_when_configured`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Missing credentials → 401; wrong password → 401; valid →
200.

### fn resource_upload_and_download

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn resource_upload_and_download`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn resource_upload_and_download
    #[tokio::test]
    async fn resource_upload_and_download() {
        let st = state(None).await;
        let (code, body) = call(
            &st,
            "POST",
            "/api/resources?title=pic&file_name=p.png&note_id=00000000-0000-0000-0000-000000000001",
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
```

**What it does** — Raw-body upload with query metadata round-trips; the
`/data` download returns the exact bytes.

### fn resource_upload_above_axum_default_limit

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn resource_upload_above_axum_default_limit`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn resource_upload_above_axum_default_limit
    #[tokio::test]
    async fn resource_upload_above_axum_default_limit() {
        let st = state(None).await;
        let big = "x".repeat(3 * 1024 * 1024);
        let (code, body) = call(
            &st,
            "POST",
            "/api/resources?title=big&file_name=big.bin&note_id=00000000-0000-0000-0000-000000000001",
            Some(&big),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let res: Resource = serde_json::from_slice(&body).unwrap();
        assert_eq!(res.size, big.len() as u64);
    }
```

**What it does** — A 3 MiB body (over axum's 2 MiB default) succeeds because the
router raises the limit to `max_body_bytes` (32 MiB in the test state).

### fn streaming_upload_round_trips

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn streaming_upload_round_trips`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn streaming_upload_round_trips
    #[tokio::test]
    async fn streaming_upload_round_trips() {
        let st = state(None).await;
        let payload = "some large attachment bytes";
        let (code, body) = call(
            &st,
            "POST",
            "/api/resources/upload?title=vid&file_name=v.bin&note_id=00000000-0000-0000-0000-000000000001",
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
```

**What it does** — `POST /api/resources/upload` stores the streamed body; size
and bytes round-trip intact.

### fn streaming_upload_over_cap_is_413

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn streaming_upload_over_cap_is_413`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — With `max_upload_bytes = 8`, a 10-byte streamed body → 413.

### fn alias_and_links_endpoints

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn alias_and_links_endpoints`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — On a `linking_state`: a note whose body declares a bookmark
with an inline alias (`[Anchor1](### "Custom")`) and a link returns the derived
bookmark inline on the note (there is no dedicated bookmark endpoint — the body
is the source of truth); the links list has the content link; adding a manual
link makes two; a malformed reference (`not-a-ref`) is 422.

### fn alias_backlinks_and_resolve_endpoints

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn alias_backlinks_and_resolve_endpoints`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Aliases live in a real notebook (Inbox notes carry none), so
both notes are placed in one: the target gets alias `note3` via the alias
endpoint; a source note linking `#note3` appears in the target's backlinks; a
3-segment `?ref=` resolves to the target note plus bookmark number 1.

### fn alias_conflicts_endpoint

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn alias_conflicts_endpoint`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — On a plain `FsBackend` state (no `LinkingBackend`, so no
write-time uniqueness check) the same alias planted on two notes in one real
notebook — the way a cross-device sync collision would appear —
`GET /api/aliases/conflicts` reports one note-conflict group (`dup`, 2
entities) and no notebook conflicts.

### fn state_with_events

**Identification** — helper; marker `// md:mod tests > fn state_with_events`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — An `AppState` whose backend is an `EventBackend` over
`FsBackend`, so mutations made through the router publish to the same `events`
channel the WebSocket route subscribes to.

### fn websocket_streams_note_create

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn websocket_streams_note_create`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — End to end over a real socket: serves the router on an
ephemeral port, connects `tokio_tungstenite` to `/api/ws` (the handler
subscribes synchronously before the upgrade response, so nothing after the
connect can be missed), creates a note through the shared backend, and asserts
the client receives a `Change::NoteCreate` text frame with the right
title/body.

---

## fn note_presence

**Identification** — `async fn note_presence(State, Path<Uuid>) ->
Json<Vec<PresenceInfo>>`. Marker `// md:fn note_presence`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `GET /api/notes/:id/presence` (collaborative presence,
design §7.3): who is inside the note's live session and where their caret is
(`collab.presence(id)`). Empty list when collaboration is disabled or nobody is
in — never an error.

---

## CursorBody

**Identification** — `#[derive(Deserialize)] struct CursorBody`. Marker
`// md:CursorBody`.

**Code** — complete and verbatim:

```rust
// md:CursorBody
#[derive(Deserialize)]
struct CursorBody {
    line_id: Uuid,
    column: usize,
}
```

**What it does** — `{ "line_id": <uuid>, "column": n }` — a caret position in
the line-based collab protocol.

---

## fn set_cursor

**Identification** — `async fn set_cursor(State, Path<Uuid>, Json<CursorBody>)
-> StatusCode`. Marker `// md:fn set_cursor`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `PUT /api/notes/:id/cursor`: publishes this device's caret
(`collab.send_cursor`, fire-and-forget) so the server fans the updated presence
out to every participant → `204`; `503` when collaboration is disabled.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `AppState`, `Shared`, `router()` — defined here; the HTTP surface (EXTRACTED; referenced by `main.rs`)
- the ~80 handler functions and request/response shapes — defined here (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/models.rs` — domain entities serialised straight to JSON (EXTRACTED: references)
- `keeplin-core/src/ordering.rs` — placement/pin/star/reorder + Inbox rules (EXTRACTED: references)
- `keeplin-core/src/linking.rs` — alias/link/backlink/resolve (EXTRACTED: references)
- `keeplin-core/src/history.rs` — the revert operations (EXTRACTED: references)
- `keeplin-core/src/interop.rs` — vCard/iCalendar import/export (EXTRACTED: references)
- `keeplin-core/src/sync/mod.rs` — `run_sync` (EXTRACTED: references)
- `keeplin-core/src/collab/mod.rs` + `collab/protocol.rs` — `CollabHandle`, presence/cursor types (EXTRACTED: references)
- `keeplin-daemon/src/auth.rs` — `verify_basic` (EXTRACTED: references)
- `keeplin-daemon/src/server.rs` — the two post-sync maintenance helpers (EXTRACTED: references)
- `keeplin-daemon/src/search.rs` — `SearchHandle`, `SearchQuery` (EXTRACTED: references)
- `keeplin-daemon/src/metrics.rs` — the shared registry (EXTRACTED: references)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/main.rs` — builds `AppState` and serves `router` (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Tombstones read and update as `404` on this surface; revival is reserved for sync's `apply_change`.
- `updated_at` is stamped server-side on every update handler; client values are ignored (issue #75).
- The Inbox system notebook (nil UUID) cannot be deleted; ordering rules are enforced by `keeplin_core::ordering`, never re-implemented here.
- Operational routes (`/health`, `/ready`, `/metrics`) bypass auth **and** the status counter; every data route passes through both, auth inside the counter.
- The `ApiError` mapping is the single source of truth for REST status codes and must stay aligned with the gRPC `storage_err` mapping in `server.rs`.
- The WebSocket handler subscribes before the upgrade response, and a lagged client gets a `{"type":"resync"}` hint instead of silent loss.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `AppState` | `// md:AppState` |
| 3 | `Shared` | `// md:Shared` |
| 4 | `fn router` | `// md:fn router` |
| 5 | `SearchParams` | `// md:SearchParams` |
| 6 | `fn search_notes` | `// md:fn search_notes` |
| 7 | `fn auth_mw` | `// md:fn auth_mw` |
| 8 | `fn status_mw` | `// md:fn status_mw` |
| 9 | `ApiError` | `// md:ApiError` |
| 10 | `impl From StorageError for ApiError` | `// md:impl From StorageError for ApiError` |
| 11 | `impl From SyncError for ApiError` | `// md:impl From SyncError for ApiError` |
| 12 | `impl IntoResponse for ApiError` | `// md:impl IntoResponse for ApiError` |
| 13 | `Pagination` | `// md:Pagination` |
| 14 | `Page` | `// md:Page` |
| 15 | `fn page` | `// md:fn page` |
| 16 | `fn health` | `// md:fn health` |
| 17 | `fn ready` | `// md:fn ready` |
| 18 | `fn metrics` | `// md:fn metrics` |
| 19 | `CreateNote` | `// md:CreateNote` |
| 20 | `fn list_notes` | `// md:fn list_notes` |
| 21 | `fn create_note` | `// md:fn create_note` |
| 22 | `fn get_note` | `// md:fn get_note` |
| 23 | `fn update_note` | `// md:fn update_note` |
| 24 | `fn delete_note` | `// md:fn delete_note` |
| 25 | `fn list_note_tags` | `// md:fn list_note_tags` |
| 26 | `fn add_note_tag` | `// md:fn add_note_tag` |
| 27 | `fn remove_note_tag` | `// md:fn remove_note_tag` |
| 28 | `AliasBody` | `// md:AliasBody` |
| 29 | `fn read_live_note` | `// md:fn read_live_note` |
| 30 | `fn set_note_alias` | `// md:fn set_note_alias` |
| 31 | `fn list_links` | `// md:fn list_links` |
| 32 | `AddLinkBody` | `// md:AddLinkBody` |
| 33 | `fn add_link` | `// md:fn add_link` |
| 34 | `fn remove_link` | `// md:fn remove_link` |
| 35 | `fn list_backlinks` | `// md:fn list_backlinks` |
| 36 | `fn list_notes_in_notebook` | `// md:fn list_notes_in_notebook` |
| 37 | `fn list_starred_notes` | `// md:fn list_starred_notes` |
| 38 | `fn pin_note` | `// md:fn pin_note` |
| 39 | `fn unpin_note` | `// md:fn unpin_note` |
| 40 | `fn star_note` | `// md:fn star_note` |
| 41 | `fn unstar_note` | `// md:fn unstar_note` |
| 42 | `ReorderBody` | `// md:ReorderBody` |
| 43 | `fn reorder_note` | `// md:fn reorder_note` |
| 44 | `ResolveQuery` | `// md:ResolveQuery` |
| 45 | `fn resolve_reference` | `// md:fn resolve_reference` |
| 46 | `fn list_alias_conflicts` | `// md:fn list_alias_conflicts` |
| 47 | `HistoryQuery` | `// md:HistoryQuery` |
| 48 | `NoteVersion` | `// md:NoteVersion` |
| 49 | `NotebookVersion` | `// md:NotebookVersion` |
| 50 | `RevertBody` | `// md:RevertBody` |
| 51 | `BatchRevertBody` | `// md:BatchRevertBody` |
| 52 | `fn note_history` | `// md:fn note_history` |
| 53 | `fn notebook_history` | `// md:fn notebook_history` |
| 54 | `fn revert_note_ep` | `// md:fn revert_note_ep` |
| 55 | `fn revert_notebook_ep` | `// md:fn revert_notebook_ep` |
| 56 | `fn revert_notebook_notes_ep` | `// md:fn revert_notebook_notes_ep` |
| 57 | `fn batch_revert_notes_ep` | `// md:fn batch_revert_notes_ep` |
| 58 | `fn note_version_dto` | `// md:fn note_version_dto` |
| 59 | `fn notebook_version_dto` | `// md:fn notebook_version_dto` |
| 60 | `fn proxy_perm` | `// md:fn proxy_perm` |
| 61 | `fn proxy_note_share` | `// md:fn proxy_note_share` |
| 62 | `fn proxy_note_shares` | `// md:fn proxy_note_shares` |
| 63 | `fn proxy_note_unshare` | `// md:fn proxy_note_unshare` |
| 64 | `fn proxy_note_transfer` | `// md:fn proxy_note_transfer` |
| 65 | `fn proxy_notebook_share` | `// md:fn proxy_notebook_share` |
| 66 | `fn proxy_notebook_shares` | `// md:fn proxy_notebook_shares` |
| 67 | `fn proxy_notebook_unshare` | `// md:fn proxy_notebook_unshare` |
| 68 | `fn proxy_notebook_transfer` | `// md:fn proxy_notebook_transfer` |
| 69 | `ContactDto` | `// md:ContactDto` |
| 70 | `impl From Contact for ContactDto` | `// md:impl From Contact for ContactDto` |
| 71 | `EventDto` | `// md:EventDto` |
| 72 | `impl From CalendarEvent for EventDto` | `// md:impl From CalendarEvent for EventDto` |
| 73 | `fn text_body` | `// md:fn text_body` |
| 74 | `fn list_contacts_ep` | `// md:fn list_contacts_ep` |
| 75 | `fn import_contact_ep` | `// md:fn import_contact_ep` |
| 76 | `fn export_contact_ep` | `// md:fn export_contact_ep` |
| 77 | `fn delete_contact_ep` | `// md:fn delete_contact_ep` |
| 78 | `fn list_events_ep` | `// md:fn list_events_ep` |
| 79 | `fn import_event_ep` | `// md:fn import_event_ep` |
| 80 | `fn export_event_ep` | `// md:fn export_event_ep` |
| 81 | `fn delete_event_ep` | `// md:fn delete_event_ep` |
| 82 | `fn import_todo_ep` | `// md:fn import_todo_ep` |
| 83 | `ProfileVcardQuery` | `// md:ProfileVcardQuery` |
| 84 | `fn profile_vcard_ep` | `// md:fn profile_vcard_ep` |
| 85 | `TitleOnly` | `// md:TitleOnly` |
| 86 | `fn list_notebooks` | `// md:fn list_notebooks` |
| 87 | `fn create_notebook` | `// md:fn create_notebook` |
| 88 | `fn read_live_notebook` | `// md:fn read_live_notebook` |
| 89 | `fn get_notebook` | `// md:fn get_notebook` |
| 90 | `fn update_notebook` | `// md:fn update_notebook` |
| 91 | `fn delete_notebook` | `// md:fn delete_notebook` |
| 92 | `fn set_notebook_alias` | `// md:fn set_notebook_alias` |
| 93 | `fn list_tags` | `// md:fn list_tags` |
| 94 | `fn create_tag` | `// md:fn create_tag` |
| 95 | `fn read_live_tag` | `// md:fn read_live_tag` |
| 96 | `fn get_tag` | `// md:fn get_tag` |
| 97 | `fn update_tag` | `// md:fn update_tag` |
| 98 | `fn delete_tag` | `// md:fn delete_tag` |
| 99 | `ResourceMeta` | `// md:ResourceMeta` |
| 100 | `ListResourcesQuery` | `// md:ListResourcesQuery` |
| 101 | `fn list_resources` | `// md:fn list_resources` |
| 102 | `fn create_resource` | `// md:fn create_resource` |
| 103 | `fn upload_resource` | `// md:fn upload_resource` |
| 104 | `fn get_resource` | `// md:fn get_resource` |
| 105 | `fn get_resource_data` | `// md:fn get_resource_data` |
| 106 | `fn delete_resource` | `// md:fn delete_resource` |
| 107 | `SyncSummary` | `// md:SyncSummary` |
| 108 | `fn sync` | `// md:fn sync` |
| 109 | `fn ws_handler` | `// md:fn ws_handler` |
| 110 | `fn stream_changes` | `// md:fn stream_changes` |
| 111 | `mod tests` (container) | `// md:mod tests` |
| 112 | `imports` | `// md:mod tests > imports` |
| 113 | `fn state` | `// md:mod tests > fn state` |
| 114 | `fn linking_state` | `// md:mod tests > fn linking_state` |
| 115 | `fn call` | `// md:mod tests > fn call` |
| 116 | `fn note_crud_round_trip` | `// md:mod tests > fn note_crud_round_trip` |
| 117 | `fn permission_endpoints_require_server_mode` | `// md:mod tests > fn permission_endpoints_require_server_mode` |
| 118 | `fn contact_import_list_export_delete_endpoints` | `// md:mod tests > fn contact_import_list_export_delete_endpoints` |
| 119 | `fn todo_import_and_profile_vcard_endpoints` | `// md:mod tests > fn todo_import_and_profile_vcard_endpoints` |
| 120 | `fn note_history_and_revert_endpoints` | `// md:mod tests > fn note_history_and_revert_endpoints` |
| 121 | `fn updates_on_deleted_entities_are_404` | `// md:mod tests > fn updates_on_deleted_entities_are_404` |
| 122 | `fn sync_endpoint_prunes_journal_within_retention` | `// md:mod tests > fn sync_endpoint_prunes_journal_within_retention` |
| 123 | `fn operational_endpoints_bypass_auth` | `// md:mod tests > fn operational_endpoints_bypass_auth` |
| 124 | `fn metrics_state` | `// md:mod tests > fn metrics_state` |
| 125 | `fn metrics_reflect_operations_and_http_status` | `// md:mod tests > fn metrics_reflect_operations_and_http_status` |
| 126 | `fn invalid_uuid_is_bad_request` | `// md:mod tests > fn invalid_uuid_is_bad_request` |
| 127 | `fn auth_is_enforced_when_configured` | `// md:mod tests > fn auth_is_enforced_when_configured` |
| 128 | `fn resource_upload_and_download` | `// md:mod tests > fn resource_upload_and_download` |
| 129 | `fn resource_upload_above_axum_default_limit` | `// md:mod tests > fn resource_upload_above_axum_default_limit` |
| 130 | `fn streaming_upload_round_trips` | `// md:mod tests > fn streaming_upload_round_trips` |
| 131 | `fn streaming_upload_over_cap_is_413` | `// md:mod tests > fn streaming_upload_over_cap_is_413` |
| 132 | `fn alias_and_links_endpoints` | `// md:mod tests > fn alias_and_links_endpoints` |
| 133 | `fn alias_backlinks_and_resolve_endpoints` | `// md:mod tests > fn alias_backlinks_and_resolve_endpoints` |
| 134 | `fn alias_conflicts_endpoint` | `// md:mod tests > fn alias_conflicts_endpoint` |
| 135 | `fn state_with_events` | `// md:mod tests > fn state_with_events` |
| 136 | `fn websocket_streams_note_create` | `// md:mod tests > fn websocket_streams_note_create` |
| 137 | `fn note_presence` | `// md:fn note_presence` |
| 138 | `CursorBody` | `// md:CursorBody` |
| 139 | `fn set_cursor` | `// md:fn set_cursor` |
