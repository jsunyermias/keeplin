//! Integration tests for change propagation between two storage backends.
//!
//! These tests model two independent devices, each backed by its own `DbBackend`
//! database file, and verify that changes recorded on one device can be collected
//! with [`SyncBackend::get_changes_since`] and replayed on the other device with
//! [`SyncBackend::apply_change`] to reach a convergent state. Conflict-resolution
//! semantics (version-vector `resolve`, including the concurrent equal-timestamp case that
//! bare last-write-wins would diverge on) are also exercised here.

use chrono::{Duration, Utc};
use keeplin_core::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
    storage::{
        db::DbBackend, fs::FsBackend, NoteRepository, NotebookRepository, ResourceRepository,
        SyncBackend, TagRepository,
    },
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
            vv: Default::default(),
            last_writer: String::new(),
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

/// Two devices editing the same note concurrently with the **identical** `updated_at`
/// converge on one deterministic winner. Under the old bare-`updated_at` last-write-wins
/// (strict `>`), each device would keep its own edit → permanent divergence. The version
/// vector's `(timestamp, device_id)` tiebreak makes both devices pick the same edit.
#[tokio::test]
async fn db_concurrent_equal_timestamp_edits_converge() {
    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let a = device().await;
    let b = device().await;

    // Shared baseline: create on A, replicate to B (B now holds the note at vv {A:1}).
    let base = a.create_note(Note::new("t", "base")).await.unwrap();
    let id = base.id;
    for c in a.get_changes_since(epoch).await.unwrap() {
        b.apply_change(c).await.unwrap();
    }

    // Concurrent edits sharing the SAME updated_at — the case bare LWW diverges on.
    let t = Utc::now();
    let mut ea = base.clone();
    ea.body = "from A".to_string();
    ea.updated_at = t;
    a.update_note(ea).await.unwrap();

    let mut eb = b.read_note(id).await.unwrap();
    eb.body = "from B".to_string();
    eb.updated_at = t;
    b.update_note(eb).await.unwrap();

    // Exchange the concurrent edits (apply_change does not re-journal, so each side's journal
    // holds only its own local edit).
    let a_changes = a.get_changes_since(epoch).await.unwrap();
    let b_changes = b.get_changes_since(epoch).await.unwrap();
    for c in b_changes {
        a.apply_change(c).await.unwrap();
    }
    for c in a_changes {
        b.apply_change(c).await.unwrap();
    }

    // Both devices converge to the SAME body (whichever device id wins the tiebreak).
    let body_a = a.read_note(id).await.unwrap().body;
    let body_b = b.read_note(id).await.unwrap().body;
    assert_eq!(
        body_a, body_b,
        "concurrent equal-timestamp edits must converge"
    );
    assert!(body_a == "from A" || body_a == "from B");
}

/// Notebooks converge under the same version-vector `resolve` as notes: two devices editing the
/// same notebook with the **identical** `updated_at` pick one deterministic winner instead of each
/// keeping its own (which bare `updated_at` last-write-wins would do). This exercises the
/// `DbBackend` notebook `apply_change` arm specifically.
#[tokio::test]
async fn db_concurrent_notebook_edits_converge() {
    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let a = device().await;
    let b = device().await;

    // Shared baseline: create on A, replicate to B.
    let base = a.create_notebook(Notebook::new("base")).await.unwrap();
    let id = base.id;
    for c in a.get_changes_since(epoch).await.unwrap() {
        b.apply_change(c).await.unwrap();
    }

    // Concurrent edits sharing the SAME updated_at.
    let t = Utc::now();
    let mut ea = base.clone();
    ea.title = "from A".to_string();
    ea.updated_at = t;
    a.update_notebook(ea).await.unwrap();

    let mut eb = b.read_notebook(id).await.unwrap();
    eb.title = "from B".to_string();
    eb.updated_at = t;
    b.update_notebook(eb).await.unwrap();

    // Exchange the concurrent edits.
    let a_changes = a.get_changes_since(epoch).await.unwrap();
    let b_changes = b.get_changes_since(epoch).await.unwrap();
    for c in b_changes {
        a.apply_change(c).await.unwrap();
    }
    for c in a_changes {
        b.apply_change(c).await.unwrap();
    }

    let title_a = a.read_notebook(id).await.unwrap().title;
    let title_b = b.read_notebook(id).await.unwrap().title;
    assert_eq!(title_a, title_b, "concurrent notebook edits must converge");
    assert!(title_a == "from A" || title_a == "from B");
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
            vv: Default::default(),
            last_writer: String::new(),
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

// Note: `FsBackend` resolves note conflicts through the per-note version-vector logs,
// not by applying wire `Change::NoteUpdate` records, so the FsBackend equivalent of the
// last-write-wins guarantee is covered by `fs_two_device_concurrent_edits_converge` in
// `tests/fs_backend.rs` rather than by an `apply_change`-driven test here.

/// A concurrent note↔tag add on one device and remove on another converge: both devices end
/// up agreeing on whether the tag is attached. Before Phase 3 associations carried no version
/// (add = INSERT OR IGNORE, remove = DELETE), so the outcome was order-dependent and could
/// differ between devices.
#[tokio::test]
async fn db_concurrent_note_tag_add_remove_converges() {
    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let a = device().await;
    let b = device().await;

    // Baseline: note + tag + attached association on A, replicated to B.
    let note = a.create_note(Note::new("n", "")).await.unwrap();
    let tag = a.create_tag(Tag::new("t")).await.unwrap();
    a.add_note_tag(NoteTag {
        note_id: note.id,
        tag_id: tag.id,
    })
    .await
    .unwrap();
    for c in a.get_changes_since(epoch).await.unwrap() {
        b.apply_change(c).await.unwrap();
    }
    assert_eq!(a.list_note_tags(note.id, 0, None).await.unwrap().0.len(), 1);
    assert_eq!(b.list_note_tags(note.id, 0, None).await.unwrap().0.len(), 1);

    // Concurrent: A detaches, B re-attaches (each from the shared baseline).
    a.remove_note_tag(note.id, tag.id).await.unwrap();
    b.add_note_tag(NoteTag {
        note_id: note.id,
        tag_id: tag.id,
    })
    .await
    .unwrap();

    // Exchange local changes both ways.
    let a_changes = a.get_changes_since(epoch).await.unwrap();
    let b_changes = b.get_changes_since(epoch).await.unwrap();
    for c in b_changes {
        a.apply_change(c).await.unwrap();
    }
    for c in a_changes {
        b.apply_change(c).await.unwrap();
    }

    // Both devices agree on the association's final presence (converged).
    let present_a = !a
        .list_note_tags(note.id, 0, None)
        .await
        .unwrap()
        .0
        .is_empty();
    let present_b = !b
        .list_note_tags(note.id, 0, None)
        .await
        .unwrap()
        .0
        .is_empty();
    assert_eq!(present_a, present_b, "concurrent add/remove must converge");
}

/// A resource delete now soft-deletes (a versioned tombstone) and propagates: after a create
/// syncs to a peer and the origin deletes it, both devices agree the resource is gone (read
/// returns NotFound, list excludes it), instead of the old order-dependent hard delete.
#[tokio::test]
async fn db_resource_delete_propagates_and_converges() {
    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let a = device().await;
    let b = device().await;

    let res = Resource::new("f", "text/plain", "f.txt", 3);
    let id = res.id;
    a.create_resource(res, b"abc".to_vec()).await.unwrap();
    for c in a.get_changes_since(epoch).await.unwrap() {
        b.apply_change(c).await.unwrap();
    }
    assert!(b.read_resource(id).await.is_ok(), "create must propagate");

    // Origin soft-deletes; propagate the tombstone.
    a.delete_resource(id).await.unwrap();
    for c in a.get_changes_since(epoch).await.unwrap() {
        b.apply_change(c).await.unwrap();
    }

    for backend in [&a, &b] {
        assert!(
            matches!(
                backend.read_resource(id).await,
                Err(StorageError::NotFound(_))
            ),
            "a soft-deleted resource reads as NotFound"
        );
        assert!(
            backend.list_resources(0, None).await.unwrap().0.is_empty(),
            "a soft-deleted resource is excluded from listings"
        );
    }
}
