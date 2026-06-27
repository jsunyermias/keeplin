use keeplin_core::{
    error::StorageError,
    models::{Note, Notebook, NoteTag, Resource, Tag},
    storage::{fs::FsBackend, StorageBackend},
};
use tempfile::tempdir;

#[tokio::test]
async fn create_and_read_note() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let note = Note::new("Test title", "Test body");
    let id = note.id;

    let created = backend.create_note(note).await.unwrap();
    assert_eq!(created.id, id);

    let read = backend.read_note(id).await.unwrap();
    assert_eq!(read.title, "Test title");
    assert_eq!(read.body, "Test body");
}

#[tokio::test]
async fn update_note() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let mut note = Note::new("Original", "Body");
    let id = note.id;
    backend.create_note(note.clone()).await.unwrap();

    note.title = "Updated".to_string();
    let updated = backend.update_note(note).await.unwrap();
    assert_eq!(updated.title, "Updated");

    let read = backend.read_note(id).await.unwrap();
    assert_eq!(read.title, "Updated");
}

#[tokio::test]
async fn delete_note_soft_deletes() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let note = Note::new("To delete", "");
    let id = note.id;
    backend.create_note(note).await.unwrap();
    backend.delete_note(id).await.unwrap();

    let notes = backend.list_notes().await.unwrap();
    assert!(!notes.iter().any(|n| n.id == id));
}

#[tokio::test]
async fn list_notes_excludes_deleted() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let a = Note::new("A", "");
    let b = Note::new("B", "");
    let a_id = a.id;
    backend.create_note(a).await.unwrap();
    backend.create_note(b).await.unwrap();
    backend.delete_note(a_id).await.unwrap();

    let notes = backend.list_notes().await.unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].title, "B");
}

#[tokio::test]
async fn read_nonexistent_note_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let id = uuid::Uuid::new_v4();
    let err = backend.read_note(id).await.unwrap_err();
    assert!(
        matches!(err, StorageError::NotFound(_)),
        "Expected NotFound, got {err:?}"
    );
}

#[tokio::test]
async fn device_id_is_stable_across_instances() {
    let dir = tempdir().unwrap();
    let b1 = FsBackend::new(dir.path()).await.unwrap();
    let id1 = b1.get_device_id().await.unwrap();

    let b2 = FsBackend::new(dir.path()).await.unwrap();
    let id2 = b2.get_device_id().await.unwrap();

    assert_eq!(id1, id2);
}

#[tokio::test]
async fn sync_state_persists() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let ts = chrono::Utc::now();
    backend.update_sync_time(ts).await.unwrap();

    let read = backend.get_last_sync_time().await.unwrap();
    // Compare with second-level precision (RFC-3339 round-trip)
    assert_eq!(
        read.timestamp(),
        ts.timestamp(),
        "Sync timestamp should persist"
    );
}

#[tokio::test]
async fn get_changes_since_scans_other_device_logs() {
    use keeplin_core::models::Change;

    let dir = tempdir().unwrap();
    let our = FsBackend::new(dir.path()).await.unwrap();

    // Simulate a log file written by a different device
    let other_note = Note::new("Remote note", "Remote body");
    let entry = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "note_id": other_note.id.to_string(),
        "operation": "create",
        "data": other_note
    });
    let log_path = dir.path().join("logs").join("other-device.log");
    tokio::fs::write(&log_path, entry.to_string() + "\n")
        .await
        .unwrap();

    let since = chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap();
    let changes = our.get_changes_since(since).await.unwrap();
    assert_eq!(changes.len(), 1);
    assert!(matches!(changes[0], Change::NoteCreate { .. }));
}

// ── Error-path tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn update_nonexistent_note_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let note = Note::new("Ghost", "");
    let err = backend.update_note(note).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

#[tokio::test]
async fn delete_nonexistent_note_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let id = uuid::Uuid::new_v4();
    let err = backend.delete_note(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

#[tokio::test]
async fn update_nonexistent_notebook_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let nb = Notebook::new("Ghost");
    let err = backend.update_notebook(nb).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

#[tokio::test]
async fn delete_nonexistent_notebook_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let id = uuid::Uuid::new_v4();
    let err = backend.delete_notebook(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

#[tokio::test]
async fn update_nonexistent_tag_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let tag = Tag::new("ghost");
    let err = backend.update_tag(tag).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

#[tokio::test]
async fn delete_nonexistent_tag_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let id = uuid::Uuid::new_v4();
    let err = backend.delete_tag(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

// ── Notebook tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_and_read_notebook() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let nb = Notebook::new("Work");
    let id = nb.id;
    backend.create_notebook(nb).await.unwrap();

    let read = backend.read_notebook(id).await.unwrap();
    assert_eq!(read.title, "Work");
    assert!(read.deleted_at.is_none());
}

#[tokio::test]
async fn delete_notebook_soft_deletes() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let nb = Notebook::new("Temp");
    let id = nb.id;
    backend.create_notebook(nb).await.unwrap();
    backend.delete_notebook(id).await.unwrap();

    let notebooks = backend.list_notebooks().await.unwrap();
    assert!(!notebooks.iter().any(|n| n.id == id));

    let raw = backend.read_notebook(id).await.unwrap();
    assert!(raw.deleted_at.is_some());
}

// ── Tag tests ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_and_read_tag() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let tag = Tag::new("rust");
    let id = tag.id;
    backend.create_tag(tag).await.unwrap();

    let read = backend.read_tag(id).await.unwrap();
    assert_eq!(read.title, "rust");
}

#[tokio::test]
async fn add_and_list_note_tags() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let note = Note::new("Tagged", "body");
    let tag = Tag::new("important");
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
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

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
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let data = b"hello world".to_vec();
    let res = Resource::new("attachment", "text/plain", "hello.txt", data.len() as u64);
    let id = res.id;
    backend.create_resource(res, data.clone()).await.unwrap();

    let (meta, bytes) = backend.read_resource(id).await.unwrap();
    assert_eq!(meta.title, "attachment");
    assert_eq!(bytes, data);
}

#[tokio::test]
async fn list_resources_excludes_data() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    for i in 0..3u8 {
        let data = vec![i];
        let res = Resource::new(format!("file{i}"), "application/octet-stream", format!("f{i}.bin"), 1);
        backend.create_resource(res, data).await.unwrap();
    }

    let list = backend.list_resources().await.unwrap();
    assert_eq!(list.len(), 3);
}

#[tokio::test]
async fn delete_resource() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let res = Resource::new("doc", "text/plain", "doc.txt", 0);
    let id = res.id;
    backend.create_resource(res, vec![]).await.unwrap();
    backend.delete_resource(id).await.unwrap();

    let err = backend.read_resource(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}
