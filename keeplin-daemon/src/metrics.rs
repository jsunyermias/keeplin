//! Operational metrics: a lightweight counter registry plus a [`StorageBackend`] decorator
//! that records every storage operation, exported in Prometheus text format.
//!
//! # Why a decorator
//!
//! [`MetricsBackend<B>`] wraps any `B: StorageBackend` and increments a counter after each
//! call, then delegates. Because it is itself a `StorageBackend` (like
//! [`crate::event_backend::EventBackend`] and
//! [`keeplin_core::encryption::EncryptedBackend`]), one instance sits behind **both** the
//! gRPC service and the REST API, so an operation from either surface is counted exactly
//! once — no per-surface instrumentation to keep in sync.
//!
//! # Placement in the decorator stack
//!
//! `MetricsBackend` is the **outermost** decorator —
//! `MetricsBackend(EventBackend(LinkingBackend([EncryptedBackend](Fs|Db))))` — so it counts
//! logical operations as a client issues them (a note create is one `note`/`create`, after
//! link derivation and decryption), not the extra inner reads those layers perform.
//!
//! # What is measured
//!
//! - `keeplin_storage_operations_total{entity,op}` — successful storage calls, by entity
//!   (`note`/`notebook`/`tag`/`resource`/`note_tag`) and operation
//!   (`create`/`read`/`update`/`delete`/`list`/`add`/`remove`).
//! - `keeplin_storage_errors_total` — storage calls that returned an error.
//! - `keeplin_sync_changes_applied_total` — remote changes applied via `apply_change`.
//! - `keeplin_http_requests_total{status}` — HTTP responses by status class (`2xx`/`4xx`/
//!   `5xx`/`other`), fed by [`crate::rest`]'s middleware, not the decorator.
//!
//! Counters only ever increase and carry no user content (labels are fixed literals), so the
//! `/api/metrics` endpoint is safe to scrape without authentication concerns beyond the
//! existing Basic-Auth gate.

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

/// The `(entity, op)` label pairs pre-registered in [`Metrics::operations`]. Kept as a fixed
/// list so every counter exists up front: incrementing never allocates or locks, and the
/// export always lists every series (a `0` is as informative as a positive count to a
/// scraper).
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

/// The HTTP status classes tracked by [`Metrics::http_requests`].
const HTTP_STATUS_CLASSES: &[&str] = &["2xx", "4xx", "5xx", "other"];

/// A process-lifetime counter registry, shared (behind an `Arc`) between the
/// [`MetricsBackend`] decorator, the REST middleware, and the `/api/metrics` handler.
///
/// Every counter is an [`AtomicU64`] touched with `Relaxed` ordering: metrics need no
/// happens-before relationship with the operations they count, only eventual accuracy, so
/// the cheapest atomic is correct here.
pub struct Metrics {
    /// Successful storage operations keyed by `(entity, op)`. Pre-populated from
    /// [`OPERATION_LABELS`], so `incr_op` only ever looks up an existing atomic.
    operations: HashMap<(&'static str, &'static str), AtomicU64>,
    /// Storage calls that returned `Err`.
    errors: AtomicU64,
    /// Remote changes applied through `apply_change`.
    sync_changes_applied: AtomicU64,
    /// HTTP responses keyed by status class (`2xx`/`4xx`/`5xx`/`other`).
    http_requests: HashMap<&'static str, AtomicU64>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    /// Build a registry with every known counter pre-registered at zero.
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

    /// Record one successful storage operation. Unknown `(entity, op)` pairs (which cannot
    /// arise from the decorator, whose labels are all in [`OPERATION_LABELS`]) are ignored.
    fn incr_op(&self, entity: &'static str, op: &'static str) {
        if let Some(counter) = self.operations.get(&(entity, op)) {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record one storage error.
    fn incr_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Record `n` remote changes applied by a sync cycle.
    fn add_sync_applied(&self, n: u64) {
        self.sync_changes_applied.fetch_add(n, Ordering::Relaxed);
    }

    /// Record one HTTP response, bucketed by status class. Called by the REST middleware.
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

    /// Render the whole registry in Prometheus text exposition format (v0.0.4).
    ///
    /// Series are emitted in a stable order (the fixed label lists, sorted) so the output is
    /// deterministic across scrapes and easy to diff in tests.
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
}

/// A [`StorageBackend`] decorator that records each operation into a shared [`Metrics`].
pub struct MetricsBackend<B> {
    inner: B,
    metrics: Arc<Metrics>,
}

impl<B> MetricsBackend<B> {
    /// Wrap `inner`, recording into `metrics`. Pass a clone of the same `Arc<Metrics>` the
    /// daemon keeps in its REST state so `/api/metrics` reads the counters this decorator
    /// writes.
    pub fn new(inner: B, metrics: Arc<Metrics>) -> Self {
        Self { inner, metrics }
    }

    /// Record `result` under `(entity, op)`: bump the operation counter on `Ok`, the shared
    /// error counter on `Err`. Returns `result` unchanged so call sites stay one-liners.
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
}

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
        // A backlink lookup is a specialised read; count it under the note/read series so the
        // decorator delegates to (and does not shadow) an inner indexed implementation.
        let r = self
            .inner
            .note_backlinks(target_id, page_size, page_token)
            .await;
        self.record("note", "read", r)
    }
}

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
}

// Sync methods delegate unchanged, except `apply_change` bumps the applied-changes counter so
// `/api/metrics` reflects inbound sync traffic. The other methods carry no per-operation
// signal worth a dedicated series here.
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

#[cfg(test)]
mod tests {
    use super::*;
    use keeplin_core::storage::fs::FsBackend;

    async fn backend() -> (MetricsBackend<FsBackend>, Arc<Metrics>) {
        let dir = tempfile::tempdir().unwrap();
        let fs = FsBackend::new(dir.path()).await.unwrap();
        std::mem::forget(dir);
        let metrics = Arc::new(Metrics::new());
        (MetricsBackend::new(fs, metrics.clone()), metrics)
    }

    #[tokio::test]
    async fn counts_operations_and_errors() {
        let (be, metrics) = backend().await;

        let note = be.create_note(Note::new("t", "b")).await.unwrap();
        be.read_note(note.id).await.unwrap();
        be.list_notes(0, None).await.unwrap();
        // A read of a missing note is one error, not a note/read.
        assert!(be.read_note(Uuid::new_v4()).await.is_err());

        let text = metrics.render_prometheus();
        assert!(text.contains("keeplin_storage_operations_total{entity=\"note\",op=\"create\"} 1"));
        assert!(text.contains("keeplin_storage_operations_total{entity=\"note\",op=\"read\"} 1"));
        assert!(text.contains("keeplin_storage_operations_total{entity=\"note\",op=\"list\"} 1"));
        assert!(text.contains("keeplin_storage_errors_total 1"));
    }

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

    #[test]
    fn http_status_buckets() {
        let metrics = Metrics::new();
        metrics.record_http_status(200);
        metrics.record_http_status(204);
        metrics.record_http_status(404);
        metrics.record_http_status(503);
        metrics.record_http_status(101); // upgrade → "other"
        let text = metrics.render_prometheus();
        assert!(text.contains("keeplin_http_requests_total{status=\"2xx\"} 2"));
        assert!(text.contains("keeplin_http_requests_total{status=\"4xx\"} 1"));
        assert!(text.contains("keeplin_http_requests_total{status=\"5xx\"} 1"));
        assert!(text.contains("keeplin_http_requests_total{status=\"other\"} 1"));
    }
}
