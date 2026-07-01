//! Integration tests for [`keeplin_core::migrate::migrate`] — a one-shot copy of all live
//! state between any two backends, in both directions and with encryption on both sides.

use keeplin_core::{
    encryption::EncryptedBackend,
    links::{Bookmark, LinkSource, NoteLink},
    migrate::migrate,
    models::{Note, NoteTag, Notebook, Resource, Tag},
    storage::{db::DbBackend, fs::FsBackend, StorageBackend},
};
use tempfile::tempdir;

/// Fixed dataset written into a source backend, then asserted on the destination after a
/// migration. Returns the ids needed to check the round-trip.
struct Seeded {
    notebook_id: uuid::Uuid,
    tag_id: uuid::Uuid,
    note_a: uuid::Uuid,
    note_b: uuid::Uuid,
    resource_id: uuid::Uuid,
    data: Vec<u8>,
}

/// Populate `src` with a notebook, a tag, two notes (one carrying an alias, a bookmark, and a
/// resolved link to the other), a note↔tag association, and a resource with binary data.
///
/// Notes are written with their navigation fields (`alias`/`bookmarks`/`links`) pre-populated
/// so the test exercises verbatim field fidelity without needing a `LinkingBackend` in the
/// stack — which is exactly how `migrate` copies them.
async fn seed(src: &dyn StorageBackend) -> Seeded {
    let notebook = Notebook::new("Work");
    let notebook_id = notebook.id;
    src.create_notebook(notebook).await.unwrap();

    let tag = Tag::new("urgent");
    let tag_id = tag.id;
    src.create_tag(tag).await.unwrap();

    // Note B is the link target; create it first so A can point at it.
    let note_b = Note::new("Target", "the destination note");
    let note_b_id = note_b.id;
    src.create_note(note_b).await.unwrap();

    let mut note_a = Note::new(
        "Source",
        "intro [Anchor](### \"Alias\") and a [link](#target)",
    );
    let note_a_id = note_a.id;
    note_a.notebook_id = Some(notebook_id);
    note_a.alias = Some("alpha".to_string());
    note_a.bookmarks = vec![Bookmark {
        number: 1,
        text: "Anchor".to_string(),
        alias: "Alias".to_string(),
    }];
    note_a.links = vec![NoteLink {
        source: LinkSource::Content,
        raw: "#target".to_string(),
        target_note_id: Some(note_b_id),
    }];
    src.create_note(note_a.clone()).await.unwrap();

    src.add_note_tag(NoteTag {
        note_id: note_a_id,
        tag_id,
    })
    .await
    .unwrap();

    let data = b"\x00\x01\x02binary-payload\xff".to_vec();
    let resource = Resource::new("img", "image/png", "img.png", data.len() as u64);
    let resource_id = resource.id;
    src.create_resource(resource, data.clone()).await.unwrap();

    Seeded {
        notebook_id,
        tag_id,
        note_a: note_a_id,
        note_b: note_b_id,
        resource_id,
        data,
    }
}

/// Assert that `dst` faithfully reproduces everything [`seed`] wrote into the source.
async fn assert_migrated(dst: &dyn StorageBackend, s: &Seeded) {
    let nb = dst.read_notebook(s.notebook_id).await.unwrap();
    assert_eq!(nb.title, "Work");

    let tag = dst.read_tag(s.tag_id).await.unwrap();
    assert_eq!(tag.title, "urgent");

    let a = dst.read_note(s.note_a).await.unwrap();
    assert_eq!(a.title, "Source");
    assert_eq!(a.notebook_id, Some(s.notebook_id));
    assert_eq!(a.alias.as_deref(), Some("alpha"));
    assert_eq!(a.bookmarks.len(), 1);
    assert_eq!(a.bookmarks[0].text, "Anchor");
    assert_eq!(a.bookmarks[0].alias, "Alias");
    assert_eq!(a.links.len(), 1);
    assert_eq!(a.links[0].raw, "#target");
    assert_eq!(a.links[0].target_note_id, Some(s.note_b));

    // The note↔tag association survives.
    let (tags, _) = dst.list_note_tags(s.note_a, 0, None).await.unwrap();
    assert!(tags.iter().any(|t| t.id == s.tag_id));

    // The resource metadata and its exact bytes survive.
    let (meta, bytes) = dst.read_resource(s.resource_id).await.unwrap();
    assert_eq!(meta.file_name, "img.png");
    assert_eq!(bytes, s.data);

    // Backlinks resolve on the destination (built from the copied `links`).
    let (back, _) = dst.note_backlinks(s.note_b, 0, None).await.unwrap();
    assert!(back.iter().any(|n| n.id == s.note_a));
}

/// Build a fresh offline `DbBackend` under a temp dir (no server URL → no WebSocket).
async fn db(dir: &std::path::Path) -> DbBackend {
    DbBackend::new(dir.join("keeplin.db"), "", "")
        .await
        .unwrap()
}

#[tokio::test]
async fn fs_to_db_round_trip() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let src = FsBackend::new(src_dir.path()).await.unwrap();
    let dst = db(dst_dir.path()).await;

    let seeded = seed(&src).await;
    let report = migrate(&src, &dst).await.unwrap();

    assert_eq!(report.notebooks, 1);
    assert_eq!(report.tags, 1);
    assert_eq!(report.notes, 2);
    assert_eq!(report.note_tags, 1);
    assert_eq!(report.resources, 1);
    assert_migrated(&dst, &seeded).await;
}

#[tokio::test]
async fn db_to_fs_round_trip() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let src = db(src_dir.path()).await;
    let dst = FsBackend::new(dst_dir.path()).await.unwrap();

    let seeded = seed(&src).await;
    let report = migrate(&src, &dst).await.unwrap();

    assert_eq!(report.notes, 2);
    assert_eq!(report.resources, 1);
    assert_migrated(&dst, &seeded).await;
}

#[tokio::test]
async fn encrypted_fs_to_encrypted_db() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    // Different passwords on each side: migration reads plaintext from the source and
    // re-encrypts under the destination's own key.
    let src = EncryptedBackend::new(
        FsBackend::new(src_dir.path()).await.unwrap(),
        "source-pass",
        b"source-salt",
    )
    .await
    .unwrap();
    let dst = EncryptedBackend::new(db(dst_dir.path()).await, "dest-pass", b"dest-salt")
        .await
        .unwrap();

    let seeded = seed(&src).await;
    migrate(&src, &dst).await.unwrap();

    // Reads through the destination's encryption decrypt correctly.
    assert_migrated(&dst, &seeded).await;
}
