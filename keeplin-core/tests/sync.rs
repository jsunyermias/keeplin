// md:Overview

use chrono::{Duration, Utc};
use keeplin_core::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag, SYSTEM_RESOURCE_NOTE_ID},
    storage::{
        db::DbBackend, fs::FsBackend, NoteRepository, NotebookRepository, ResourceRepository,
        SyncBackend, TagRepository,
    },
};
use tempfile::tempdir;

// md:fn device
async fn device() -> DbBackend {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(db_path, "", "").await.unwrap()
}

// md:fn create_propagates_between_devices
#[tokio::test]
async fn create_propagates_between_devices() {
    let a = device().await;
    let b = device().await;

    let note = Note::new("Shared", "from A");
    let id = note.id;
    a.create_note(note).await.unwrap();

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

// md:fn stale_remote_update_does_not_clobber_newer_local
#[tokio::test]
async fn stale_remote_update_does_not_clobber_newer_local() {
    let local = device().await;

    let mut note = Note::new("Title", "current local body");
    let id = note.id;
    note.updated_at = Utc::now();
    local.create_note(note.clone()).await.unwrap();

    let mut stale = note.clone();
    stale.body = "stale remote body".to_string();
    stale.updated_at = Utc::now() - Duration::minutes(1);

    local
        .apply_change(Change::NoteUpdate { note: stale })
        .await
        .unwrap();

    let read = local.read_note(id).await.unwrap();
    assert_eq!(
        read.body, "current local body",
        "a stale remote update must not overwrite a newer local edit"
    );
}

// md:fn db_stale_delete_does_not_override_newer_edit
#[tokio::test]
async fn db_stale_delete_does_not_override_newer_edit() {
    let local = device().await;
    let mut note = Note::new("Title", "current body");
    let id = note.id;
    note.updated_at = Utc::now();
    local.create_note(note).await.unwrap();

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

// md:fn db_stale_update_does_not_resurrect_tombstone
#[tokio::test]
async fn db_stale_update_does_not_resurrect_tombstone() {
    let local = device().await;
    let mut note = Note::new("Title", "original body");
    let id = note.id;
    note.updated_at = Utc::now() - Duration::minutes(5);
    local.create_note(note.clone()).await.unwrap();

    local.delete_note(id).await.unwrap();

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

// md:fn db_concurrent_equal_timestamp_edits_converge
#[tokio::test]
async fn db_concurrent_equal_timestamp_edits_converge() {
    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let a = device().await;
    let b = device().await;

    let base = a.create_note(Note::new("t", "base")).await.unwrap();
    let id = base.id;
    for c in a.get_changes_since(epoch).await.unwrap() {
        b.apply_change(c).await.unwrap();
    }

    let t = Utc::now();
    let mut ea = base.clone();
    ea.body = "from A".to_string();
    ea.updated_at = t;
    a.update_note(ea).await.unwrap();

    let mut eb = b.read_note(id).await.unwrap();
    eb.body = "from B".to_string();
    eb.updated_at = t;
    b.update_note(eb).await.unwrap();

    let a_changes = a.get_changes_since(epoch).await.unwrap();
    let b_changes = b.get_changes_since(epoch).await.unwrap();
    for c in b_changes {
        a.apply_change(c).await.unwrap();
    }
    for c in a_changes {
        b.apply_change(c).await.unwrap();
    }

    let body_a = a.read_note(id).await.unwrap().body;
    let body_b = b.read_note(id).await.unwrap().body;
    assert_eq!(
        body_a, body_b,
        "concurrent equal-timestamp edits must converge"
    );
    assert!(body_a == "from A" || body_a == "from B");
}

// md:fn db_concurrent_notebook_edits_converge
#[tokio::test]
async fn db_concurrent_notebook_edits_converge() {
    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let a = device().await;
    let b = device().await;

    let base = a.create_notebook(Notebook::new("base")).await.unwrap();
    let id = base.id;
    for c in a.get_changes_since(epoch).await.unwrap() {
        b.apply_change(c).await.unwrap();
    }

    let t = Utc::now();
    let mut ea = base.clone();
    ea.title = "from A".to_string();
    ea.updated_at = t;
    a.update_notebook(ea).await.unwrap();

    let mut eb = b.read_notebook(id).await.unwrap();
    eb.title = "from B".to_string();
    eb.updated_at = t;
    b.update_notebook(eb).await.unwrap();

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

// md:fn fs_tombstones_resolve_by_timestamp
#[tokio::test]
async fn fs_tombstones_resolve_by_timestamp() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

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

// md:fn db_concurrent_note_tag_add_remove_converges
#[tokio::test]
async fn db_concurrent_note_tag_add_remove_converges() {
    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let a = device().await;
    let b = device().await;

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

    a.remove_note_tag(note.id, tag.id).await.unwrap();
    b.add_note_tag(NoteTag {
        note_id: note.id,
        tag_id: tag.id,
    })
    .await
    .unwrap();

    let a_changes = a.get_changes_since(epoch).await.unwrap();
    let b_changes = b.get_changes_since(epoch).await.unwrap();
    for c in b_changes {
        a.apply_change(c).await.unwrap();
    }
    for c in a_changes {
        b.apply_change(c).await.unwrap();
    }

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

// md:fn db_resource_delete_propagates_and_converges
#[tokio::test]
async fn db_resource_delete_propagates_and_converges() {
    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let a = device().await;
    let b = device().await;

    let res = Resource::new(SYSTEM_RESOURCE_NOTE_ID, "f", "text/plain", "f.txt", 3);
    let id = res.id;
    a.create_resource(res, b"abc".to_vec()).await.unwrap();
    for c in a.get_changes_since(epoch).await.unwrap() {
        b.apply_change(c).await.unwrap();
    }
    assert!(b.read_resource(id).await.is_ok(), "create must propagate");

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
