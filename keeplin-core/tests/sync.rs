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

/// A tombstone older than the local edit must not delete a newer note (DbBackend).
#[tokio::test]
async fn db_stale_delete_does_not_override_newer_edit() {
    let local = device().await;
    let mut note = Note::new("Title", "current body");
    let id = note.id;
    note.updated_at = Utc::now();
    local.create_note(note).await.unwrap();

    // A delete that happened a minute *before* the local edit.
    local
        .apply_change(Change::NoteDelete {
            id,
            deleted_at: Utc::now() - Duration::minutes(1),
        })
        .await
        .unwrap();

    let read = local.read_note(id).await.unwrap();
    assert!(
        read.deleted_at.is_none(),
        "a stale delete must not tombstone a newer note"
    );
}

/// A stale edit must not resurrect a newer tombstone (DbBackend).
#[tokio::test]
async fn db_stale_update_does_not_resurrect_tombstone() {
    let local = device().await;
    let mut note = Note::new("Title", "original body");
    let id = note.id;
    note.updated_at = Utc::now() - Duration::minutes(5);
    local.create_note(note.clone()).await.unwrap();

    // Delete locally now — the tombstone's updated_at becomes "now".
    local.delete_note(id).await.unwrap();

    // A stale update from before the delete arrives from a peer.
    let mut stale = note.clone();
    stale.body = "resurrected?".to_string();
    local
        .apply_change(Change::NoteUpdate { note: stale })
        .await
        .unwrap();

    let read = local.read_note(id).await.unwrap();
    assert!(
        read.deleted_at.is_some(),
        "a stale update must not resurrect a tombstoned note"
    );
}

/// Tombstone semantics must hold on `FsBackend` too: a stale delete is ignored and a
/// stale update cannot resurrect a newer delete.
#[tokio::test]
async fn fs_tombstones_resolve_by_timestamp() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    // (a) Stale delete must not tombstone a newer note.
    let mut a = Note::new("A", "body");
    let a_id = a.id;
    a.updated_at = Utc::now();
    backend.create_note(a).await.unwrap();
    backend
        .apply_change(Change::NoteDelete {
            id: a_id,
            deleted_at: Utc::now() - Duration::minutes(1),
        })
        .await
        .unwrap();
    assert!(backend.read_note(a_id).await.unwrap().deleted_at.is_none());

    // (b) Stale update must not resurrect a newer local delete.
    let mut b = Note::new("B", "body");
    let b_id = b.id;
    b.updated_at = Utc::now() - Duration::minutes(5);
    backend.create_note(b.clone()).await.unwrap();
    backend.delete_note(b_id).await.unwrap();
    let mut stale = b.clone();
    stale.body = "resurrected?".to_string();
    backend
        .apply_change(Change::NoteUpdate { note: stale })
        .await
        .unwrap();
    assert!(backend.read_note(b_id).await.unwrap().deleted_at.is_some());
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
