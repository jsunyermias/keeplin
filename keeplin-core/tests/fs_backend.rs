//! Integration tests for [`FsBackend`] — the filesystem-backed storage implementation.
//!
//! Every test in this module creates a fresh temporary directory with [`tempfile::tempdir`],
//! constructs a new [`FsBackend`] rooted there, and exercises the full
//! [`StorageBackend`] API against real files on disk. The tests verify both the
//! happy path (create → read → update → delete) and error paths (operations on
//! non-existent entities must return [`StorageError::NotFound`]).

use keeplin_core::{
    error::StorageError,
    models::{Note, NoteTag, Notebook, Resource, Tag},
    storage::{
        fs::FsBackend, NoteRepository, NotebookRepository, ResourceRepository, SyncBackend,
        TagRepository,
    },
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

    let (notes, _) = backend.list_notes(0, None).await.unwrap();
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

    let (notes, _) = backend.list_notes(0, None).await.unwrap();
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
    // The sync timestamp is serialised as an RFC-3339 string and then deserialised
    // back. Sub-second precision may be lost during that round-trip, so the
    // comparison is done at second-level granularity using Unix timestamps.
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

    // Simulate a log file that a different device has written and Syncthing has
    // replicated into the `logs/` directory. The file name must differ from this
    // device's own log file name so that `get_changes_since` does not skip it.
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

    let (notebooks, _) = backend.list_notebooks(0, None).await.unwrap();
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

    let (tags, _) = backend.list_note_tags(note_id, 0, None).await.unwrap();
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
    let (tags, _) = backend.list_note_tags(note_id, 0, None).await.unwrap();
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
        let res = Resource::new(
            format!("file{i}"),
            "application/octet-stream",
            format!("f{i}.bin"),
            1,
        );
        backend.create_resource(res, data).await.unwrap();
    }

    let (list, _) = backend.list_resources(0, None).await.unwrap();
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

// ── Pagination test ───────────────────────────────────────────────────────────

#[tokio::test]
async fn list_notes_paginates_without_duplicates_or_gaps() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    // Insert more notes than a single page holds so the cursor must be walked.
    let total = 23usize;
    for i in 0..total {
        backend
            .create_note(Note::new(format!("Note {i:02}"), ""))
            .await
            .unwrap();
    }

    let page_size = 7u32;
    let mut seen = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let (page, next) = backend.list_notes(page_size, token).await.unwrap();
        assert!(page.len() <= page_size as usize);
        seen.extend(page.iter().map(|n| n.id));
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }

    assert_eq!(
        seen.len(),
        total,
        "every note must be returned exactly once"
    );
    let unique: std::collections::HashSet<_> = seen.iter().copied().collect();
    assert_eq!(unique.len(), total, "no note may appear on two pages");

    let (all, _) = backend.list_notes(total as u32 + 5, None).await.unwrap();
    let all_ids: Vec<_> = all.iter().map(|n| n.id).collect();
    assert_eq!(seen, all_ids, "paged order must match single-shot order");
}

// ── Version-vector note model ─────────────────────────────────────────────────

/// Simulate Syncthing replicating a note from one root to another by copying only its
/// per-device log files (the single-writer source of truth), not the local projections.
async fn replicate_note(from_root: &std::path::Path, to_root: &std::path::Path, id: uuid::Uuid) {
    let from = from_root.join("notes").join(id.to_string());
    let to = to_root.join("notes").join(id.to_string());
    tokio::fs::create_dir_all(&to).await.unwrap();
    let mut rd = tokio::fs::read_dir(&from).await.unwrap();
    while let Some(e) = rd.next_entry().await.unwrap() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("log.") && name.ends_with(".msgpack") {
            tokio::fs::copy(e.path(), to.join(&name)).await.unwrap();
        }
    }
}

#[tokio::test]
async fn fs_note_uses_three_file_layout() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let note = Note::new("Title", "# Markdown body");
    let id = note.id;
    backend.create_note(note).await.unwrap();

    let ndir = dir.path().join("notes").join(id.to_string());
    assert!(ndir.join("note.md").exists(), "note.md must exist");
    assert!(
        ndir.join("meta.msgpack").exists(),
        "meta.msgpack must exist"
    );

    let mut found_log = false;
    for e in std::fs::read_dir(&ndir).unwrap() {
        let n = e.unwrap().file_name().to_string_lossy().into_owned();
        if n.starts_with("log.") && n.ends_with(".msgpack") {
            found_log = true;
        }
    }
    assert!(found_log, "a per-device log file must exist");

    // The markdown body is stored verbatim (unencrypted backend).
    let body = std::fs::read_to_string(ndir.join("note.md")).unwrap();
    assert_eq!(body, "# Markdown body");
}

#[tokio::test]
async fn fs_two_device_causal_sync() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    let a = FsBackend::new(dir_a.path()).await.unwrap();
    let b = FsBackend::new(dir_b.path()).await.unwrap();

    let note = Note::new("Title", "from A");
    let id = note.id;
    a.create_note(note).await.unwrap();

    // A → B: B sees A's note after the log replicates.
    replicate_note(dir_a.path(), dir_b.path(), id).await;
    assert_eq!(b.read_note(id).await.unwrap().body, "from A");

    // B edits causally (it has seen A's version).
    let mut edited = b.read_note(id).await.unwrap();
    edited.body = "edited by B".to_string();
    b.update_note(edited).await.unwrap();

    // B → A: the causal edit wins with no conflict.
    replicate_note(dir_b.path(), dir_a.path(), id).await;
    assert_eq!(a.read_note(id).await.unwrap().body, "edited by B");
}

#[tokio::test]
async fn fs_two_device_concurrent_edits_converge() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    let a = FsBackend::new(dir_a.path()).await.unwrap();
    let b = FsBackend::new(dir_b.path()).await.unwrap();

    let note = Note::new("T", "base");
    let id = note.id;
    a.create_note(note).await.unwrap();
    replicate_note(dir_a.path(), dir_b.path(), id).await;
    b.read_note(id).await.unwrap();

    // Concurrent edits with no exchange between them.
    let mut ea = a.read_note(id).await.unwrap();
    ea.body = "A wins?".to_string();
    a.update_note(ea).await.unwrap();

    let mut eb = b.read_note(id).await.unwrap();
    eb.body = "B wins?".to_string();
    b.update_note(eb).await.unwrap();

    // Cross-replicate both logs; both devices must converge to the SAME winner
    // (deterministic last-write-wins by timestamp, then device id).
    replicate_note(dir_b.path(), dir_a.path(), id).await;
    replicate_note(dir_a.path(), dir_b.path(), id).await;
    let winner_a = a.read_note(id).await.unwrap().body;
    let winner_b = b.read_note(id).await.unwrap().body;
    assert!(winner_a == "A wins?" || winner_a == "B wins?");
    assert_eq!(
        winner_a, winner_b,
        "both devices must converge to one winner"
    );
}
