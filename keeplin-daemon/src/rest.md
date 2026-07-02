# `rest.rs` — REST/JSON API + WebSocket feed (axum)

## Purpose

Serves the same operations as the gRPC service over plain HTTP with JSON bodies (straight over
the `keeplin-core` domain models — no protobuf), plus the WebSocket live-change feed. It runs
on an optional second listener (`http_addr`) alongside the gRPC server, sharing one backend
`Arc` and one auth model. axum 0.7 runs on hyper 1.x, the same as tonic 0.12, so the two
coexist.

## State

`AppState` (behind `Arc`, the axum `State`):

| Field | Purpose |
|-------|---------|
| `backend: Arc<dyn StorageBackend>` | shared with the gRPC server (top of the decorator stack) |
| `events: broadcast::Sender<Change>` | the live-feed channel; each WS connection subscribes |
| `metrics: Arc<crate::metrics::Metrics>` | operational counters, shared with the outermost `MetricsBackend` decorator; rendered by `GET /metrics` |
| `max_body_bytes: usize` | request-body cap (from `max_message_size`), raising axum's 2 MiB default |
| `journal_retention_days: u64` | days of change-journal history to keep; `POST /sync` prunes older rows |
| `auth_username` / `auth_password` | Basic-Auth credentials (both `Some` → auth required) |

## Endpoints

The router is two sub-routers merged under `/api`: **operational** endpoints
(`/health`, `/ready`, `/metrics`) sit *outside* the auth middleware and the HTTP-status
counter (probes/scrapers cannot authenticate, and their traffic must not inflate the request
metrics); every other route is behind auth and counted by `status_mw`.

| Method & path | Auth | Purpose |
|---------------|------|---------|
| `GET /health` | none | liveness (`200 ok`); does not touch storage |
| `GET /ready` | none | readiness: one `list_notes(1)` probe → `200 ready` or `503` when storage is unreachable |
| `GET /metrics` | none | Prometheus exposition (`text/plain; version=0.0.4`) — see `metrics.md` |
| `GET/POST /notes`, `GET/PUT/DELETE /notes/:id` | note CRUD (cursor pagination on list) |
| `GET /notes/:id/tags`, `PUT/DELETE /notes/:note_id/tags/:tag_id` | note↔tag associations |
| `PUT /notes/:id/alias`, `PUT /notebooks/:id/alias` | set/clear an alias |
| `GET/POST /notes/:id/links`, `DELETE /notes/:id/links/:index` | list / add-manual / remove links |
| `GET /notes/:id/backlinks?page_size=&page_token=` | notes linking **to** this note (paginated) |
| `GET /links/resolve?ref=#…` | resolve a reference → `{ note_id, bookmark_number }` |
| `GET /aliases/conflicts` | aliases shared by 2+ live entities (sync collisions) |
| `GET/POST/PUT/DELETE /notebooks`, `/tags` | notebook / tag CRUD |
| `GET/POST /resources`, `GET/PUT/DELETE /resources/:id`, `GET /resources/:id/data` | resource metadata CRUD + raw upload/download |
| `POST /sync` | run one sync cycle → `{ "applied": n }`, then prune journal rows older than `journal_retention_days` (shared `server::prune_journal_after_sync`) |
| `GET /ws` | upgrade to the WebSocket live-change feed |

## Auth middleware

`auth_mw` mirrors the gRPC interceptor: when both credentials are configured it requires a
valid `Authorization: Basic …` header (via `crate::auth::verify_basic`), returning `401` +
`WWW-Authenticate: Basic` otherwise; when unconfigured it is a no-op.

## Error mapping (`ApiError`)

| `StorageError` | HTTP status |
|----------------|-------------|
| `NotFound` | `404` |
| `Conflict` (duplicate alias) | `409` |
| `CorruptedData` / invalid link ref | `422` |
| invalid UUID / body | `400` (axum extractor rejection) |
| anything else | `500` |

Reads **and updates** of a soft-deleted note/notebook/tag return `404` (the gRPC `Get` RPCs
still return the tombstone for sync — a deliberate divergence — but the `Update` RPCs reject
it with `NOT_FOUND` too). Without the update guard, a `PUT` whose body defaults `deleted_at`
to null would silently *revive* the entity; revival is reserved for the sync path
(`apply_change` resolving a causal edit made after the delete). The alias/link endpoints
inherit the same rule from the `linking` helpers.

## WebSocket feed (`GET /api/ws`)

On upgrade the handler `subscribe()`s to `events` and forwards each `Change` as a JSON text
frame; on `Lagged` it sends `{"type":"resync"}`; a client close frame ends the loop. The
upgrade request passes through the same auth middleware. See `event_backend.md` for what is
published.

## Resource upload

`POST /api/resources?title=&file_name=` with the raw file bytes as the body and `Content-Type`
as the MIME type. The body is capped at `max_body_bytes` (= `max_message_size`, 32 MiB default)
via `DefaultBodyLimit`, matching gRPC.

## Tests

Because the daemon is a binary crate, integration tests can't reach these internals, so tests
live **inline** (`#[cfg(test)] mod tests`) and drive the router in-process with
`tower::ServiceExt::oneshot`; the WebSocket test opens a real socket.

## Related files

- `keeplin-daemon/src/main.rs` — builds `AppState` and serves this router next to gRPC.
- `keeplin-daemon/src/event_backend.rs` — the feed source.
- `keeplin-daemon/src/metrics.rs` — the counter registry and `MetricsBackend` behind `/metrics`.
- `keeplin-daemon/src/auth.rs` — the shared Basic-Auth check.
- `keeplin-core/src/linking.rs` — the bookmark/link/alias helpers the routes call.
