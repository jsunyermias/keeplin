use keeplin_core::{
    models::{Note, Notebook, NoteTag, Resource, Tag},
    storage::{db::DbBackend, StorageBackend},
};
use tempfile::tempdir;

async fn in_memory_backend() -> DbBackend {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    // Leaking the dir so the path stays valid for the duration of the test.
    std::mem::forget(dir);
    DbBackend::new(db_path, "", "").await.unwrap()
}

#[tokio::test]
async fn create_and_read_note() {
    let backend = in_memory_backend().await;

    let note = Note::new("Hello", "World");
    let id = note.id;

    backend.create_note(note).await.unwrap();
    let read = backend.read_note(id).await.unwrap();
    assert_eq!(read.title, "Hello");
    assert_eq!(read.body, "World");
}

#[tokio::test]
async fn update_note() {
    let backend = in_memory_backend().await;

    let mut note = Note::new("Old", "Body");
    let id = note.id;
    backend.create_note(note.clone()).await.unwrap();

    note.title = "New".to_string();
    backend.update_note(note).await.unwrap();

    let read = backend.read_note(id).await.unwrap();
    assert_eq!(read.title, "New");
}

#[tokio::test]
async fn delete_note_soft_deletes() {
    let backend = in_memory_backend().await;

    let note = Note::new("Temporary", "");
    let id = note.id;
    backend.create_note(note).await.unwrap();
    backend.delete_note(id).await.unwrap();

    let notes = backend.list_notes().await.unwrap();
    assert!(!notes.iter().any(|n| n.id == id));
}

#[tokio::test]
async fn list_notes_excludes_deleted() {
    let backend = in_memory_backend().await;

    let a = Note::new("Keep", "");
    let b = Note::new("Delete me", "");
    let b_id = b.id;
    backend.create_note(a).await.unwrap();
    backend.create_note(b).await.unwrap();
    backend.delete_note(b_id).await.unwrap();

    let notes = backend.list_notes().await.unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].title, "Keep");
}

#[tokio::test]
async fn read_nonexistent_returns_not_found() {
    let backend = in_memory_backend().await;
    let id = uuid::Uuid::new_v4();
    let err = backend.read_note(id).await.unwrap_err();
    assert!(matches!(
        err,
        keeplin_core::error::StorageError::NotFound(_)
    ));
}

#[tokio::test]
async fn device_id_is_stable() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("keep.db");

    let b1 = DbBackend::new(&db_path, "", "").await.unwrap();
    let id1 = b1.get_device_id().await.unwrap();

    let b2 = DbBackend::new(&db_path, "", "").await.unwrap();
    let id2 = b2.get_device_id().await.unwrap();

    assert_eq!(id1, id2);
}

#[tokio::test]
async fn sync_state_round_trips() {
    let backend = in_memory_backend().await;

    let ts = chrono::Utc::now();
    backend.update_sync_time(ts).await.unwrap();
    let read = backend.get_last_sync_time().await.unwrap();
    assert_eq!(read.timestamp(), ts.timestamp());
}

#[tokio::test]
async fn get_changes_since_returns_updated_notes() {
    use keeplin_core::models::Change;

    let backend = in_memory_backend().await;
    let before = chrono::Utc::now() - chrono::Duration::seconds(1);

    let note = Note::new("New note", "Body");
    backend.create_note(note).await.unwrap();

    let changes = backend.get_changes_since(before).await.unwrap();
    assert!(!changes.is_empty());
    // A note created after `since` must surface as Create, not Update.
    assert!(matches!(changes[0], Change::Create { .. }));
}

// ── Notebook tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_and_read_notebook() {
    let backend = in_memory_backend().await;
    let nb = Notebook::new("Personal");
    let id = nb.id;
    backend.create_notebook(nb).await.unwrap();

    let read = backend.read_notebook(id).await.unwrap();
    assert_eq!(read.title, "Personal");
    assert!(read.deleted_at.is_none());
}

#[tokio::test]
async fn delete_notebook_soft_deletes() {
    let backend = in_memory_backend().await;
    let nb = Notebook::new("Trash");
    let id = nb.id;
    backend.create_notebook(nb).await.unwrap();
    backend.delete_notebook(id).await.unwrap();

    let list = backend.list_notebooks().await.unwrap();
    assert!(!list.iter().any(|n| n.id == id));

    let raw = backend.read_notebook(id).await.unwrap();
    assert!(raw.deleted_at.is_some());
}

// ── Tag tests ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_and_read_tag() {
    let backend = in_memory_backend().await;
    let tag = Tag::new("async");
    let id = tag.id;
    backend.create_tag(tag).await.unwrap();

    let read = backend.read_tag(id).await.unwrap();
    assert_eq!(read.title, "async");
}

#[tokio::test]
async fn add_and_list_note_tags() {
    let backend = in_memory_backend().await;

    let note = Note::new("Tagged note", "body");
    let tag = Tag::new("urgent");
    let note_id = note.id;
    let tag_id = tag.id;
    backend.create_note(note).await.unwrap();
    backend.create_tag(tag).await.unwrap();
    backend
        .add_note_tag(NoteTag { note_id, tag_id })
        .await
        .unwrap();

    let tags = backend.list_note_tags(note_id).await.unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].id, tag_id);
}

#[tokio::test]
async fn remove_note_tag() {
    let backend = in_memory_backend().await;

    let note = Note::new("N", "");
    let tag = Tag::new("T");
    let note_id = note.id;
    let tag_id = tag.id;
    backend.create_note(note).await.unwrap();
    backend.create_tag(tag).await.unwrap();
    backend
        .add_note_tag(NoteTag { note_id, tag_id })
        .await
        .unwrap();

    backend.remove_note_tag(note_id, tag_id).await.unwrap();
    let tags = backend.list_note_tags(note_id).await.unwrap();
    assert!(tags.is_empty());
}

// ── Resource tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_and_read_resource() {
    let backend = in_memory_backend().await;

    let data = b"binary content".to_vec();
    let res = Resource::new("img", "image/png", "img.png", data.len() as u64);
    let id = res.id;
    backend.create_resource(res, data.clone()).await.unwrap();

    let (meta, bytes) = backend.read_resource(id).await.unwrap();
    assert_eq!(meta.title, "img");
    assert_eq!(bytes, data);
}

#[tokio::test]
async fn list_resources_excludes_data() {
    let backend = in_memory_backend().await;

    for i in 0..3u8 {
        let data = vec![i];
        let res = Resource::new(
            format!("file{i}"),
            "application/octet-stream",
            format!("f{i}.bin"),
            1,
        );
        backend.create_resource(res, data).await.unwrap();
    }

    let list = backend.list_resources().await.unwrap();
    assert_eq!(list.len(), 3);
}

#[tokio::test]
async fn delete_resource() {
    let backend = in_memory_backend().await;

    let res = Resource::new("doc", "text/plain", "doc.txt", 0);
    let id = res.id;
    backend.create_resource(res, vec![]).await.unwrap();
    backend.delete_resource(id).await.unwrap();

    let err = backend.read_resource(id).await.unwrap_err();
    assert!(matches!(
        err,
        keeplin_core::error::StorageError::NotFound(_)
    ));
}
