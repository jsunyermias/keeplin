// md:Overview

use chrono::Utc;
use keeplin_core::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
    storage::{
        fs::FsBackend, NoteRepository, NotebookRepository, ResourceRepository, SyncBackend,
        TagRepository,
    },
};
use tempfile::tempdir;

// md:fn create_and_read_note
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

// md:fn update_note
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

// md:fn delete_note_soft_deletes
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

// md:fn list_notes_excludes_deleted
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

// md:fn read_nonexistent_note_returns_not_found
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

// md:fn device_id_is_stable_across_instances
#[tokio::test]
async fn device_id_is_stable_across_instances() {
    let dir = tempdir().unwrap();
    let b1 = FsBackend::new(dir.path()).await.unwrap();
    let id1 = b1.get_device_id().await.unwrap();

    let b2 = FsBackend::new(dir.path()).await.unwrap();
    let id2 = b2.get_device_id().await.unwrap();

    assert_eq!(id1, id2);
}

// md:fn sync_state_persists
#[tokio::test]
async fn sync_state_persists() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let ts = chrono::Utc::now();
    backend.update_sync_time(ts).await.unwrap();

    let read = backend.get_last_sync_time().await.unwrap();
    assert_eq!(
        read.timestamp(),
        ts.timestamp(),
        "Sync timestamp should persist"
    );
}

// md:fn get_changes_since_scans_other_device_logs
#[tokio::test]
async fn get_changes_since_scans_other_device_logs() {
    use keeplin_core::models::Change;

    let dir = tempdir().unwrap();
    let our = FsBackend::new(dir.path()).await.unwrap();

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

// md:fn update_nonexistent_note_returns_not_found
#[tokio::test]
async fn update_nonexistent_note_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let note = Note::new("Ghost", "");
    let err = backend.update_note(note).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

// md:fn delete_nonexistent_note_returns_not_found
#[tokio::test]
async fn delete_nonexistent_note_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let id = uuid::Uuid::new_v4();
    let err = backend.delete_note(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

// md:fn update_nonexistent_notebook_returns_not_found
#[tokio::test]
async fn update_nonexistent_notebook_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let nb = Notebook::new("Ghost");
    let err = backend.update_notebook(nb).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

// md:fn delete_nonexistent_notebook_returns_not_found
#[tokio::test]
async fn delete_nonexistent_notebook_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let id = uuid::Uuid::new_v4();
    let err = backend.delete_notebook(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

// md:fn update_nonexistent_tag_returns_not_found
#[tokio::test]
async fn update_nonexistent_tag_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let tag = Tag::new("ghost");
    let err = backend.update_tag(tag).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

// md:fn delete_nonexistent_tag_returns_not_found
#[tokio::test]
async fn delete_nonexistent_tag_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let id = uuid::Uuid::new_v4();
    let err = backend.delete_tag(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

// md:fn create_and_read_notebook
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

// md:fn list_notebooks_includes_created
#[tokio::test]
async fn list_notebooks_includes_created() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let nb = Notebook::new("Work");
    let id = nb.id;
    backend.create_notebook(nb).await.unwrap();

    let (notebooks, _) = backend.list_notebooks(0, None).await.unwrap();
    assert!(notebooks.iter().any(|n| n.id == id && n.title == "Work"));
}

// md:fn delete_notebook_soft_deletes
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

// md:fn create_and_read_tag
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

// md:fn list_tags_includes_created
#[tokio::test]
async fn list_tags_includes_created() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let tag = Tag::new("rust");
    let id = tag.id;
    backend.create_tag(tag).await.unwrap();

    let (tags, _) = backend.list_tags(0, None).await.unwrap();
    assert!(tags.iter().any(|t| t.id == id && t.title == "rust"));
}

// md:fn add_and_list_note_tags
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

// md:fn add_note_tag_rejects_missing_or_deleted_ends
#[tokio::test]
async fn add_note_tag_rejects_missing_or_deleted_ends() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let note = Note::new("N", "");
    let tag = Tag::new("T");
    let (note_id, tag_id) = (note.id, tag.id);
    backend.create_note(note).await.unwrap();
    backend.create_tag(tag).await.unwrap();

    let err = backend
        .add_note_tag(NoteTag {
            note_id: uuid::Uuid::new_v4(),
            tag_id,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)), "got: {err}");
    let err = backend
        .add_note_tag(NoteTag {
            note_id,
            tag_id: uuid::Uuid::new_v4(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)), "got: {err}");

    backend.delete_tag(tag_id).await.unwrap();
    let err = backend
        .add_note_tag(NoteTag { note_id, tag_id })
        .await
        .unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)), "got: {err}");

    let (tags, _) = backend.list_note_tags(note_id, 0, None).await.unwrap();
    assert!(tags.is_empty());
}

// md:fn remove_note_tag
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

// md:fn create_and_read_resource
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

// md:fn list_resources_excludes_data
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

// md:fn delete_resource
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

// md:fn list_notes_paginates_without_duplicates_or_gaps
#[tokio::test]
async fn list_notes_paginates_without_duplicates_or_gaps() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

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

// md:fn replicate_note
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

// md:fn fs_note_uses_three_file_layout
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

    let body = std::fs::read_to_string(ndir.join("note.md")).unwrap();
    assert_eq!(body, "# Markdown body");
}

// md:fn fs_two_device_causal_sync
#[tokio::test]
async fn fs_two_device_causal_sync() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    let a = FsBackend::new(dir_a.path()).await.unwrap();
    let b = FsBackend::new(dir_b.path()).await.unwrap();

    let note = Note::new("Title", "from A");
    let id = note.id;
    a.create_note(note).await.unwrap();

    replicate_note(dir_a.path(), dir_b.path(), id).await;
    assert_eq!(b.read_note(id).await.unwrap().body, "from A");

    let mut edited = b.read_note(id).await.unwrap();
    edited.body = "edited by B".to_string();
    b.update_note(edited).await.unwrap();

    replicate_note(dir_b.path(), dir_a.path(), id).await;
    assert_eq!(a.read_note(id).await.unwrap().body, "edited by B");
}

// md:fn fs_two_device_concurrent_edits_converge
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

    let mut ea = a.read_note(id).await.unwrap();
    ea.body = "A wins?".to_string();
    a.update_note(ea).await.unwrap();

    let mut eb = b.read_note(id).await.unwrap();
    eb.body = "B wins?".to_string();
    b.update_note(eb).await.unwrap();

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

// md:fn note_alias_bookmarks_links_persist_in_meta
#[tokio::test]
async fn note_alias_bookmarks_links_persist_in_meta() {
    use keeplin_core::links::{Bookmark, LinkSource, NoteLink};

    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let mut note = Note::new("t", "###Bookmark1 [l](#other)");
    note.alias = Some("note3".to_string());
    note.bookmarks = vec![Bookmark {
        number: 1,
        text: "Bookmark1".to_string(),
        alias: "Custom".to_string(),
    }];
    note.links = vec![NoteLink {
        source: LinkSource::Manual,
        raw: "#other".to_string(),
        target_note_id: None,
    }];
    let id = note.id;
    backend.create_note(note.clone()).await.unwrap();

    let read = backend.read_note(id).await.unwrap();
    assert_eq!(read.alias.as_deref(), Some("note3"));
    assert_eq!(read.bookmarks, note.bookmarks);
    assert_eq!(read.links, note.links);

    let backend2 = FsBackend::new(dir.path()).await.unwrap();
    let seen = backend2.read_note(id).await.unwrap();
    assert_eq!(seen.alias.as_deref(), Some("note3"));
    assert_eq!(seen.bookmarks, note.bookmarks);
    assert_eq!(seen.links, note.links);
}

// md:fn backlinks_default_scan_is_paginated
#[tokio::test]
async fn backlinks_default_scan_is_paginated() {
    use keeplin_core::links::{LinkSource, NoteLink};

    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let target = backend.create_note(Note::new("target", "")).await.unwrap();
    for i in 0..3 {
        let mut s = Note::new(format!("s{i}"), "");
        s.links = vec![NoteLink {
            source: LinkSource::Content,
            raw: "#x".to_string(),
            target_note_id: Some(target.id),
        }];
        backend.create_note(s).await.unwrap();
    }

    let (p1, next) = backend.note_backlinks(target.id, 2, None).await.unwrap();
    assert_eq!(p1.len(), 2);
    let cursor = next.expect("a second page");
    let (p2, next2) = backend
        .note_backlinks(target.id, 2, Some(cursor))
        .await
        .unwrap();
    assert_eq!(p2.len(), 1);
    assert!(next2.is_none());
    let ids: std::collections::HashSet<_> = p1.iter().chain(&p2).map(|n| n.id).collect();
    assert_eq!(ids.len(), 3);
}

// md:fn fs_notebook_concurrent_equal_timestamp_edits_converge
#[tokio::test]
async fn fs_notebook_concurrent_equal_timestamp_edits_converge() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    let a = FsBackend::new(dir_a.path()).await.unwrap();
    let b = FsBackend::new(dir_b.path()).await.unwrap();

    let nb = a.create_notebook(Notebook::new("shared")).await.unwrap();
    let id = nb.id;
    b.apply_change(Change::NotebookCreate {
        notebook: nb.clone(),
    })
    .await
    .unwrap();

    let t = Utc::now();
    let mut ea = a.read_notebook(id).await.unwrap();
    ea.title = "from A".to_string();
    ea.updated_at = t;
    let ua = a.update_notebook(ea).await.unwrap();

    let mut eb = b.read_notebook(id).await.unwrap();
    eb.title = "from B".to_string();
    eb.updated_at = t;
    let ub = b.update_notebook(eb).await.unwrap();

    a.apply_change(Change::NotebookUpdate { notebook: ub })
        .await
        .unwrap();
    b.apply_change(Change::NotebookUpdate { notebook: ua })
        .await
        .unwrap();

    let title_a = a.read_notebook(id).await.unwrap().title;
    let title_b = b.read_notebook(id).await.unwrap().title;
    assert_eq!(title_a, title_b, "concurrent notebook edits must converge");
    assert!(title_a == "from A" || title_a == "from B");
}

// md:fn fs_concurrent_note_tag_add_remove_converges
#[tokio::test]
async fn fs_concurrent_note_tag_add_remove_converges() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    let a = FsBackend::new(dir_a.path()).await.unwrap();
    let b = FsBackend::new(dir_b.path()).await.unwrap();

    let note = a.create_note(Note::new("n", "")).await.unwrap();
    let tag = a.create_tag(Tag::new("t")).await.unwrap();
    a.add_note_tag(NoteTag {
        note_id: note.id,
        tag_id: tag.id,
    })
    .await
    .unwrap();
    replicate_logs(dir_a.path(), dir_b.path()).await;
    drain_sync(&b).await;
    assert_eq!(b.list_note_tags(note.id, 0, None).await.unwrap().0.len(), 1);

    a.remove_note_tag(note.id, tag.id).await.unwrap();
    b.add_note_tag(NoteTag {
        note_id: note.id,
        tag_id: tag.id,
    })
    .await
    .unwrap();

    replicate_logs(dir_a.path(), dir_b.path()).await;
    replicate_logs(dir_b.path(), dir_a.path()).await;
    drain_sync(&a).await;
    drain_sync(&b).await;

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
    assert_eq!(
        present_a, present_b,
        "concurrent note↔tag add/remove must converge on FsBackend"
    );
}

// md:fn note_log_len
async fn note_log_len(root: &std::path::Path, id: uuid::Uuid) -> usize {
    use keeplin_core::storage::note_log::NoteLogEntry;
    let dir = root.join("notes").join(id.to_string());
    let mut rd = tokio::fs::read_dir(&dir).await.unwrap();
    while let Some(e) = rd.next_entry().await.unwrap() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("log.") && name.ends_with(".msgpack") {
            let bytes = tokio::fs::read(e.path()).await.unwrap();
            let entries: Vec<NoteLogEntry> = rmp_serde::from_slice(&bytes).unwrap();
            return entries.len();
        }
    }
    panic!("no per-device note log found for {id}");
}

// md:fn fs_note_log_compacts_and_still_converges
#[tokio::test]
async fn fs_note_log_compacts_and_still_converges() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    let a = FsBackend::new(dir_a.path()).await.unwrap();
    let b = FsBackend::new(dir_b.path()).await.unwrap();

    let note = Note::new("t", "v0");
    let id = note.id;
    a.create_note(note.clone()).await.unwrap();

    let edits = 1000u64;
    for i in 1..=edits {
        let mut edited = note.clone();
        edited.body = format!("v{i}");
        a.update_note(edited).await.unwrap();
    }

    let len = note_log_len(dir_a.path(), id).await;
    assert!(
        len <= 257,
        "compacted per-note log must stay bounded, had {len} entries"
    );
    assert_eq!(a.read_note(id).await.unwrap().body, format!("v{edits}"));

    replicate_note(dir_a.path(), dir_b.path(), id).await;
    assert_eq!(b.read_note(id).await.unwrap().body, format!("v{edits}"));

    a.delete_note(id).await.unwrap();
    replicate_note(dir_a.path(), dir_b.path(), id).await;
    assert!(
        b.read_note(id).await.unwrap().deleted_at.is_some(),
        "the delete must converge on the peer after compaction"
    );
}

// md:fn replicate_logs
async fn replicate_logs(from: &std::path::Path, to: &std::path::Path) {
    let from_logs = from.join("logs");
    let to_logs = to.join("logs");
    tokio::fs::create_dir_all(&to_logs).await.unwrap();
    let mut rd = tokio::fs::read_dir(&from_logs).await.unwrap();
    while let Some(e) = rd.next_entry().await.unwrap() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".log") {
            tokio::fs::copy(e.path(), to_logs.join(&name))
                .await
                .unwrap();
        }
    }

    let from_notes = from.join("notes");
    if let Ok(mut notes_rd) = tokio::fs::read_dir(&from_notes).await {
        while let Some(note_dir) = notes_rd.next_entry().await.unwrap() {
            let to_note_dir = to.join("notes").join(note_dir.file_name());
            tokio::fs::create_dir_all(&to_note_dir).await.unwrap();
            let mut files = tokio::fs::read_dir(note_dir.path()).await.unwrap();
            while let Some(f) = files.next_entry().await.unwrap() {
                let name = f.file_name().to_string_lossy().into_owned();
                if name.starts_with("log.") && name.ends_with(".msgpack") {
                    tokio::fs::copy(f.path(), to_note_dir.join(&name))
                        .await
                        .unwrap();
                }
            }
        }
    }
}

// md:fn drain_sync
async fn drain_sync(b: &FsBackend) {
    for c in b.receive_changes().await.unwrap() {
        b.apply_change(c).await.unwrap();
    }
}

// md:fn own_log_stats
async fn own_log_stats(root: &std::path::Path, backend: &FsBackend) -> (u64, usize) {
    let device = backend.get_device_id().await.unwrap();
    let path = root.join("logs").join(format!("{device}.log"));
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    let mut epoch = 0u64;
    let mut count = 0usize;
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.contains("__keeplin_epoch__") {
            epoch = t
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0);
        } else {
            count += 1;
        }
    }
    (epoch, count)
}

// md:fn fs_global_log_compacts_and_peer_converges
#[tokio::test]
async fn fs_global_log_compacts_and_peer_converges() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    let a = FsBackend::new(dir_a.path()).await.unwrap();
    let b = FsBackend::new(dir_b.path()).await.unwrap();

    let x = a.create_notebook(Notebook::new("x0")).await.unwrap();
    let y = a.create_notebook(Notebook::new("y0")).await.unwrap();
    replicate_logs(dir_a.path(), dir_b.path()).await;
    drain_sync(&b).await;
    assert_eq!(b.read_notebook(x.id).await.unwrap().title, "x0");
    assert_eq!(b.read_notebook(y.id).await.unwrap().title, "y0");

    for i in 1..=600u64 {
        let mut e = a.read_notebook(x.id).await.unwrap();
        e.title = format!("x{i}");
        a.update_notebook(e).await.unwrap();
    }
    a.delete_notebook(y.id).await.unwrap();

    let (epoch, entry_count) = own_log_stats(dir_a.path(), &a).await;
    assert!(
        epoch >= 1,
        "the global log must have compacted at least once"
    );
    assert!(
        entry_count < 600,
        "snapshot compaction must bound the log, had {entry_count} entries"
    );

    replicate_logs(dir_a.path(), dir_b.path()).await;
    drain_sync(&b).await;
    assert_eq!(
        b.read_notebook(x.id).await.unwrap().title,
        "x600",
        "peer converges on the latest state through the snapshot"
    );
    assert!(
        b.read_notebook(y.id).await.unwrap().deleted_at.is_some(),
        "a tombstone carried in the snapshot must still delete on the peer"
    );
}

// md:fn fs_global_log_snapshot_covers_all_entity_types
#[tokio::test]
async fn fs_global_log_snapshot_covers_all_entity_types() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    let a = FsBackend::new(dir_a.path()).await.unwrap();
    let b = FsBackend::new(dir_b.path()).await.unwrap();

    let nb = a.create_notebook(Notebook::new("nb")).await.unwrap();
    let tag = a.create_tag(Tag::new("tag")).await.unwrap();
    let note = a.create_note(Note::new("n", "")).await.unwrap();
    a.add_note_tag(NoteTag {
        note_id: note.id,
        tag_id: tag.id,
    })
    .await
    .unwrap();
    let res = Resource::new("f", "text/plain", "f.txt", 3);
    let res_id = res.id;
    a.create_resource(res, b"abc".to_vec()).await.unwrap();

    for i in 1..=560u64 {
        let mut e = a.read_notebook(nb.id).await.unwrap();
        e.title = format!("nb{i}");
        a.update_notebook(e).await.unwrap();
    }
    let (epoch, _) = own_log_stats(dir_a.path(), &a).await;
    assert!(epoch >= 1, "the global log must have compacted");

    replicate_logs(dir_a.path(), dir_b.path()).await;
    drain_sync(&b).await;

    assert_eq!(b.read_notebook(nb.id).await.unwrap().title, "nb560");
    assert_eq!(b.read_tag(tag.id).await.unwrap().title, "tag");
    let (resources, _) = b.list_resources(0, None).await.unwrap();
    assert!(
        resources.iter().any(|r| r.id == res_id),
        "the resource must be reconstructed from the snapshot"
    );
    let (tags, _) = b.list_note_tags(note.id, 0, None).await.unwrap();
    assert!(
        tags.iter().any(|t| t.id == tag.id),
        "the note↔tag association must be reconstructed from the snapshot"
    );
}

// md:fn ordering_fields_round_trip_and_manual_order_query
#[tokio::test]
async fn ordering_fields_round_trip_and_manual_order_query() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let nb = backend.create_notebook(Notebook::new("nb")).await.unwrap();

    let mut pinned = Note::new("pinned", "");
    pinned.notebook_id = nb.id;
    pinned.is_pinned = true;
    pinned.sort_key = 5;
    let mut legacy = Note::new("legacy", "");
    legacy.notebook_id = nb.id;
    let mut normal = Note::new("normal", "");
    normal.notebook_id = nb.id;
    normal.sort_key = 1500;
    let mut starred = Note::new("starred", "");
    starred.is_starred = true;
    for n in [&pinned, &legacy, &normal, &starred] {
        backend.create_note(n.clone()).await.unwrap();
    }

    let read = backend.read_note(pinned.id).await.unwrap();
    assert!(read.is_pinned);
    assert_eq!(read.sort_key, 5);

    let mut walked = Vec::new();
    let mut token = None;
    loop {
        let (page, next) = backend
            .list_notes_in_notebook(nb.id, 1, token)
            .await
            .unwrap();
        walked.extend(page.into_iter().map(|n| n.title));
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    assert_eq!(walked, ["pinned", "legacy", "normal"]);

    let (stars, _) = backend.list_starred_notes(0, None).await.unwrap();
    assert_eq!(stars.len(), 1);
    assert_eq!(stars[0].title, "starred");

    let profile = backend.notebook_sort_profile(nb.id).await.unwrap();
    assert_eq!(profile.pinned_keys, [5]);
    assert_eq!(profile.max_normal_key, Some(1500));
}

// md:fn note_index_reflects_local_writes_after_it_is_built
#[tokio::test]
async fn note_index_reflects_local_writes_after_it_is_built() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let a = backend.create_note(Note::new("a", "")).await.unwrap();
    let (page, _) = backend.list_notes(0, None).await.unwrap();
    assert_eq!(page.len(), 1);

    let b = backend.create_note(Note::new("b", "")).await.unwrap();
    let mut ids: Vec<_> = backend
        .list_notes(0, None)
        .await
        .unwrap()
        .0
        .into_iter()
        .map(|n| n.id)
        .collect();
    ids.sort();
    let mut want = vec![a.id, b.id];
    want.sort();
    assert_eq!(ids, want);

    backend.delete_note(a.id).await.unwrap();
    let ids: Vec<_> = backend
        .list_notes(0, None)
        .await
        .unwrap()
        .0
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(ids, vec![b.id]);
}

// md:fn note_index_reflects_changes_pulled_from_a_peer
#[tokio::test]
async fn note_index_reflects_changes_pulled_from_a_peer() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    let a = FsBackend::new(dir_a.path()).await.unwrap();
    let b = FsBackend::new(dir_b.path()).await.unwrap();

    assert!(b.list_notes(0, None).await.unwrap().0.is_empty());

    let mut note = Note::new("from A", "body");
    note.is_starred = true;
    let id = note.id;
    a.create_note(note).await.unwrap();
    replicate_logs(dir_a.path(), dir_b.path()).await;

    drain_sync(&b).await;
    let (page, _) = b.list_notes(0, None).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].id, id);
    let (starred, _) = b.list_starred_notes(0, None).await.unwrap();
    assert_eq!(starred.len(), 1);
    assert_eq!(starred[0].id, id);
}

// md:fn delete_for_unknown_sidecar_entity_leaves_a_tombstone_blocking_a_stale_create
#[tokio::test]
async fn delete_for_unknown_sidecar_entity_leaves_a_tombstone_blocking_a_stale_create() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let vv = |dev: &str, n: u64| std::collections::BTreeMap::from([(dev.to_string(), n)]);
    let ts = Utc::now();

    let nb_id = uuid::Uuid::new_v4();
    backend
        .apply_change(Change::NotebookDelete {
            id: nb_id,
            deleted_at: ts,
            vv: vv("peer", 2),
            last_writer: "peer".into(),
        })
        .await
        .unwrap();
    let mut nb = Notebook::new("resurrected?");
    nb.id = nb_id;
    nb.vv = vv("peer", 1);
    nb.last_writer = "peer".into();
    backend
        .apply_change(Change::NotebookCreate { notebook: nb })
        .await
        .unwrap();
    let (nbs, _) = backend.list_notebooks(0, None).await.unwrap();
    assert!(
        !nbs.iter().any(|n| n.id == nb_id),
        "a stale create must not resurrect a notebook deleted before it was known"
    );

    let tag_id = uuid::Uuid::new_v4();
    backend
        .apply_change(Change::TagDelete {
            id: tag_id,
            deleted_at: ts,
            vv: vv("peer", 2),
            last_writer: "peer".into(),
        })
        .await
        .unwrap();
    let mut tag = Tag::new("resurrected?");
    tag.id = tag_id;
    tag.vv = vv("peer", 1);
    tag.last_writer = "peer".into();
    backend
        .apply_change(Change::TagCreate { tag })
        .await
        .unwrap();
    let (tags, _) = backend.list_tags(0, None).await.unwrap();
    assert!(!tags.iter().any(|t| t.id == tag_id), "tag stays deleted");

    let res_id = uuid::Uuid::new_v4();
    backend
        .apply_change(Change::ResourceDelete {
            id: res_id,
            deleted_at: ts,
            vv: vv("peer", 2),
            last_writer: "peer".into(),
        })
        .await
        .unwrap();
    let res = Resource {
        id: res_id,
        title: "resurrected?".into(),
        mime_type: "text/plain".into(),
        file_name: "f.txt".into(),
        size: 3,
        created_at: ts,
        deleted_at: None,
        vv: vv("peer", 1),
        last_writer: "peer".into(),
    };
    backend
        .apply_change(Change::ResourceCreate {
            resource: res,
            data: Some(b"abc".to_vec()),
        })
        .await
        .unwrap();
    let (resources, _) = backend.list_resources(0, None).await.unwrap();
    assert!(
        !resources.iter().any(|r| r.id == res_id),
        "resource stays deleted"
    );
}
