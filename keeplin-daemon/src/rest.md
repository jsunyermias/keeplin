# `rest.rs` — the REST/JSON + WebSocket surface

Self-contained companion for `keeplin-daemon/src/rest.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must
be able to understand it without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module doc and the imports. Marker
`// md:Overview`.

```rust
use std::sync::Arc;
use axum::{body::{Body, Bytes}, extract::{ws::…, DefaultBodyLimit, Path, Query,
    Request, State}, http::{header::…, HeaderMap, StatusCode},
    middleware::{self, Next}, response::{IntoResponse, Response},
    routing::{get, post, put}, Json, Router};
use chrono::{DateTime, Utc};
use keeplin_core::{error::{StorageError, SyncError}, history,
    interop::{self, CalendarEvent, Contact}, linking,
    links::{parse_link_ref, NoteLink},
    models::{now, Change, Note, NoteTag, Notebook, Resource, Tag},
    ordering, storage::{EntityVersion, StorageBackend}, sync::run_sync};
use serde::{Deserialize, Serialize}; use serde_json::json;
use tokio::sync::broadcast; use uuid::Uuid;
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

**What it does** — Handler-facing state alias; `Arc` makes it cheaply cloneable
for axum's `State` extractor.

---

## fn router

**Identification** — `pub fn router(state: Shared) -> Router`. Marker
`// md:fn router`.

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

**What it does** — Middleware: runs the request, then records the response's
status class into the shared registry (`keeplin_http_requests_total` via
`Metrics::record_http_status`). Applied only to the data API, so probe/scrape
traffic does not inflate the counts.

**Used by** — layered onto the data API in `router`.

---

## ApiError

**Identification** — `struct ApiError(StorageError)`. Marker `// md:ApiError`.

**What it does** — Newtype letting handlers return `Result<_, ApiError>` with
`?` on backend calls; the `IntoResponse` impl below does the HTTP mapping.

**Used by** — every fallible handler in this file.

---

## impl From StorageError for ApiError

**Identification** — marker `// md:impl From StorageError for ApiError`.

**What it does** — Plain wrap, enabling `?` on storage results.

---

## impl From SyncError for ApiError

**Identification** — marker `// md:impl From SyncError for ApiError`.

**What it does** — `SyncError::Storage(s)` keeps its precise mapping (e.g.
NotFound → 404); other variants (Conflict/Failed — transport- or protocol-level
sync failures) become `StorageError::InvalidState` and surface as a 500 with the
underlying message rather than inventing a finer status.

**Used by** — the `sync` handler's `?`.

---

## impl IntoResponse for ApiError

**Identification** — marker `// md:impl IntoResponse for ApiError`.

**What it does** — The single `StorageError` → HTTP status mapping:

| `StorageError` | HTTP | Why |
|---|---|---|
| `NotFound` | 404 | missing or tombstoned entity |
| `CorruptedData` | 422 | undecryptable/unparsable payload |
| `Conflict` | 409 | duplicate alias — a client conflict, not a server bug |
| `InvalidInput` | 400 | domain-rule rejection (pin an Inbox note, out-of-band sort key, delete the Inbox) |
| everything else | 500 | |

Body is `{"error": "<message>"}`.

---

## Pagination

**Identification** — `#[derive(Debug, Deserialize)] struct Pagination`. Marker
`// md:Pagination`.

**What it does** — `?page_size=&page_token=` for every list endpoint (both
defaulted; `page_size` `0` lets the backend choose).

---

## Page

**Identification** — `#[derive(Debug, Serialize)] struct Page<T>`. Marker
`// md:Page`.

**What it does** — A page of results: `items` plus `next_page_token` (omitted
from the JSON when `None` — no more pages).

---

## fn page

**Identification** — `fn page<T>((items, next): (Vec<T>, Option<String>)) ->
Json<Page<T>>`. Marker `// md:fn page`.

**What it does** — Adapter from the backend's `(Vec<T>, Option<String>)` list
result to the JSON page shape; used with `?` in every list handler.

---

## fn health

**Identification** — `async fn health() -> &'static str`. Marker
`// md:fn health`.

**What it does** — Liveness probe: always `200 ok`, never touches the backend —
stays green even if storage is momentarily unavailable (that is what `ready` is
for).

---

## fn ready

**Identification** — `async fn ready(State) -> Response`. Marker
`// md:fn ready`.

**What it does** — Readiness probe: one cheap backend read (`list_notes` page
size 1). `200 ready` when storage answers; `503` with the error otherwise, so an
orchestrator stops routing to an instance whose database is locked or
unreachable. (The read flows through the metrics decorator, so a busy readiness
schedule contributes to the `note`/`list` counter.)

---

## fn metrics

**Identification** — `async fn metrics(State) -> Response`. Marker
`// md:fn metrics`.

**What it does** — Prometheus exposition (`text/plain; version=0.0.4`) of the
shared registry. Unauthenticated: counters carry only fixed-label aggregates, no
user content.

---

## CreateNote

**Identification** — `#[derive(Debug, Deserialize)] struct CreateNote`. Marker
`// md:CreateNote`.

**What it does** — `POST /api/notes` body: `title` (required), `body`,
`notebook_id` (absent → Inbox), `is_todo`, `todo_due` (all defaulted).

---

## fn list_notes

**Identification** — marker `// md:fn list_notes`.

**What it does** — `GET /api/notes`: paginated `backend.list_notes`.

---

## fn create_note

**Identification** — marker `// md:fn create_note`.

**What it does** — `POST /api/notes`: builds `Note::new(title, body)`; absent
`notebook_id` → nil UUID (the Inbox); applies `is_todo`/`todo_due`; then
`ordering::place_new_note` gives the initial manual position (top of the Inbox,
or the end of a normal notebook's unpinned band) before `create_note`.

---

## fn get_note

**Identification** — marker `// md:fn get_note`.

**What it does** — `GET /api/notes/:id`. The backend retains soft-deleted
entities as tombstones (for sync); this surface presents a clean lifecycle, so a
deleted note reads as **404** (checked via `deleted_at`).

---

## fn update_note

**Identification** — marker `// md:fn update_note`.

**What it does** — `PUT /api/notes/:id` with a full `Note` JSON body. A
tombstoned note is 404 (via `read_live_note`) — otherwise a PUT (whose body
defaults `deleted_at` to null) would silently revive it; revival is reserved for
sync's `apply_change`. The path id overrides the body id. Then
`ordering::reconcile_notebook_move`: moving to a different notebook re-places
the note (its old position and pinned state belonged to the source notebook); a
plain edit keeps its position. `updated_at = now()` server-side.

---

## fn delete_note

**Identification** — marker `// md:fn delete_note`.

**What it does** — `DELETE /api/notes/:id` → `204` (soft delete).

---

## fn list_note_tags

**Identification** — marker `// md:fn list_note_tags`.

**What it does** — `GET /api/notes/:id/tags`: paginated tags on one note.

---

## fn add_note_tag

**Identification** — marker `// md:fn add_note_tag`.

**What it does** — `PUT /api/notes/:note_id/tags/:tag_id` → `204`
(`backend.add_note_tag`, idempotent at the storage layer).

---

## fn remove_note_tag

**Identification** — marker `// md:fn remove_note_tag`.

**What it does** — `DELETE /api/notes/:note_id/tags/:tag_id` → `204`.

---

## AliasBody

**Identification** — `#[derive(Debug, Deserialize)] struct AliasBody`. Marker
`// md:AliasBody`.

**What it does** — `{ "alias": "…" | null }` body shared by the two
alias-setting endpoints (`null`/absent clears the alias).

---

## fn read_live_note

**Identification** — `async fn read_live_note(s: &Shared, id: Uuid) ->
Result<Note, ApiError>`. Marker `// md:fn read_live_note`.

**What it does** — Read a live note or 404 for a missing or soft-deleted one
(mirrors `get_note`); the shared tombstone guard for note handlers.

**Used by** — `update_note`, `list_links`.

---

## fn set_note_alias

**Identification** — marker `// md:fn set_note_alias`.

**What it does** — `PUT /api/notes/:id/alias`: `linking::set_note_alias`
(uniqueness enforced by the linking layer; duplicate → 409).

---

## fn list_links

**Identification** — marker `// md:fn list_links`.

**What it does** — `GET /api/notes/:id/links`: the live note's `links` array
(content-derived + manual).

---

## AddLinkBody

**Identification** — `#[derive(Debug, Deserialize)] struct AddLinkBody`. Marker
`// md:AddLinkBody`.

**What it does** — `{ "raw": "#notebook1#note3#5" }` body for adding a manual
(global) link.

---

## fn add_link

**Identification** — marker `// md:fn add_link`.

**What it does** — `POST /api/notes/:id/links`: validates the reference syntax
up front with `parse_link_ref` so a bad body is a **422** (`CorruptedData`), not
a 500; then `linking::add_manual_link`.

---

## fn remove_link

**Identification** — marker `// md:fn remove_link`.

**What it does** — `DELETE /api/notes/:id/links/:index`:
`linking::remove_link` by index into the note's links array.

---

## fn list_backlinks

**Identification** — marker `// md:fn list_backlinks`.

**What it does** — `GET /api/notes/:id/backlinks`: paginated
`linking::backlinks` — the notes whose links resolve to this one.

---

## fn list_notes_in_notebook

**Identification** — marker `// md:fn list_notes_in_notebook`.

**What it does** — `GET /api/notebooks/:id/notes`: the notebook's notes in
manual order (pinned band first). Use the nil UUID for the Inbox.

---

## fn list_starred_notes

**Identification** — marker `// md:fn list_starred_notes`.

**What it does** — `GET /api/notes/starred`: every live starred note, across all
notebooks.

---

## fn pin_note

**Identification** — marker `// md:fn pin_note`.

**What it does** — `POST /api/notes/:id/pin`: `ordering::pin_note` — into the
notebook's pinned band (`1..=999`; Inbox notes reject with 400, full band 409).

---

## fn unpin_note

**Identification** — marker `// md:fn unpin_note`.

**What it does** — `DELETE /api/notes/:id/pin`: back to the end of the normal
band.

---

## fn star_note

**Identification** — marker `// md:fn star_note`.

**What it does** — `POST /api/notes/:id/star`: sets the global star (never moves
the note).

---

## fn unstar_note

**Identification** — marker `// md:fn unstar_note`.

**What it does** — `DELETE /api/notes/:id/star`.

---

## ReorderBody

**Identification** — `#[derive(Deserialize)] struct ReorderBody`. Marker
`// md:ReorderBody`.

**What it does** — `{ "sort_key": … }` body for the reorder endpoint.

---

## fn reorder_note

**Identification** — marker `// md:fn reorder_note`.

**What it does** — `PUT /api/notes/:id/sort-key`: `ordering::reorder_note` — a
new manual position within the note's current band (pinned `1..=999`, normal
`>= 1000`, Inbox `>= 1`); out-of-band keys are 400.

---

## ResolveQuery

**Identification** — `#[derive(Debug, Deserialize)] struct ResolveQuery` with
`#[serde(rename = "ref")]`. Marker `// md:ResolveQuery`.

**What it does** — `?ref=#notebook1#note3#5` query for reference resolution.

---

## fn resolve_reference

**Identification** — marker `// md:fn resolve_reference`.

**What it does** — `GET /api/links/resolve?ref=…`: `linking::resolve` → JSON
`{note_id, bookmark_number}`; an unresolvable reference is `{null, null}`, not
an error.

---

## fn list_alias_conflicts

**Identification** — marker `// md:fn list_alias_conflicts`.

**What it does** — `GET /api/aliases/conflicts`: note/notebook aliases shared by
two or more live entities (the residue of a cross-device alias collision), so a
human can rename one side. Serialises `linking::AliasConflicts` directly.

---

## HistoryQuery

**Identification** — `#[derive(Debug, Default, Deserialize)] struct
HistoryQuery`. Marker `// md:HistoryQuery`.

**What it does** — `?limit=` for the history endpoints; `0` (absent) uses the
backend's default cap.

---

## NoteVersion

**Identification** — `#[derive(Debug, Serialize)] struct NoteVersion`. Marker
`// md:NoteVersion`.

**What it does** — One past version of a note (`timestamp`, `device_id`,
optional `note` — absent when the version is a tombstone).

---

## NotebookVersion

**Identification** — `#[derive(Debug, Serialize)] struct NotebookVersion`.
Marker `// md:NotebookVersion`.

**What it does** — The notebook counterpart of `NoteVersion`.

---

## RevertBody

**Identification** — `#[derive(Debug, Deserialize)] struct RevertBody`. Marker
`// md:RevertBody`.

**What it does** — `{ "at": "<RFC-3339>" }` — the instant to roll an entity back
to (the newest version at or before it).

---

## BatchRevertBody

**Identification** — `#[derive(Debug, Deserialize)] struct BatchRevertBody`.
Marker `// md:BatchRevertBody`.

**What it does** — `{ "at": …, "note_ids": [ … ] }` — batch forward-revert of
the listed notes.

---

## fn note_history

**Identification** — marker `// md:fn note_history`.

**What it does** — `GET /api/notes/:id/history`: `backend.note_history`, newest
first, mapped through `note_version_dto`.

---

## fn notebook_history

**Identification** — marker `// md:fn notebook_history`.

**What it does** — `GET /api/notebooks/:id/history`, mapped through
`notebook_version_dto`.

---

## fn revert_note_ep

**Identification** — marker `// md:fn revert_note_ep`.

**What it does** — `POST /api/notes/:id/revert`: `history::revert_note` — a
**forward** revert (writes the old state as a new version; non-destructive).

---

## fn revert_notebook_ep

**Identification** — marker `// md:fn revert_notebook_ep`.

**What it does** — `POST /api/notebooks/:id/revert`:
`history::revert_notebook`.

---

## fn revert_notebook_notes_ep

**Identification** — marker `// md:fn revert_notebook_notes_ep`.

**What it does** — `POST /api/notebooks/:id/notes/revert`:
`history::revert_notebook_notes_to` — batch-revert every note currently in the
notebook to its state as of `at` (the roll-back companion to a destructive
notebook-wide change).

---

## fn batch_revert_notes_ep

**Identification** — marker `// md:fn batch_revert_notes_ep`.

**What it does** — `POST /api/history/revert`: `history::revert_notes_to` over
an explicit note-id list.

---

## fn note_version_dto

**Identification** — `fn note_version_dto(v: EntityVersion<Note>) ->
NoteVersion`. Marker `// md:fn note_version_dto`.

**What it does** — Field map from the storage-layer version record.

---

## fn notebook_version_dto

**Identification** — `fn notebook_version_dto(v: EntityVersion<Notebook>) ->
NotebookVersion`. Marker `// md:fn notebook_version_dto`.

**What it does** — Notebook counterpart of `note_version_dto`.

---

## fn proxy_perm

**Identification** — `async fn proxy_perm(s: &Shared, method: &str, path:
String, body: Option<serde_json::Value>) -> Response`. Marker
`// md:fn proxy_perm`.

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

**What it does** — `POST /api/notes/:id/share` → forwards body to the server's
same path.

---

## fn proxy_note_shares

**Identification** — marker `// md:fn proxy_note_shares`.

**What it does** — `GET /api/notes/:id/share` → forwarded.

---

## fn proxy_note_unshare

**Identification** — marker `// md:fn proxy_note_unshare`.

**What it does** — `DELETE /api/notes/:id/share/:user_id` → forwarded.

---

## fn proxy_note_transfer

**Identification** — marker `// md:fn proxy_note_transfer`.

**What it does** — `POST /api/notes/:id/transfer` → forwarded with body.

---

## fn proxy_notebook_share

**Identification** — marker `// md:fn proxy_notebook_share`.

**What it does** — `POST /api/notebooks/:id/share` → forwarded with body.

---

## fn proxy_notebook_shares

**Identification** — marker `// md:fn proxy_notebook_shares`.

**What it does** — `GET /api/notebooks/:id/share` → forwarded.

---

## fn proxy_notebook_unshare

**Identification** — marker `// md:fn proxy_notebook_unshare`.

**What it does** — `DELETE /api/notebooks/:id/share/:user_id` → forwarded.

---

## fn proxy_notebook_transfer

**Identification** — marker `// md:fn proxy_notebook_transfer`.

**What it does** — `POST /api/notebooks/:id/transfer` → forwarded with body.

---

## ContactDto

**Identification** — `#[derive(Debug, Serialize)] struct ContactDto`. Marker
`// md:ContactDto`.

**What it does** — JSON view of `keeplin_core::interop::Contact` (`uid`,
`formatted_name`, optional name parts, `emails`, `phones`, optional
`org`/`note`; `None`s omitted).

---

## impl From Contact for ContactDto

**Identification** — marker `// md:impl From Contact for ContactDto`.

**What it does** — Field-for-field map.

---

## EventDto

**Identification** — `#[derive(Debug, Serialize)] struct EventDto`. Marker
`// md:EventDto`.

**What it does** — JSON view of a calendar event (`uid`, `summary`, optional
`start`/`end`/`location`/`description`).

---

## impl From CalendarEvent for EventDto

**Identification** — marker `// md:impl From CalendarEvent for EventDto`.

**What it does** — Field-for-field map.

---

## fn text_body

**Identification** — `fn text_body(mime: &'static str, body: String) ->
Response`. Marker `// md:fn text_body`.

**What it does** — Builds a raw `text/vcard` or `text/calendar` body response.

**Used by** — `export_contact_ep`, `export_event_ep`, `profile_vcard_ep`.

---

## fn list_contacts_ep

**Identification** — marker `// md:fn list_contacts_ep`.

**What it does** — `GET /api/contacts`: `interop::list_contacts` → `ContactDto`
list.

---

## fn import_contact_ep

**Identification** — marker `// md:fn import_contact_ep`.

**What it does** — `POST /api/contacts/import`: the raw body is a vCard;
`Contact::from_vcard` (invalid → 400 `InvalidInput`), stored with
`interop::save_contact`, stored contact returned.

---

## fn export_contact_ep

**Identification** — marker `// md:fn export_contact_ep`.

**What it does** — `GET /api/contacts/:uid/export`: the contact as a
`text/vcard` body (`interop::MIME_VCARD`); unknown uid → 404.

---

## fn delete_contact_ep

**Identification** — marker `// md:fn delete_contact_ep`.

**What it does** — `DELETE /api/contacts/:uid` → `204`.

---

## fn list_events_ep

**Identification** — marker `// md:fn list_events_ep`.

**What it does** — `GET /api/events`: `interop::list_events` → `EventDto` list.

---

## fn import_event_ep

**Identification** — marker `// md:fn import_event_ep`.

**What it does** — `POST /api/events/import`: the raw body is an iCalendar file;
**every** `VEVENT` is parsed (`CalendarEvent::from_ics_all`) and stored, and the
stored events are returned in document order — a whole exported calendar imports
in one call. No `VEVENT` at all → 400.

---

## fn export_event_ep

**Identification** — marker `// md:fn export_event_ep`.

**What it does** — `GET /api/events/:uid/export`: the event as a
`text/calendar` body (`interop::MIME_ICALENDAR`); unknown uid → 404.

---

## fn delete_event_ep

**Identification** — marker `// md:fn delete_event_ep`.

**What it does** — `DELETE /api/events/:uid` → `204`.

---

## fn import_todo_ep

**Identification** — marker `// md:fn import_todo_ep`.

**What it does** — `POST /api/todos/import`: a Keeplin to-do note is created
from **every** `VTODO` in the iCalendar body (`interop::import_todos`), returned
in document order.

---

## ProfileVcardQuery

**Identification** — `#[derive(Debug, Deserialize)] struct ProfileVcardQuery`.
Marker `// md:ProfileVcardQuery`.

**What it does** — `?name=&email=` (email required) for the profile-vCard
endpoint.

---

## fn profile_vcard_ep

**Identification** — marker `// md:fn profile_vcard_ep`.

**What it does** — `GET /api/profile/vcard?email=&name=`: renders the account
owner's profile vCard (`interop::user_vcard`). The caller supplies the profile —
the daemon does not own user identity; a blank `name` defaults to the email's
local part.

---

## TitleOnly

**Identification** — `#[derive(Debug, Deserialize)] struct TitleOnly`. Marker
`// md:TitleOnly`.

**What it does** — `{ "title": … }` body shared by notebook and tag creation.

---

## fn list_notebooks

**Identification** — marker `// md:fn list_notebooks`.

**What it does** — `GET /api/notebooks`: paginated.

---

## fn create_notebook

**Identification** — marker `// md:fn create_notebook`.

**What it does** — `POST /api/notebooks`: `Notebook::new(title)`.

---

## fn read_live_notebook

**Identification** — `async fn read_live_notebook(s: &Shared, id: Uuid) ->
Result<Notebook, ApiError>`. Marker `// md:fn read_live_notebook`.

**What it does** — Live-or-404 tombstone guard for notebooks.

**Used by** — `get_notebook`, `update_notebook`.

---

## fn get_notebook

**Identification** — marker `// md:fn get_notebook`.

**What it does** — `GET /api/notebooks/:id` via the live guard (tombstone →
404).

---

## fn update_notebook

**Identification** — marker `// md:fn update_notebook`.

**What it does** — `PUT /api/notebooks/:id`: tombstone → 404 (no revival); path
id wins; `updated_at = now()` server-side.

---

## fn delete_notebook

**Identification** — marker `// md:fn delete_notebook`.

**What it does** — `DELETE /api/notebooks/:id`: **the Inbox system notebook
(nil UUID) cannot be deleted** (`ordering::is_inbox` → 400); otherwise soft
delete → `204`.

---

## fn set_notebook_alias

**Identification** — marker `// md:fn set_notebook_alias`.

**What it does** — `PUT /api/notebooks/:id/alias`:
`linking::set_notebook_alias`.

---

## fn list_tags

**Identification** — marker `// md:fn list_tags`.

**What it does** — `GET /api/tags`: paginated.

---

## fn create_tag

**Identification** — marker `// md:fn create_tag`.

**What it does** — `POST /api/tags`: `Tag::new(title)`.

---

## fn read_live_tag

**Identification** — `async fn read_live_tag(s: &Shared, id: Uuid) ->
Result<Tag, ApiError>`. Marker `// md:fn read_live_tag`.

**What it does** — Live-or-404 tombstone guard for tags.

**Used by** — `get_tag`, `update_tag`.

---

## fn get_tag

**Identification** — marker `// md:fn get_tag`.

**What it does** — `GET /api/tags/:id` via the live guard.

---

## fn update_tag

**Identification** — marker `// md:fn update_tag`.

**What it does** — `PUT /api/tags/:id`: tombstone → 404; path id wins;
`updated_at = now()` server-side.

---

## fn delete_tag

**Identification** — marker `// md:fn delete_tag`.

**What it does** — `DELETE /api/tags/:id` → `204`.

---

## ResourceMeta

**Identification** — `#[derive(Debug, Deserialize)] struct ResourceMeta`. Marker
`// md:ResourceMeta`.

**What it does** — `?title=&file_name=` query metadata for the two upload
routes (both defaulted).

---

## fn list_resources

**Identification** — marker `// md:fn list_resources`.

**What it does** — `GET /api/resources`: paginated metadata.

---

## fn create_resource

**Identification** — marker `// md:fn create_resource`.

**What it does** — `POST /api/resources?title=&file_name=`: the raw request body
is the payload (`Bytes`, bounded by the router's `max_body_bytes` layer), the
`Content-Type` header is recorded as the MIME type
(`application/octet-stream` default).

---

## fn upload_resource

**Identification** — `async fn upload_resource(State, Query<ResourceMeta>,
HeaderMap, Body) -> Response`. Marker `// md:fn upload_resource`.

**What it does** — `POST /api/resources/upload?title=&file_name=`, the
**streaming** upload: this route disables the router-wide body limit and reads
the body incrementally with `axum::body::to_bytes(body, limit)` where `limit` is
`max_upload_bytes` (`0` → `usize::MAX`) — `to_bytes` errors once past the limit,
so an oversized upload never fully materialises in memory; over the cap → `413`
with a JSON error. Then stores like `create_resource`.

---

## fn get_resource

**Identification** — marker `// md:fn get_resource`.

**What it does** — `GET /api/resources/:id`: metadata only (the payload read is
discarded).

---

## fn get_resource_data

**Identification** — marker `// md:fn get_resource_data`.

**What it does** — `GET /api/resources/:id/data`: the raw bytes, served with the
stored MIME type.

---

## fn delete_resource

**Identification** — marker `// md:fn delete_resource`.

**What it does** — `DELETE /api/resources/:id` → `204` (payload reclaimed later
by the post-sync purge).

---

## SyncSummary

**Identification** — `#[derive(Debug, Serialize)] struct SyncSummary`. Marker
`// md:SyncSummary`.

**What it does** — `{ "applied": n }` — how many remote changes were applied.

---

## fn sync

**Identification** — `async fn sync(State) -> Result<Json<SyncSummary>,
ApiError>`. Marker `// md:fn sync`.

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

**What it does** — `GET /api/ws`: subscribes a fresh broadcast receiver
**before** the upgrade response is sent (so no event created after the connect
can be missed) and upgrades to a WebSocket driven by `stream_changes`. The
upgrade request passes through the same Basic-Auth middleware as the REST
routes.

---

## fn stream_changes

**Identification** — `async fn stream_changes(mut socket: WebSocket, mut rx:
broadcast::Receiver<Change>)`. Marker `// md:fn stream_changes`.

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

### fn state

**Identification** — helper; marker `// md:mod tests > fn state`.

**What it does** — An `AppState` over a fresh `FsBackend` in a leaked temp dir,
optionally with Basic-Auth credentials; generous body/upload limits, 30-day
journal retention.

### fn linking_state

**Identification** — helper; marker `// md:mod tests > fn linking_state`.

**What it does** — Like `state` but wraps the backend in `LinkingBackend`, so
writes derive bookmarks/links and resolve references — required by the
bookmark/link endpoint tests.

### fn call

**Identification** — helper; marker `// md:mod tests > fn call`.

**What it does** — One request against a fresh `router` over the shared state
via `oneshot`; returns `(status, body bytes)`. Sets `Content-Type:
application/json` when a body is given and an `Authorization` header when
provided.

### fn note_crud_round_trip

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn note_crud_round_trip`.

**What it does** — POST → GET → list (1 item) → DELETE (204) → GET is 404.

### fn permission_endpoints_require_server_mode

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn permission_endpoints_require_server_mode`.

**What it does** — With `collab: None`, share/list-shares/transfer all answer
`503`, not a panic.

### fn contact_import_list_export_delete_endpoints

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn contact_import_list_export_delete_endpoints`.

**What it does** — vCard import round-trips (name/uid), lists once, exports
containing `FN:Ada`, deletes to an empty list.

### fn todo_import_and_profile_vcard_endpoints

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn todo_import_and_profile_vcard_endpoints`.

**What it does** — Two `VTODO`s in one calendar import as two to-do notes in
document order; `/api/profile/vcard?email=me@x.com` defaults the name to the
email local part.

### fn note_history_and_revert_endpoints

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn note_history_and_revert_endpoints`.

**What it does** — Create + edit → history has two versions newest-first;
revert to the first instant restores `v1` and adds a **third** version
(non-destructive forward revert).

### fn updates_on_deleted_entities_are_404

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn updates_on_deleted_entities_are_404`.

**What it does** — For note (including the alias PUT), notebook, and tag:
create → delete → PUT is 404, no silent revival; the note stays deleted.

### fn sync_endpoint_prunes_journal_within_retention

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn sync_endpoint_prunes_journal_within_retention`.

**What it does** — On a `DbBackend` state (empty `server_url` → local-only sync;
exercises the prune path `FsBackend` no-ops): `POST /api/sync` applies 0 remote
changes and fresh journal rows survive a 30-day retention window (the prune ran
and respected the window).

### fn operational_endpoints_bypass_auth

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn operational_endpoints_bypass_auth`.

**What it does** — With auth configured, `/api/health`, `/api/ready`,
`/api/metrics` remain reachable without credentials while `/api/notes` is 401.

### fn metrics_state

**Identification** — helper; marker `// md:mod tests > fn metrics_state`.

**What it does** — An `AppState` whose backend is a `MetricsBackend` over
`FsBackend` sharing the state's own `Arc<Metrics>` — mirroring `main`'s wiring —
so router operations move the same counters `GET /api/metrics` renders.

### fn metrics_reflect_operations_and_http_status

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn metrics_reflect_operations_and_http_status`.

**What it does** — One successful create + one 404 GET → the exposition shows
`keeplin_storage_operations_total{entity="note",op="create"} 1`, one `2xx`, one
`4xx`; the `/metrics` scrape itself is not counted (operational routes bypass
the status middleware).

### fn invalid_uuid_is_bad_request

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn invalid_uuid_is_bad_request`.

**What it does** — `GET /api/notes/not-a-uuid` → 400 (axum path-extractor
rejection).

### fn auth_is_enforced_when_configured

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn auth_is_enforced_when_configured`.

**What it does** — Missing credentials → 401; wrong password → 401; valid →
200.

### fn resource_upload_and_download

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn resource_upload_and_download`.

**What it does** — Raw-body upload with query metadata round-trips; the
`/data` download returns the exact bytes.

### fn resource_upload_above_axum_default_limit

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn resource_upload_above_axum_default_limit`.

**What it does** — A 3 MiB body (over axum's 2 MiB default) succeeds because the
router raises the limit to `max_body_bytes` (32 MiB in the test state).

### fn streaming_upload_round_trips

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn streaming_upload_round_trips`.

**What it does** — `POST /api/resources/upload` stores the streamed body; size
and bytes round-trip intact.

### fn streaming_upload_over_cap_is_413

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn streaming_upload_over_cap_is_413`.

**What it does** — With `max_upload_bytes = 8`, a 10-byte streamed body → 413.

### fn alias_and_links_endpoints

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn alias_and_links_endpoints`.

**What it does** — On a `linking_state`: a note whose body declares a bookmark
with an inline alias (`[Anchor1](### "Custom")`) and a link returns the derived
bookmark inline on the note (there is no dedicated bookmark endpoint — the body
is the source of truth); the links list has the content link; adding a manual
link makes two; a malformed reference (`not-a-ref`) is 422.

### fn alias_backlinks_and_resolve_endpoints

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn alias_backlinks_and_resolve_endpoints`.

**What it does** — Aliases live in a real notebook (Inbox notes carry none), so
both notes are placed in one: the target gets alias `note3` via the alias
endpoint; a source note linking `#note3` appears in the target's backlinks; a
3-segment `?ref=` resolves to the target note plus bookmark number 1.

### fn alias_conflicts_endpoint

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn alias_conflicts_endpoint`.

**What it does** — On a plain `FsBackend` state (no `LinkingBackend`, so no
write-time uniqueness check) the same alias planted on two notes in one real
notebook — the way a cross-device sync collision would appear —
`GET /api/aliases/conflicts` reports one note-conflict group (`dup`, 2
entities) and no notebook conflicts.

### fn state_with_events

**Identification** — helper; marker `// md:mod tests > fn state_with_events`.

**What it does** — An `AppState` whose backend is an `EventBackend` over
`FsBackend`, so mutations made through the router publish to the same `events`
channel the WebSocket route subscribes to.

### fn websocket_streams_note_create

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn websocket_streams_note_create`.

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

**What it does** — `GET /api/notes/:id/presence` (collaborative presence,
design §7.3): who is inside the note's live session and where their caret is
(`collab.presence(id)`). Empty list when collaboration is disabled or nobody is
in — never an error.

---

## CursorBody

**Identification** — `#[derive(Deserialize)] struct CursorBody`. Marker
`// md:CursorBody`.

**What it does** — `{ "line_id": <uuid>, "column": n }` — a caret position in
the line-based collab protocol.

---

## fn set_cursor

**Identification** — `async fn set_cursor(State, Path<Uuid>, Json<CursorBody>)
-> StatusCode`. Marker `// md:fn set_cursor`.

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
| 1 | module doc + imports | `// md:Overview` |
| 2–3 | `AppState`, `Shared` | `// md:AppState`, `// md:Shared` |
| 4 | `fn router` | `// md:fn router` |
| 5–6 | search (`SearchParams`, `search_notes`) | `// md:SearchParams`, `// md:fn search_notes` |
| 7–8 | middleware (`auth_mw`, `status_mw`) | `// md:fn auth_mw`, `// md:fn status_mw` |
| 9–12 | `ApiError` + its three impls | `// md:ApiError`, `// md:impl …` |
| 13–15 | pagination (`Pagination`, `Page`, `fn page`) | `// md:…` |
| 16–18 | ops probes (`health`, `ready`, `metrics`) | `// md:fn …` |
| 19–27 | notes CRUD + note-tags | `// md:CreateNote`, `// md:fn …` |
| 28–35 | aliases & links | `// md:AliasBody` … `// md:fn list_backlinks` |
| 36–43 | listing/pin/star/reorder | `// md:fn …`, `// md:ReorderBody` |
| 44–46 | resolve + conflicts | `// md:ResolveQuery` … |
| 47–59 | history & revert (5 shapes, 6 handlers, 2 DTO fns) | `// md:…` |
| 60–68 | permission proxies (`proxy_perm` + 8 routes) | `// md:fn proxy_…` |
| 69–84 | interop (DTOs, impls, 10 handlers/shapes) | `// md:…` |
| 85–92 | notebooks | `// md:TitleOnly` … `// md:fn set_notebook_alias` |
| 93–98 | tags | `// md:fn list_tags` … `// md:fn delete_tag` |
| 99–105 | resources (incl. streaming upload) | `// md:ResourceMeta` … `// md:fn delete_resource` |
| 106–107 | sync (`SyncSummary`, `fn sync`) | `// md:…` |
| 108–109 | WebSocket feed (`ws_handler`, `stream_changes`) | `// md:fn …` |
| 110 | `mod tests` (+ 5 helpers + 19 tests) | `// md:mod tests` (+ `> fn …`) |
| 111–113 | collab presence (`note_presence`, `CursorBody`, `set_cursor`) | `// md:…` |
