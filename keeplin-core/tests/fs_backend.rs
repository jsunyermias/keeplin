//! Integration tests for [`FsBackend`] — the filesystem-backed storage implementation.
//!
//! Every test in this module creates a fresh temporary directory with [`tempfile::tempdir`],
//! constructs a new [`FsBackend`] rooted there, and exercises the full
//! [`StorageBackend`] API against real files on disk. The tests verify both the
//! happy path (create → read → update → delete) and error paths (operations on
//! non-existent entities must return [`StorageError::NotFound`]).

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
async fn list_notebooks_includes_created() {
    // Regression: the sidecar is written as `{id}.msgpack`, so the listing must filter on
    // that extension. A previous `.json` filter matched nothing and returned an empty list.
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let nb = Notebook::new("Work");
    let id = nb.id;
    backend.create_notebook(nb).await.unwrap();

    let (notebooks, _) = backend.list_notebooks(0, None).await.unwrap();
    assert!(notebooks.iter().any(|n| n.id == id && n.title == "Work"));
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
async fn list_tags_includes_created() {
    // Regression: same `.msgpack`-vs-`.json` listing bug as notebooks, for tags.
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let tag = Tag::new("rust");
    let id = tag.id;
    backend.create_tag(tag).await.unwrap();

    let (tags, _) = backend.list_tags(0, None).await.unwrap();
    assert!(tags.iter().any(|t| t.id == id && t.title == "rust"));
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
async fn add_note_tag_rejects_missing_or_deleted_ends() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let note = Note::new("N", "");
    let tag = Tag::new("T");
    let (note_id, tag_id) = (note.id, tag.id);
    backend.create_note(note).await.unwrap();
    backend.create_tag(tag).await.unwrap();

    // Nonexistent note / tag: no dangling association may be created.
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

    // Soft-deleted ends are rejected the same way.
    backend.delete_tag(tag_id).await.unwrap();
    let err = backend
        .add_note_tag(NoteTag { note_id, tag_id })
        .await
        .unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)), "got: {err}");

    // Nothing was attached by the failed calls.
    let (tags, _) = backend.list_note_tags(note_id, 0, None).await.unwrap();
    assert!(tags.is_empty());
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

    // Reads materialize from the per-device log; the new fields survive the round-trip
    // through `log.{device}.msgpack` + `meta.msgpack`.
    let read = backend.read_note(id).await.unwrap();
    assert_eq!(read.alias.as_deref(), Some("note3"));
    assert_eq!(read.bookmarks, note.bookmarks);
    assert_eq!(read.links, note.links);

    // A second backend over the same root (a different "device") materializes the same
    // state from the replicated log — i.e. the fields converge.
    let backend2 = FsBackend::new(dir.path()).await.unwrap();
    let seen = backend2.read_note(id).await.unwrap();
    assert_eq!(seen.alias.as_deref(), Some("note3"));
    assert_eq!(seen.bookmarks, note.bookmarks);
    assert_eq!(seen.links, note.links);
}

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

/// Two `FsBackend` devices editing the same notebook concurrently with the **identical**
/// `updated_at` converge on one deterministic winner via version-vector `resolve`. Under the
/// old `updated_at`-only comparison (`>`), equal timestamps meant neither device applied the
/// other's edit → permanent divergence.
#[tokio::test]
async fn fs_notebook_concurrent_equal_timestamp_edits_converge() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    let a = FsBackend::new(dir_a.path()).await.unwrap();
    let b = FsBackend::new(dir_b.path()).await.unwrap();

    // Shared baseline: create on A, replicate the create to B (B now holds vv {A:1}).
    let nb = a.create_notebook(Notebook::new("shared")).await.unwrap();
    let id = nb.id;
    b.apply_change(Change::NotebookCreate {
        notebook: nb.clone(),
    })
    .await
    .unwrap();

    // Concurrent edits sharing the SAME updated_at.
    let t = Utc::now();
    let mut ea = a.read_notebook(id).await.unwrap();
    ea.title = "from A".to_string();
    ea.updated_at = t;
    let ua = a.update_notebook(ea).await.unwrap();

    let mut eb = b.read_notebook(id).await.unwrap();
    eb.title = "from B".to_string();
    eb.updated_at = t;
    let ub = b.update_notebook(eb).await.unwrap();

    // Exchange the concurrent edits.
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

/// A concurrent note↔tag attach-vs-detach converges on `FsBackend` exactly as on `DbBackend`:
/// both devices agree on the association's final presence, resolved through the shared version
/// vectors rather than being order-dependent. This is the FS mirror of
/// `sync::db_concurrent_note_tag_add_remove_converges`.
#[tokio::test]
async fn fs_concurrent_note_tag_add_remove_converges() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    let a = FsBackend::new(dir_a.path()).await.unwrap();
    let b = FsBackend::new(dir_b.path()).await.unwrap();

    // Baseline: note + tag + attached association on A, replicated to B.
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

    // Concurrent: A detaches, B re-attaches (each from the shared baseline).
    a.remove_note_tag(note.id, tag.id).await.unwrap();
    b.add_note_tag(NoteTag {
        note_id: note.id,
        tag_id: tag.id,
    })
    .await
    .unwrap();

    // Exchange both ways through the replicated logs.
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

/// Count the entries in a note's single per-device log file (the `log.*.msgpack` in its dir).
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

/// A note edited far past the compaction threshold keeps its own per-device log bounded (the
/// log is collapsed to its frontier) while reads still return the latest content, and the
/// compacted log replicates to a peer that converges on the same state — including after a
/// delete, whose tombstone still recovers its content from the retained newest upsert.
#[tokio::test]
async fn fs_note_log_compacts_and_still_converges() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    let a = FsBackend::new(dir_a.path()).await.unwrap();
    let b = FsBackend::new(dir_b.path()).await.unwrap();

    let note = Note::new("t", "v0");
    let id = note.id;
    a.create_note(note.clone()).await.unwrap();

    // Edit well past the compaction threshold so compaction fires repeatedly.
    let edits = 1000u64;
    for i in 1..=edits {
        let mut edited = note.clone();
        edited.body = format!("v{i}");
        a.update_note(edited).await.unwrap();
    }

    // Despite 1000 edits, the log is bounded to at most the threshold (+1): each time it grows
    // past 256 entries it is collapsed back to its frontier, rather than growing to 1001.
    let len = note_log_len(dir_a.path(), id).await;
    assert!(
        len <= 257,
        "compacted per-note log must stay bounded, had {len} entries"
    );
    assert_eq!(a.read_note(id).await.unwrap().body, format!("v{edits}"));

    // The compacted log replicates to a fresh peer, which converges on the latest content.
    replicate_note(dir_a.path(), dir_b.path(), id).await;
    assert_eq!(b.read_note(id).await.unwrap().body, format!("v{edits}"));

    // Deleting after all that churn still produces a tombstone that carries the recovered
    // content (the newest upsert was retained by compaction), and it propagates.
    a.delete_note(id).await.unwrap();
    replicate_note(dir_a.path(), dir_b.path(), id).await;
    assert!(
        b.read_note(id).await.unwrap().deleted_at.is_some(),
        "the delete must converge on the peer after compaction"
    );
}

/// Simulate Syncthing replicating one device's single-writer log files to another: every
/// global `logs/*.log` file plus every per-note `notes/{id}/log.*.msgpack` op log. Each has a
/// single writer, so this never conflicts. Projections (`note.md`, `meta.msgpack`) are *not*
/// copied — they are per-device caches the receiver regenerates from the logs on sync.
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

/// Pull and apply every change a device can currently see from its peers' replicated logs.
async fn drain_sync(b: &FsBackend) {
    for c in b.receive_changes().await.unwrap() {
        b.apply_change(c).await.unwrap();
    }
}

/// The generation epoch and change-entry count of a device's own global log (parsing the log
/// text directly, without depending on `FsBackend` internals).
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

/// The global NDJSON journal is bounded by generation-epoch snapshot compaction: churning one
/// notebook far past the threshold rewrites the log as a small current-state snapshot behind a
/// bumped epoch, and a peer that already synced the pre-compaction baseline detects the new
/// generation, re-reads the snapshot, and converges — a live entity at its latest state and a
/// deleted one tombstoned.
#[tokio::test]
async fn fs_global_log_compacts_and_peer_converges() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    let a = FsBackend::new(dir_a.path()).await.unwrap();
    let b = FsBackend::new(dir_b.path()).await.unwrap();

    // Baseline: two notebooks on A, synced to B before any compaction.
    let x = a.create_notebook(Notebook::new("x0")).await.unwrap();
    let y = a.create_notebook(Notebook::new("y0")).await.unwrap();
    replicate_logs(dir_a.path(), dir_b.path()).await;
    drain_sync(&b).await;
    assert_eq!(b.read_notebook(x.id).await.unwrap().title, "x0");
    assert_eq!(b.read_notebook(y.id).await.unwrap().title, "y0");

    // Churn X well past the global-log threshold (512) to force at least one snapshot
    // compaction, and delete Y so a tombstone must survive in the snapshot.
    for i in 1..=600u64 {
        let mut e = a.read_notebook(x.id).await.unwrap();
        e.title = format!("x{i}");
        a.update_notebook(e).await.unwrap();
    }
    a.delete_notebook(y.id).await.unwrap();

    // A's own log was compacted: it carries a generation header (epoch >= 1) and far fewer
    // entries than the ~601 mutations, because each notebook collapsed to one snapshot entry.
    let (epoch, entry_count) = own_log_stats(dir_a.path(), &a).await;
    assert!(
        epoch >= 1,
        "the global log must have compacted at least once"
    );
    assert!(
        entry_count < 600,
        "snapshot compaction must bound the log, had {entry_count} entries"
    );

    // B — which had synced only the baseline (epoch 0) — notices the new generation, re-reads
    // the snapshot from the header, and converges on the latest state of both notebooks.
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

/// The snapshot written on global-log compaction covers **every** globally-journalled entity
/// type — notebooks, tags, resources, and note↔tag associations — so a fresh peer that only ever
/// receives the post-compaction snapshot still reconstructs all of them.
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

    // Force at least one global-log compaction by churning the notebook past the threshold.
    for i in 1..=560u64 {
        let mut e = a.read_notebook(nb.id).await.unwrap();
        e.title = format!("nb{i}");
        a.update_notebook(e).await.unwrap();
    }
    let (epoch, _) = own_log_stats(dir_a.path(), &a).await;
    assert!(epoch >= 1, "the global log must have compacted");

    // A brand-new peer receives only the compacted snapshot, yet reconstructs each entity type.
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
