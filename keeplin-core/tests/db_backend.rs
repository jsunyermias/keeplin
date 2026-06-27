use keeplin_core::{
    models::Note,
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
    assert!(matches!(changes[0], Change::Update { .. }));
}
