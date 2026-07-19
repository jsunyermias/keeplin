// md:Overview

use keeplin_core::{
    encryption::EncryptedBackend,
    models::{Note, NoteTag, Notebook, Resource, Tag},
    storage::{
        fs::FsBackend, NoteRepository, NotebookRepository, ResourceRepository, TagRepository,
    },
};
use tempfile::tempdir;

// md:TEST_SALT
const TEST_SALT: &[u8] = b"keeplin-test-salt";

// md:fn enc_backend
async fn enc_backend(dir: &std::path::Path) -> EncryptedBackend<FsBackend> {
    let fs = FsBackend::new(dir).await.unwrap();
    EncryptedBackend::new(fs, "test-password", TEST_SALT)
        .await
        .unwrap()
}

// md:fn note_round_trips
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

// md:fn storage_contains_ciphertext_not_plaintext
#[tokio::test]
async fn storage_contains_ciphertext_not_plaintext() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let note = Note::new("plaintext-title", "plaintext-body");
    let id = note.id;
    backend.create_note(note).await.unwrap();

    let ndir = dir.path().join("notes").join(id.to_string());
    let md = std::fs::read_to_string(ndir.join("note.md")).unwrap();
    assert!(
        !md.contains("plaintext-body"),
        "note.md should not contain plaintext body"
    );
    let meta = std::fs::read(ndir.join("meta.msgpack")).unwrap();
    let title_needle = b"plaintext-title";
    assert!(
        !meta.windows(title_needle.len()).any(|w| w == title_needle),
        "meta.msgpack should not contain plaintext title"
    );
}

// md:fn wrong_password_fails_to_decrypt
#[tokio::test]
async fn wrong_password_fails_to_decrypt() {
    let dir = tempdir().unwrap();

    let fs1 = FsBackend::new(dir.path()).await.unwrap();
    let enc1 = EncryptedBackend::new(fs1, "correct", TEST_SALT)
        .await
        .unwrap();
    let note = Note::new("Hello", "World");
    let id = note.id;
    enc1.create_note(note).await.unwrap();

    let fs2 = FsBackend::new(dir.path()).await.unwrap();
    let enc2 = EncryptedBackend::new(fs2, "wrong", TEST_SALT)
        .await
        .unwrap();
    assert!(
        enc2.read_note(id).await.is_err(),
        "wrong password must fail to decrypt"
    );
}

// md:fn update_note_encrypts_new_content
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

// md:fn list_notes_decrypts_all
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

    let (notes, _) = backend.list_notes(0, None).await.unwrap();
    assert_eq!(notes.len(), 3);
    for note in &notes {
        assert!(
            note.title.starts_with("Note "),
            "list_notes must return decrypted titles, got: {}",
            note.title
        );
    }
}

// md:fn notebook_round_trips
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

// md:fn tag_round_trips
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

// md:fn note_tag_relation_preserved
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

    let (tags, _) = backend.list_note_tags(note_id, 0, None).await.unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].title, "T");
}

// md:fn resource_round_trips
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

// md:fn resource_data_stored_encrypted
#[tokio::test]
async fn resource_data_stored_encrypted() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let data = b"supersecret".to_vec();
    let res = Resource::new("r", "text/plain", "r.txt", data.len() as u64);
    let id = res.id;
    backend.create_resource(res, data).await.unwrap();

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

// md:fn list_notes_paginates_and_decrypts_each_page
#[tokio::test]
async fn list_notes_paginates_and_decrypts_each_page() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let total = 20usize;
    for i in 0..total {
        backend
            .create_note(Note::new(format!("Secret {i:02}"), "body"))
            .await
            .unwrap();
    }

    let page_size = 6u32;
    let mut seen = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let (page, next) = backend.list_notes(page_size, token).await.unwrap();
        assert!(page.len() <= page_size as usize);
        for note in &page {
            assert!(
                note.title.starts_with("Secret "),
                "title must be decrypted, got: {}",
                note.title
            );
        }
        seen.extend(page.iter().map(|n| n.id));
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }

    assert_eq!(seen.len(), total);
    let unique: std::collections::HashSet<_> = seen.iter().copied().collect();
    assert_eq!(unique.len(), total, "no note may appear on two pages");
}
