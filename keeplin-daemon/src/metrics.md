# `keeplin-daemon/src/metrics.rs` — operational metrics

## Purpose

Process-lifetime counters for observability, exported over HTTP in Prometheus text format at
`GET /api/metrics`. Two pieces:

- **`Metrics`** — a small registry of atomic counters, shared behind an `Arc`.
- **`MetricsBackend<B>`** — a `StorageBackend` decorator that records every storage operation
  into a `Metrics`, then delegates to the inner backend.

## Why a decorator (and where it sits)

Like `EventBackend` and `EncryptedBackend`, `MetricsBackend` *is* a `StorageBackend`, so one
instance sits behind **both** the gRPC service and the REST API and each operation is counted
exactly once — no per-surface instrumentation to keep in sync.

It is the **outermost** decorator:

```
MetricsBackend( EventBackend( LinkingBackend( [EncryptedBackend]( Fs | Db ) ) ) )
```

so it counts logical operations as a client issues them (one note create = one
`note`/`create`), not the extra inner reads `LinkingBackend` performs to resolve links or
enforce alias uniqueness.

`main` constructs it with a clone of the same `Arc<Metrics>` it stores in the REST `AppState`,
so `GET /api/metrics` renders exactly the counters the decorator writes.

## Exported series

| Metric | Type | Labels | Source |
|--------|------|--------|--------|
| `keeplin_storage_operations_total` | counter | `entity` (`note`/`notebook`/`tag`/`resource`/`note_tag`), `op` (`create`/`read`/`update`/`delete`/`list`/`add`/`remove`) | `MetricsBackend`, on `Ok` |
| `keeplin_storage_errors_total` | counter | — | `MetricsBackend`, on `Err` |
| `keeplin_sync_changes_applied_total` | counter | — | `MetricsBackend::apply_change`, on `Ok` |
| `keeplin_http_requests_total` | counter | `status` (`2xx`/`4xx`/`5xx`/`other`) | REST `status_mw` middleware |

Every `(entity, op)` series is pre-registered at zero (`OPERATION_LABELS`), so incrementing
never allocates or locks and the export always lists every series. Counters use `Relaxed`
atomics — metrics need eventual accuracy, not a happens-before relationship with the
operations they count.

`note_backlinks` is counted under `note`/`read` (it is a specialised read) and delegates to
the inner backend so an indexed implementation is still reached.

## Endpoints (served by `rest.rs`)

| Route | Auth | Behaviour |
|-------|------|-----------|
| `GET /api/health` | none | Liveness. Always `200 ok`; does not touch the backend. |
| `GET /api/ready` | none | Readiness. One `list_notes(1)` backend probe → `200 ready`, or `503` with the error when storage is unreachable. |
| `GET /api/metrics` | none | Prometheus exposition (`text/plain; version=0.0.4`). |

These three sit **outside** the Basic-Auth middleware (probes and scrapers cannot present
credentials; the endpoints carry no user content) and **outside** the HTTP-status counter, so
frequent probe/scrape traffic does not inflate `keeplin_http_requests_total`. The readiness
probe's `list_notes` does flow through the decorator, so a busy readiness schedule contributes
to the `note`/`list` counter.

## Related files

- `keeplin-daemon/src/rest.rs` — the `/health`, `/ready`, `/metrics` routes, the `status_mw`
  middleware, and the router split (operational vs. auth-gated data API).
- `keeplin-daemon/src/main.rs` — builds the decorator stack and shares the `Arc<Metrics>`.
- `keeplin-daemon/src/event_backend.rs` — the sibling decorator this one is modelled on.
