# `metrics.rs` — operational metrics

Self-contained companion for `keeplin-daemon/src/metrics.rs`. It documents **every
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

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use keeplin_core::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
    storage::{NoteRepository, NotebookRepository, ResourceRepository, SyncBackend, TagRepository},
};
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

**Code** — complete and verbatim:

```rust
// md:OPERATION_LABELS
const OPERATION_LABELS: &[(&str, &str)] = &[
    ("note", "create"),
    ("note", "read"),
    ("note", "update"),
    ("note", "delete"),
    ("note", "list"),
    ("notebook", "create"),
    ("notebook", "read"),
    ("notebook", "update"),
    ("notebook", "delete"),
    ("notebook", "list"),
    ("tag", "create"),
    ("tag", "read"),
    ("tag", "update"),
    ("tag", "delete"),
    ("tag", "list"),
    ("resource", "create"),
    ("resource", "read"),
    ("resource", "delete"),
    ("resource", "list"),
    ("note_tag", "add"),
    ("note_tag", "remove"),
    ("note_tag", "list"),
];
```

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

**Code** — complete and verbatim:

```rust
// md:HTTP_STATUS_CLASSES
const HTTP_STATUS_CLASSES: &[&str] = &["2xx", "4xx", "5xx", "other"];
```

**What it does** — `["2xx", "4xx", "5xx", "other"]`.

**Used by** — `Metrics::new`, `record_http_status`, `render_prometheus`.

---

## Metrics

**Identification** — `pub struct Metrics`; marker `// md:Metrics`.

**Code** — complete and verbatim:

```rust
// md:Metrics
pub struct Metrics {
    operations: HashMap<(&'static str, &'static str), AtomicU64>,
    errors: AtomicU64,
    sync_changes_applied: AtomicU64,
    http_requests: HashMap<&'static str, AtomicU64>,
}
```

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

**Code** — complete and verbatim:

```rust
// md:impl Default for Metrics
impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}
```

**What it does** — Delegates to `new`.

---

## impl Metrics

**Identification** — inherent impl; marker `// md:impl Metrics`. Six methods.

**Code** — container: members documented as sub-blocks below: fn new, fn incr_op, fn incr_error, fn add_sync_applied, fn record_http_status, fn render_prometheus.

### fn new

**Identification** — marker `// md:impl Metrics > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl Metrics > fn new
    pub fn new() -> Self {
        Self {
            operations: OPERATION_LABELS
                .iter()
                .map(|&labels| (labels, AtomicU64::new(0)))
                .collect(),
            errors: AtomicU64::new(0),
            sync_changes_applied: AtomicU64::new(0),
            http_requests: HTTP_STATUS_CLASSES
                .iter()
                .map(|&class| (class, AtomicU64::new(0)))
                .collect(),
        }
    }
```

**What it does** — Every known counter pre-registered at zero from the two
label lists.

### fn incr_op

**Identification** — marker `// md:impl Metrics > fn incr_op`.

**Code** — complete and verbatim:

```rust
    // md:impl Metrics > fn incr_op
    fn incr_op(&self, entity: &'static str, op: &'static str) {
        if let Some(counter) = self.operations.get(&(entity, op)) {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    }
```

**What it does** — Bumps one `(entity, op)` counter; unknown pairs (impossible
from the decorator) are ignored.

### fn incr_error

**Identification** — marker `// md:impl Metrics > fn incr_error`.

**Code** — complete and verbatim:

```rust
    // md:impl Metrics > fn incr_error
    fn incr_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }
```

**What it does** — Bumps the shared error counter.

### fn add_sync_applied

**Identification** — marker `// md:impl Metrics > fn add_sync_applied`.

**Code** — complete and verbatim:

```rust
    // md:impl Metrics > fn add_sync_applied
    fn add_sync_applied(&self, n: u64) {
        self.sync_changes_applied.fetch_add(n, Ordering::Relaxed);
    }
```

**What it does** — Adds `n` applied remote changes.

### fn record_http_status

**Identification** — `pub fn record_http_status(&self, status: u16)`; marker
`// md:impl Metrics > fn record_http_status`.

**Code** — complete and verbatim:

```rust
    // md:impl Metrics > fn record_http_status
    pub fn record_http_status(&self, status: u16) {
        let class = match status {
            200..=299 => "2xx",
            400..=499 => "4xx",
            500..=599 => "5xx",
            _ => "other",
        };
        if let Some(counter) = self.http_requests.get(class) {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    }
```

**What it does** — Buckets a response into `2xx`/`4xx`/`5xx`/`other` (e.g.
`101` → other) and bumps it. Called by the REST middleware.

### fn render_prometheus

**Identification** — `pub fn render_prometheus(&self) -> String`; marker
`// md:impl Metrics > fn render_prometheus`.

**Code** — complete and verbatim:

```rust
    // md:impl Metrics > fn render_prometheus
    pub fn render_prometheus(&self) -> String {
        let mut out = String::new();

        out.push_str("# HELP keeplin_storage_operations_total Successful storage operations.\n");
        out.push_str("# TYPE keeplin_storage_operations_total counter\n");
        let mut ops: Vec<_> = self.operations.iter().collect();
        ops.sort_by_key(|(labels, _)| **labels);
        for ((entity, op), counter) in ops {
            out.push_str(&format!(
                "keeplin_storage_operations_total{{entity=\"{entity}\",op=\"{op}\"}} {}\n",
                counter.load(Ordering::Relaxed)
            ));
        }

        out.push_str("# HELP keeplin_storage_errors_total Storage operations that errored.\n");
        out.push_str("# TYPE keeplin_storage_errors_total counter\n");
        out.push_str(&format!(
            "keeplin_storage_errors_total {}\n",
            self.errors.load(Ordering::Relaxed)
        ));

        out.push_str(
            "# HELP keeplin_sync_changes_applied_total Remote changes applied via sync.\n",
        );
        out.push_str("# TYPE keeplin_sync_changes_applied_total counter\n");
        out.push_str(&format!(
            "keeplin_sync_changes_applied_total {}\n",
            self.sync_changes_applied.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP keeplin_http_requests_total HTTP responses by status class.\n");
        out.push_str("# TYPE keeplin_http_requests_total counter\n");
        for &class in HTTP_STATUS_CLASSES {
            let count = self
                .http_requests
                .get(class)
                .map(|c| c.load(Ordering::Relaxed))
                .unwrap_or(0);
            out.push_str(&format!(
                "keeplin_http_requests_total{{status=\"{class}\"}} {count}\n"
            ));
        }

        out
    }
```

**What it does** — The whole registry in Prometheus text exposition format
(v0.0.4) with `# HELP`/`# TYPE` headers, series in a stable sorted order so
output is deterministic across scrapes and easy to diff in tests.

---

## MetricsBackend

**Identification** — `pub struct MetricsBackend<B>`; marker
`// md:MetricsBackend`.

**Code** — complete and verbatim:

```rust
// md:MetricsBackend
pub struct MetricsBackend<B> {
    inner: B,
    metrics: Arc<Metrics>,
}
```

**What it does** — The decorator: `inner: B` + `metrics: Arc<Metrics>`.

**Used by** — `main.rs` stack assembly.

---

## impl MetricsBackend

**Identification** — inherent impl; marker `// md:impl MetricsBackend`. Two
methods.

**Code** — container: members documented as sub-blocks below: fn new, fn record.

### fn new

**Identification** — marker `// md:impl MetricsBackend > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl MetricsBackend > fn new
    pub fn new(inner: B, metrics: Arc<Metrics>) -> Self {
        Self { inner, metrics }
    }
```

**What it does** — Wraps `inner`; pass a clone of the same `Arc<Metrics>` the
daemon keeps in REST state so `/api/metrics` reads what this writes.

### fn record

**Identification** —
`fn record<T>(&self, entity, op, result: Result<T, StorageError>) -> Result<T, StorageError>`;
marker `// md:impl MetricsBackend > fn record`.

**Code** — complete and verbatim:

```rust
    // md:impl MetricsBackend > fn record
    fn record<T>(
        &self,
        entity: &'static str,
        op: &'static str,
        result: Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        match &result {
            Ok(_) => self.metrics.incr_op(entity, op),
            Err(_) => self.metrics.incr_error(),
        }
        result
    }
```

**What it does** — Bumps the operation counter on `Ok`, the error counter on
`Err`, and returns `result` unchanged so call sites stay one-liners.

---

## impl NoteRepository for MetricsBackend

**Identification** — marker `// md:impl NoteRepository for MetricsBackend`.

**Code** — complete and verbatim:

```rust
// md:impl NoteRepository for MetricsBackend
#[async_trait]
impl<B: NoteRepository> NoteRepository for MetricsBackend<B> {
    async fn create_note(&self, note: Note) -> Result<Note, StorageError> {
        let r = self.inner.create_note(note).await;
        self.record("note", "create", r)
    }

    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        let r = self.inner.read_note(id).await;
        self.record("note", "read", r)
    }

    async fn update_note(&self, note: Note) -> Result<Note, StorageError> {
        let r = self.inner.update_note(note).await;
        self.record("note", "update", r)
    }

    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        let r = self.inner.delete_note(id).await;
        self.record("note", "delete", r)
    }

    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let r = self.inner.list_notes(page_size, page_token).await;
        self.record("note", "list", r)
    }

    async fn note_backlinks(
        &self,
        target_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let r = self
            .inner
            .note_backlinks(target_id, page_size, page_token)
            .await;
        self.record("note", "read", r)
    }

    async fn list_notes_in_notebook(
        &self,
        notebook_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let r = self
            .inner
            .list_notes_in_notebook(notebook_id, page_size, page_token)
            .await;
        self.record("note", "list", r)
    }

    async fn list_starred_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let r = self.inner.list_starred_notes(page_size, page_token).await;
        self.record("note", "list", r)
    }

    async fn notebook_sort_profile(
        &self,
        notebook_id: Uuid,
    ) -> Result<keeplin_core::storage::NotebookSortProfile, StorageError> {
        self.inner.notebook_sort_profile(notebook_id).await
    }
}
```

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

**Code** — complete and verbatim:

```rust
// md:impl NotebookRepository for MetricsBackend
#[async_trait]
impl<B: NotebookRepository> NotebookRepository for MetricsBackend<B> {
    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        let r = self.inner.create_notebook(notebook).await;
        self.record("notebook", "create", r)
    }

    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        let r = self.inner.read_notebook(id).await;
        self.record("notebook", "read", r)
    }

    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        let r = self.inner.update_notebook(notebook).await;
        self.record("notebook", "update", r)
    }

    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        let r = self.inner.delete_notebook(id).await;
        self.record("notebook", "delete", r)
    }

    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        let r = self.inner.list_notebooks(page_size, page_token).await;
        self.record("notebook", "list", r)
    }
}
```

**What it does** — The five methods recorded under `notebook`/*.

---

## impl TagRepository for MetricsBackend

**Identification** — marker `// md:impl TagRepository for MetricsBackend`.

**Code** — complete and verbatim:

```rust
// md:impl TagRepository for MetricsBackend
#[async_trait]
impl<B: TagRepository> TagRepository for MetricsBackend<B> {
    async fn create_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        let r = self.inner.create_tag(tag).await;
        self.record("tag", "create", r)
    }

    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        let r = self.inner.read_tag(id).await;
        self.record("tag", "read", r)
    }

    async fn update_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        let r = self.inner.update_tag(tag).await;
        self.record("tag", "update", r)
    }

    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        let r = self.inner.delete_tag(id).await;
        self.record("tag", "delete", r)
    }

    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let r = self.inner.list_tags(page_size, page_token).await;
        self.record("tag", "list", r)
    }

    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        let r = self.inner.add_note_tag(note_tag).await;
        self.record("note_tag", "add", r)
    }

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        let r = self.inner.remove_note_tag(note_id, tag_id).await;
        self.record("note_tag", "remove", r)
    }

    async fn list_note_tags(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let r = self
            .inner
            .list_note_tags(note_id, page_size, page_token)
            .await;
        self.record("note_tag", "list", r)
    }
}
```

**What it does** — Tag CRUD under `tag`/*; associations under
`note_tag`/`add`/`remove`/`list`.

---

## impl ResourceRepository for MetricsBackend

**Identification** — marker
`// md:impl ResourceRepository for MetricsBackend`.

**Code** — complete and verbatim:

```rust
// md:impl ResourceRepository for MetricsBackend
#[async_trait]
impl<B: ResourceRepository> ResourceRepository for MetricsBackend<B> {
    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let r = self.inner.create_resource(resource, data).await;
        self.record("resource", "create", r)
    }

    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let r = self.inner.read_resource(id).await;
        self.record("resource", "read", r)
    }

    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        let r = self.inner.delete_resource(id).await;
        self.record("resource", "delete", r)
    }

    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let r = self.inner.list_resources(page_size, page_token).await;
        self.record("resource", "list", r)
    }

    async fn purge_deleted_resources(
        &self,
        older_than: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, StorageError> {
        let r = self.inner.purge_deleted_resources(older_than).await;
        self.record("resource", "purge", r)
    }
}
```

**What it does** — Resource ops under `resource`/*; note
`purge_deleted_resources` records under `resource`/`purge` — a label pair
**not** in `OPERATION_LABELS`, so `incr_op` ignores it (only errors would
count). Documented as-is; the export therefore never shows a purge series.

---

## impl SyncBackend for MetricsBackend

**Identification** — marker `// md:impl SyncBackend for MetricsBackend`.

**Code** — complete and verbatim:

```rust
// md:impl SyncBackend for MetricsBackend
#[async_trait]
impl<B: SyncBackend> SyncBackend for MetricsBackend<B> {
    async fn get_device_id(&self) -> Result<String, StorageError> {
        self.inner.get_device_id().await
    }

    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
        self.inner.get_last_sync_time().await
    }

    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        self.inner.update_sync_time(ts).await
    }

    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError> {
        self.inner.get_changes_since(since).await
    }

    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        let r = self.inner.apply_change(change).await;
        if r.is_ok() {
            self.metrics.add_sync_applied(1);
        }
        r
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
```

**What it does** — Delegation, except `apply_change` bumps
`sync_changes_applied` on success so `/api/metrics` reflects inbound sync
traffic; the other methods carry no per-operation signal worth a series.

---

## impl HistoryRepository for MetricsBackend

**Identification** — marker
`// md:impl HistoryRepository for MetricsBackend`.

**Code** — complete and verbatim:

```rust
// md:impl HistoryRepository for MetricsBackend
#[async_trait]
impl<B: keeplin_core::storage::HistoryRepository> keeplin_core::storage::HistoryRepository
    for MetricsBackend<B>
{
    async fn note_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<keeplin_core::storage::EntityVersion<Note>>, StorageError> {
        self.inner.note_history(id, limit).await
    }

    async fn notebook_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<keeplin_core::storage::EntityVersion<Notebook>>, StorageError> {
        self.inner.notebook_history(id, limit).await
    }
}
```

**What it does** — Pure delegation.

---

## mod tests

**Identification** — `#[cfg(test)] mod tests`; marker `// md:mod tests`. One
helper + three tests.

**Code** — container: members documented as sub-blocks below: fn backend, fn counts_operations_and_errors, fn counts_applied_sync_changes, fn http_status_buckets.

**What it does** — Pins counting, error separation, sync counting, and status
bucketing.

The explicit `imports` leaf below preserves the test-module dependency preamble
verbatim.

### imports

**Identification** — test-module dependencies; marker `// md:mod tests > imports`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > imports
    use super::*;
    use keeplin_core::storage::fs::FsBackend;
```

**What it does** — Brings the parent module API and test-only dependencies into
scope.

### fn backend

**Identification** — helper; marker `// md:mod tests > fn backend`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn backend
    async fn backend() -> (MetricsBackend<FsBackend>, Arc<Metrics>) {
        let dir = tempfile::tempdir().unwrap();
        let fs = FsBackend::new(dir.path()).await.unwrap();
        std::mem::forget(dir);
        let metrics = Arc::new(Metrics::new());
        (MetricsBackend::new(fs, metrics.clone()), metrics)
    }
```

**What it does** — `MetricsBackend<FsBackend>` over a leaked tempdir + the
shared registry.

### fn counts_operations_and_errors

**Identification** — tokio test; marker
`// md:mod tests > fn counts_operations_and_errors`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn counts_operations_and_errors
    #[tokio::test]
    async fn counts_operations_and_errors() {
        let (be, metrics) = backend().await;

        let note = be.create_note(Note::new("t", "b")).await.unwrap();
        be.read_note(note.id).await.unwrap();
        be.list_notes(0, None).await.unwrap();
        assert!(be.read_note(Uuid::new_v4()).await.is_err());

        let text = metrics.render_prometheus();
        assert!(text.contains("keeplin_storage_operations_total{entity=\"note\",op=\"create\"} 1"));
        assert!(text.contains("keeplin_storage_operations_total{entity=\"note\",op=\"read\"} 1"));
        assert!(text.contains("keeplin_storage_operations_total{entity=\"note\",op=\"list\"} 1"));
        assert!(text.contains("keeplin_storage_errors_total 1"));
    }
```

**What it does** — create/read/list each count 1; a read of a missing note
counts one **error**, not a read.

### fn counts_applied_sync_changes

**Identification** — tokio test; marker
`// md:mod tests > fn counts_applied_sync_changes`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn counts_applied_sync_changes
    #[tokio::test]
    async fn counts_applied_sync_changes() {
        let (be, metrics) = backend().await;
        let remote = Note::new("remote", "peer");
        be.apply_change(Change::NoteCreate { note: remote })
            .await
            .unwrap();
        assert!(metrics
            .render_prometheus()
            .contains("keeplin_sync_changes_applied_total 1"));
    }
```

**What it does** — One applied `NoteCreate` → `…sync_changes_applied_total 1`.

### fn http_status_buckets

**Identification** — unit test; marker
`// md:mod tests > fn http_status_buckets`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn http_status_buckets
    #[test]
    fn http_status_buckets() {
        let metrics = Metrics::new();
        metrics.record_http_status(200);
        metrics.record_http_status(204);
        metrics.record_http_status(404);
        metrics.record_http_status(503);
        metrics.record_http_status(101);
        let text = metrics.render_prometheus();
        assert!(text.contains("keeplin_http_requests_total{status=\"2xx\"} 2"));
        assert!(text.contains("keeplin_http_requests_total{status=\"4xx\"} 1"));
        assert!(text.contains("keeplin_http_requests_total{status=\"5xx\"} 1"));
        assert!(text.contains("keeplin_http_requests_total{status=\"other\"} 1"));
    }
```

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
| 1 | `Overview` | `// md:Overview` |
| 2 | `OPERATION_LABELS` | `// md:OPERATION_LABELS` |
| 3 | `HTTP_STATUS_CLASSES` | `// md:HTTP_STATUS_CLASSES` |
| 4 | `Metrics` | `// md:Metrics` |
| 5 | `impl Default for Metrics` | `// md:impl Default for Metrics` |
| 6 | `impl Metrics` (container) | `// md:impl Metrics` |
| 7 | `fn new` | `// md:impl Metrics > fn new` |
| 8 | `fn incr_op` | `// md:impl Metrics > fn incr_op` |
| 9 | `fn incr_error` | `// md:impl Metrics > fn incr_error` |
| 10 | `fn add_sync_applied` | `// md:impl Metrics > fn add_sync_applied` |
| 11 | `fn record_http_status` | `// md:impl Metrics > fn record_http_status` |
| 12 | `fn render_prometheus` | `// md:impl Metrics > fn render_prometheus` |
| 13 | `MetricsBackend` | `// md:MetricsBackend` |
| 14 | `impl MetricsBackend` (container) | `// md:impl MetricsBackend` |
| 15 | `fn new` | `// md:impl MetricsBackend > fn new` |
| 16 | `fn record` | `// md:impl MetricsBackend > fn record` |
| 17 | `impl NoteRepository for MetricsBackend` | `// md:impl NoteRepository for MetricsBackend` |
| 18 | `impl NotebookRepository for MetricsBackend` | `// md:impl NotebookRepository for MetricsBackend` |
| 19 | `impl TagRepository for MetricsBackend` | `// md:impl TagRepository for MetricsBackend` |
| 20 | `impl ResourceRepository for MetricsBackend` | `// md:impl ResourceRepository for MetricsBackend` |
| 21 | `impl SyncBackend for MetricsBackend` | `// md:impl SyncBackend for MetricsBackend` |
| 22 | `impl HistoryRepository for MetricsBackend` | `// md:impl HistoryRepository for MetricsBackend` |
| 23 | `mod tests` (container) | `// md:mod tests` |
| 24 | `imports` | `// md:mod tests > imports` |
| 25 | `fn backend` | `// md:mod tests > fn backend` |
| 26 | `fn counts_operations_and_errors` | `// md:mod tests > fn counts_operations_and_errors` |
| 27 | `fn counts_applied_sync_changes` | `// md:mod tests > fn counts_applied_sync_changes` |
| 28 | `fn http_status_buckets` | `// md:mod tests > fn http_status_buckets` |
