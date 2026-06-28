//! Integration tests for change propagation between two storage backends.
//!
//! These tests model two independent devices, each backed by its own `DbBackend`
//! database file, and verify that changes recorded on one device can be collected
//! with [`SyncBackend::get_changes_since`] and replayed on the other device with
//! [`SyncBackend::apply_change`] to reach a convergent state. Conflict-resolution
//! semantics (last-write-wins by `updated_at`) are also exercised here.

use chrono::{Duration, Utc};
use keeplin_core::{
    models::{Change, Note},
    storage::{db::DbBackend, fs::FsBackend, NoteRepository, SyncBackend},
};
use tempfile::tempdir;

/// Create a standalone offline `DbBackend` backed by a temporary file. The temp dir is
/// leaked so it outlives the open database connection for the duration of the test.
async fn device() -> DbBackend {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(db_path, "", "").await.unwrap()
}

/// A change created on device A must propagate to device B via the change journal.
#[tokio::test]
async fn create_propagates_between_devices() {
    let a = device().await;
    let b = device().await;

    let note = Note::new("Shared", "from A");
    let id = note.id;
    a.create_note(note).await.unwrap();

    // Collect every change A has recorded and replay it on B.
    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let changes = a.get_changes_since(epoch).await.unwrap();
    assert!(!changes.is_empty(), "device A must have recorded a change");
    for c in changes {
        b.apply_change(c).await.unwrap();
    }

    let read = b.read_note(id).await.unwrap();
    assert_eq!(read.title, "Shared");
    assert_eq!(read.body, "from A");
}

/// A stale remote update (older `updated_at`) must not clobber a newer local edit.
///
/// This pins last-write-wins-by-timestamp conflict resolution: applying a change
/// whose `updated_at` is older than the local record is a no-op.
#[tokio::test]
async fn stale_remote_update_does_not_clobber_newer_local() {
    let local = device().await;

    // The note as it currently exists locally — freshly edited "now".
    let mut note = Note::new("Title", "current local body");
    let id = note.id;
    note.updated_at = Utc::now();
    local.create_note(note.clone()).await.unwrap();

    // A remote device sends an *older* version of the same note (edited a minute ago).
    let mut stale = note.clone();
    stale.body = "stale remote body".to_string();
    stale.updated_at = Utc::now() - Duration::minutes(1);

    local
        .apply_change(Change::NoteUpdate { note: stale })
        .await
        .unwrap();

    // The newer local body must survive; the stale remote write must be ignored.
    let read = local.read_note(id).await.unwrap();
    assert_eq!(
        read.body, "current local body",
        "a stale remote update must not overwrite a newer local edit"
    );
}

/// The same last-write-wins guarantee must hold for `FsBackend`: a newer remote edit
/// applies, but an older one is ignored.
#[tokio::test]
async fn fs_apply_change_respects_last_write_wins() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let mut note = Note::new("Title", "current local body");
    let id = note.id;
    note.updated_at = Utc::now();
    backend.create_note(note.clone()).await.unwrap();

    // Older remote edit → ignored.
    let mut stale = note.clone();
    stale.body = "stale remote body".to_string();
    stale.updated_at = Utc::now() - Duration::minutes(1);
    backend
        .apply_change(Change::NoteUpdate { note: stale })
        .await
        .unwrap();
    assert_eq!(
        backend.read_note(id).await.unwrap().body,
        "current local body"
    );

    // Newer remote edit → applied.
    let mut fresh = note.clone();
    fresh.body = "newer remote body".to_string();
    fresh.updated_at = Utc::now() + Duration::minutes(1);
    backend
        .apply_change(Change::NoteUpdate { note: fresh })
        .await
        .unwrap();
    assert_eq!(
        backend.read_note(id).await.unwrap().body,
        "newer remote body"
    );
}
