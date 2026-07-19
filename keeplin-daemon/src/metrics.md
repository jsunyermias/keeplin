# `metrics.rs` — operational metrics

Self-contained companion for `keeplin-daemon/src/metrics.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must
be able to understand it without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

```rust
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
// … async_trait, chrono, uuid; keeplin-core error/model/storage-trait types
```

**What it does** — A lightweight counter registry (`Metrics`) plus a
`StorageBackend` decorator (`MetricsBackend<B>`) that counts every storage
operation, exported in Prometheus text format at `/api/metrics`. Why a
decorator: like `EventBackend`, one instance sits behind **both** gRPC and
REST, so an operation from either surface is counted exactly once. Placement:
the **outermost** decorator —
`MetricsBackend(EventBackend(LinkingBackend([EncryptedBackend](Fs|Db))))` —
so it counts logical operations as a client issues them, not the extra inner
reads those layers perform. Measured series:
`keeplin_storage_operations_total{entity,op}` (successes),
`keeplin_storage_errors_total`, `keeplin_sync_changes_applied_total`, and
`keeplin_http_requests_total{status}` (fed by `rest.rs`'s middleware, not the
decorator). Counters only increase and labels are fixed literals — no user
content — so scraping is safe behind the existing Basic-Auth gate.

**Dependencies** — `std::sync::atomic`, `async_trait`; keeplin-core types.

**Used by** — `main.rs` (stack assembly + `Arc<Metrics>` in REST state);
`rest.rs` (`record_http_status`, the `/api/metrics` handler).

**Repeated context** — decorator conventions: implement every sub-trait,
delegate defaulted methods explicitly, never let instrumentation change
behaviour.

---

## OPERATION_LABELS

**Identification** — `const OPERATION_LABELS: &[(&str, &str)]`; marker
`// md:OPERATION_LABELS`.

**What it does** — The fixed `(entity, op)` pairs pre-registered in
`Metrics::operations` (22 series over note/notebook/tag/resource/note_tag ×
create/read/update/delete/list/add/remove). A fixed list means incrementing
never allocates or locks, and the export always lists every series (a `0` is
as informative as a positive count to a scraper).

**Used by** — `Metrics::new`, `render_prometheus`.

---

## HTTP_STATUS_CLASSES

**Identification** — `const HTTP_STATUS_CLASSES: &[&str]`; marker
`// md:HTTP_STATUS_CLASSES`.

**What it does** — `["2xx", "4xx", "5xx", "other"]`.

**Used by** — `Metrics::new`, `record_http_status`, `render_prometheus`.

---

## Metrics

**Identification** — `pub struct Metrics`; marker `// md:Metrics`.

**What it does** — The process-lifetime registry shared behind an `Arc`
between the decorator, the REST middleware, and the `/api/metrics` handler:
`operations` (pre-populated map of `AtomicU64`s), `errors`,
`sync_changes_applied`, `http_requests`. Every counter uses **`Relaxed`**
ordering: metrics need no happens-before with the operations they count, only
eventual accuracy, so the cheapest atomic is correct.

**Used by** — everything in this file plus `rest.rs`.

---

## impl Default for Metrics

**Identification** — marker `// md:impl Default for Metrics`.

**What it does** — Delegates to `new`.

---

## impl Metrics

**Identification** — inherent impl; marker `// md:impl Metrics`. Six methods.

### fn new

**Identification** — marker `// md:impl Metrics > fn new`.

**What it does** — Every known counter pre-registered at zero from the two
label lists.

### fn incr_op

**Identification** — marker `// md:impl Metrics > fn incr_op`.

**What it does** — Bumps one `(entity, op)` counter; unknown pairs (impossible
from the decorator) are ignored.

### fn incr_error

**Identification** — marker `// md:impl Metrics > fn incr_error`.

**What it does** — Bumps the shared error counter.

### fn add_sync_applied

**Identification** — marker `// md:impl Metrics > fn add_sync_applied`.

**What it does** — Adds `n` applied remote changes.

### fn record_http_status

**Identification** — `pub fn record_http_status(&self, status: u16)`; marker
`// md:impl Metrics > fn record_http_status`.

**What it does** — Buckets a response into `2xx`/`4xx`/`5xx`/`other` (e.g.
`101` → other) and bumps it. Called by the REST middleware.

### fn render_prometheus

**Identification** — `pub fn render_prometheus(&self) -> String`; marker
`// md:impl Metrics > fn render_prometheus`.

**What it does** — The whole registry in Prometheus text exposition format
(v0.0.4) with `# HELP`/`# TYPE` headers, series in a stable sorted order so
output is deterministic across scrapes and easy to diff in tests.

---

## MetricsBackend

**Identification** — `pub struct MetricsBackend<B>`; marker
`// md:MetricsBackend`.

**What it does** — The decorator: `inner: B` + `metrics: Arc<Metrics>`.

**Used by** — `main.rs` stack assembly.

---

## impl MetricsBackend

**Identification** — inherent impl; marker `// md:impl MetricsBackend`. Two
methods.

### fn new

**Identification** — marker `// md:impl MetricsBackend > fn new`.

**What it does** — Wraps `inner`; pass a clone of the same `Arc<Metrics>` the
daemon keeps in REST state so `/api/metrics` reads what this writes.

### fn record

**Identification** —
`fn record<T>(&self, entity, op, result: Result<T, StorageError>) -> Result<T, StorageError>`;
marker `// md:impl MetricsBackend > fn record`.

**What it does** — Bumps the operation counter on `Ok`, the error counter on
`Err`, and returns `result` unchanged so call sites stay one-liners.

---

## impl NoteRepository for MetricsBackend

**Identification** — marker `// md:impl NoteRepository for MetricsBackend`.

**What it does** — Every method delegates then `record`s:
create/read/update/delete under their own ops; the three listings under
`note`/`list`; `note_backlinks` under `note`/`read` (a specialised read —
counted there while still delegating so an inner indexed implementation is
reached); `notebook_sort_profile` delegates **unrecorded** (internal placement
metadata, not a user-facing operation).

---

## impl NotebookRepository for MetricsBackend

**Identification** — marker
`// md:impl NotebookRepository for MetricsBackend`.

**What it does** — The five methods recorded under `notebook`/*.

---

## impl TagRepository for MetricsBackend

**Identification** — marker `// md:impl TagRepository for MetricsBackend`.

**What it does** — Tag CRUD under `tag`/*; associations under
`note_tag`/`add`/`remove`/`list`.

---

## impl ResourceRepository for MetricsBackend

**Identification** — marker
`// md:impl ResourceRepository for MetricsBackend`.

**What it does** — Resource ops under `resource`/*; note
`purge_deleted_resources` records under `resource`/`purge` — a label pair
**not** in `OPERATION_LABELS`, so `incr_op` ignores it (only errors would
count). Documented as-is; the export therefore never shows a purge series.

---

## impl SyncBackend for MetricsBackend

**Identification** — marker `// md:impl SyncBackend for MetricsBackend`.

**What it does** — Delegation, except `apply_change` bumps
`sync_changes_applied` on success so `/api/metrics` reflects inbound sync
traffic; the other methods carry no per-operation signal worth a series.

---

## impl HistoryRepository for MetricsBackend

**Identification** — marker
`// md:impl HistoryRepository for MetricsBackend`.

**What it does** — Pure delegation.

---

## mod tests

**Identification** — `#[cfg(test)] mod tests`; marker `// md:mod tests`. One
helper + three tests.

**What it does** — Pins counting, error separation, sync counting, and status
bucketing.

### fn backend

**Identification** — helper; marker `// md:mod tests > fn backend`.

**What it does** — `MetricsBackend<FsBackend>` over a leaked tempdir + the
shared registry.

### fn counts_operations_and_errors

**Identification** — tokio test; marker
`// md:mod tests > fn counts_operations_and_errors`.

**What it does** — create/read/list each count 1; a read of a missing note
counts one **error**, not a read.

### fn counts_applied_sync_changes

**Identification** — tokio test; marker
`// md:mod tests > fn counts_applied_sync_changes`.

**What it does** — One applied `NoteCreate` → `…sync_changes_applied_total 1`.

### fn http_status_buckets

**Identification** — unit test; marker
`// md:mod tests > fn http_status_buckets`.

**What it does** — 200/204 → 2xx=2; 404 → 4xx; 503 → 5xx; 101 → other.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `Metrics`, `MetricsBackend<B>` — defined here (EXTRACTED)
- the six trait implementations (implements×6) (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs`, `models.rs`, `storage/backend.rs` (EXTRACTED: references×38/×26/×4)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/main.rs` — stack assembly (INFERRED)
- `keeplin-daemon/src/rest.rs` — HTTP middleware + `/api/metrics` (INFERRED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `const OPERATION_LABELS` | `// md:OPERATION_LABELS` |
| 3 | `const HTTP_STATUS_CLASSES` | `// md:HTTP_STATUS_CLASSES` |
| 4 | `struct Metrics` | `// md:Metrics` |
| 5 | `impl Default for Metrics` | `// md:impl Default for Metrics` |
| 6 | `impl Metrics` (+ six methods) | `// md:impl Metrics` (+ `> fn …`) |
| 7 | `struct MetricsBackend` | `// md:MetricsBackend` |
| 8 | `impl MetricsBackend` (+ `new`, `record`) | `// md:impl MetricsBackend` (+ `> fn …`) |
| 9–14 | the six trait impls | `// md:impl <Trait> for MetricsBackend` |
| 15 | `mod tests` (+ helper + three tests) | `// md:mod tests` (+ `> fn …`) |
