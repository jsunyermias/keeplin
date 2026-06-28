use keeplin_core::{
    encryption::EncryptedBackend,
    models::{Note, NoteTag, Notebook, Resource, Tag},
    storage::{fs::FsBackend, StorageBackend},
};
use tempfile::tempdir;

async fn enc_backend(dir: &std::path::Path) -> EncryptedBackend<FsBackend> {
    let fs = FsBackend::new(dir).await.unwrap();
    EncryptedBackend::new(fs, "test-password").await.unwrap()
}

#[tokio::test]
async fn note_round_trips() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let note = Note::new("Secret title", "Secret body");
    let id = note.id;
    backend.create_note(note).await.unwrap();

    let read = backend.read_note(id).await.unwrap();
    assert_eq!(read.title, "Secret title");
    assert_eq!(read.body, "Secret body");
}

#[tokio::test]
async fn storage_contains_ciphertext_not_plaintext() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let note = Note::new("plaintext-title", "plaintext-body");
    let id = note.id;
    backend.create_note(note).await.unwrap();

    // Raw on-disk file must not expose the plaintext
    let meta_path = dir
        .path()
        .join("notes")
        .join(id.to_string())
        .join("meta.json");
    let raw = std::fs::read_to_string(&meta_path).unwrap();
    assert!(
        !raw.contains("plaintext-title"),
        "meta.json should not contain plaintext title"
    );
    assert!(
        !raw.contains("plaintext-body"),
        "meta.json should not contain plaintext body"
    );
}

#[tokio::test]
async fn wrong_password_fails_to_decrypt() {
    let dir = tempdir().unwrap();

    // Write with correct password
    let fs1 = FsBackend::new(dir.path()).await.unwrap();
    let enc1 = EncryptedBackend::new(fs1, "correct").await.unwrap();
    let note = Note::new("Hello", "World");
    let id = note.id;
    enc1.create_note(note).await.unwrap();

    // Read with wrong password
    let fs2 = FsBackend::new(dir.path()).await.unwrap();
    let enc2 = EncryptedBackend::new(fs2, "wrong").await.unwrap();
    assert!(
        enc2.read_note(id).await.is_err(),
        "wrong password must fail to decrypt"
    );
}

#[tokio::test]
async fn update_note_encrypts_new_content() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let mut note = Note::new("Old title", "Old body");
    let id = note.id;
    backend.create_note(note.clone()).await.unwrap();

    note.title = "New title".to_string();
    note.body = "New body".to_string();
    backend.update_note(note).await.unwrap();

    let read = backend.read_note(id).await.unwrap();
    assert_eq!(read.title, "New title");
    assert_eq!(read.body, "New body");
}

#[tokio::test]
async fn list_notes_decrypts_all() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    for i in 0..3 {
        backend
            .create_note(Note::new(format!("Note {i}"), "body"))
            .await
            .unwrap();
    }

    let notes = backend.list_notes().await.unwrap();
    assert_eq!(notes.len(), 3);
    for note in &notes {
        assert!(
            note.title.starts_with("Note "),
            "list_notes must return decrypted titles, got: {}",
            note.title
        );
    }
}

#[tokio::test]
async fn notebook_round_trips() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let nb = Notebook::new("Private Notebook");
    let id = nb.id;
    backend.create_notebook(nb).await.unwrap();

    let read = backend.read_notebook(id).await.unwrap();
    assert_eq!(read.title, "Private Notebook");
}

#[tokio::test]
async fn tag_round_trips() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let tag = Tag::new("confidential");
    let id = tag.id;
    backend.create_tag(tag).await.unwrap();

    let read = backend.read_tag(id).await.unwrap();
    assert_eq!(read.title, "confidential");
}

#[tokio::test]
async fn note_tag_relation_preserved() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

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

    let tags = backend.list_note_tags(note_id).await.unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].title, "T");
}

#[tokio::test]
async fn resource_round_trips() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let data = b"secret binary content".to_vec();
    let res = Resource::new(
        "attachment",
        "application/octet-stream",
        "file.bin",
        data.len() as u64,
    );
    let id = res.id;
    backend.create_resource(res, data.clone()).await.unwrap();

    let (meta, bytes) = backend.read_resource(id).await.unwrap();
    assert_eq!(meta.title, "attachment");
    assert_eq!(bytes, data);
}

#[tokio::test]
async fn resource_data_stored_encrypted() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let data = b"supersecret".to_vec();
    let res = Resource::new("r", "text/plain", "r.txt", data.len() as u64);
    let id = res.id;
    backend.create_resource(res, data).await.unwrap();

    // Raw bytes on disk should not match plaintext
    let data_path = dir
        .path()
        .join("resources")
        .join(id.to_string())
        .join("data");
    let raw = std::fs::read(&data_path).unwrap();
    assert_ne!(
        raw, b"supersecret",
        "resource data must not be stored in plaintext"
    );
}
