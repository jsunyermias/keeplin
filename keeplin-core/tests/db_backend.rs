//! Integration tests for [`DbBackend`] — the LibSQL-backed storage implementation.
//!
//! Every test in this module uses [`in_memory_backend`], a helper that creates a
//! [`DbBackend`] backed by a temporary file with an empty `server_url` so no
//! WebSocket connection is attempted. The tests cover the complete
//! [`StorageBackend`] API at the SQLite level, including soft-deletion semantics,
//! the `entity_changes` change journal, and device-ID persistence. WebSocket
//! synchronisation paths are not exercised here because they require a live server.

use keeplin_core::{
    error::StorageError,
    models::{Note, NoteTag, Notebook, Resource, Tag},
    storage::{
        db::DbBackend, NoteRepository, NotebookRepository, ResourceRepository, SyncBackend,
        TagRepository,
    },
};
use tempfile::tempdir;

/// Create a `DbBackend` backed by a temporary file database with no server URL.
///
/// Passing an empty string for `server_url` and `auth_token` puts the backend in
/// offline mode so no WebSocket connection is attempted. The `tempdir` is intentionally
/// leaked with `std::mem::forget` to prevent the temporary directory from being deleted
/// before the test completes — the directory must stay alive as long as the database
/// file is open.
async fn in_memory_backend() -> DbBackend {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    // The temporary directory must outlive the database connection. Leaking it here
    // prevents the destructor from deleting the directory while the database file
    // is still open. The OS will clean up the temporary directory when the process exits.
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

    let (notes, _) = backend.list_notes(0, None).await.unwrap();
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

    let (notes, _) = backend.list_notes(0, None).await.unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].title, "Keep");
}

#[tokio::test]
async fn read_nonexistent_returns_not_found() {
    let backend = in_memory_backend().await;
    let id = uuid::Uuid::new_v4();
    let err = backend.read_note(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
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
    // A note that was created (not merely updated) after `since` must appear
    // in the change list as `Change::NoteCreate`, not `Change::NoteUpdate`,
    // because the `entity_changes` journal records the original operation type.
    assert!(matches!(changes[0], Change::NoteCreate { .. }));
}

#[tokio::test]
async fn apply_change_is_not_re_journaled() {
    use keeplin_core::models::Change;

    // The `entity_changes` journal holds only changes that ORIGINATED on this device.
    // A change ingested via `apply_change` (a remote change pulled from the broadcast relay)
    // must be applied to the tables but must NOT enter the journal, so it is never re-sent
    // to the relay. See the invariant documented on `DbBackend::apply_change`.
    let backend = in_memory_backend().await;
    let epoch = chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap();

    let remote = Note::new("remote", "from a peer");
    let remote_id = remote.id;
    backend
        .apply_change(Change::NoteCreate { note: remote })
        .await
        .unwrap();
    // It really was applied (readable from the tables)…
    assert_eq!(backend.read_note(remote_id).await.unwrap().title, "remote");

    // …and a locally created note DOES enter the journal, for contrast.
    let local = Note::new("local", "mine");
    let local_id = local.id;
    backend.create_note(local).await.unwrap();

    let journaled: Vec<_> = backend
        .get_changes_since(epoch)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|c| match c {
            Change::NoteCreate { note } | Change::NoteUpdate { note } => Some(note.id),
            _ => None,
        })
        .collect();
    assert!(
        journaled.contains(&local_id),
        "a locally created note must be journaled"
    );
    assert!(
        !journaled.contains(&remote_id),
        "a change applied via apply_change must NOT be re-journaled"
    );
}

// ── Error-path tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn update_nonexistent_note_returns_not_found() {
    let backend = in_memory_backend().await;
    let note = Note::new("Ghost", "");
    let err = backend.update_note(note).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

#[tokio::test]
async fn delete_nonexistent_note_returns_not_found() {
    let backend = in_memory_backend().await;
    let id = uuid::Uuid::new_v4();
    let err = backend.delete_note(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

#[tokio::test]
async fn update_nonexistent_notebook_returns_not_found() {
    let backend = in_memory_backend().await;
    let nb = Notebook::new("Ghost");
    let err = backend.update_notebook(nb).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

#[tokio::test]
async fn delete_nonexistent_notebook_returns_not_found() {
    let backend = in_memory_backend().await;
    let id = uuid::Uuid::new_v4();
    let err = backend.delete_notebook(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

#[tokio::test]
async fn update_nonexistent_tag_returns_not_found() {
    let backend = in_memory_backend().await;
    let tag = Tag::new("ghost");
    let err = backend.update_tag(tag).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

#[tokio::test]
async fn delete_nonexistent_tag_returns_not_found() {
    let backend = in_memory_backend().await;
    let id = uuid::Uuid::new_v4();
    let err = backend.delete_tag(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
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

    let (list, _) = backend.list_notebooks(0, None).await.unwrap();
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

    let (tags, _) = backend.list_note_tags(note_id, 0, None).await.unwrap();
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
    let (tags, _) = backend.list_note_tags(note_id, 0, None).await.unwrap();
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

    let (list, _) = backend.list_resources(0, None).await.unwrap();
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
    assert!(matches!(err, StorageError::NotFound(_)));
}

// ── Pagination tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn list_notes_paginates_without_duplicates_or_gaps() {
    let backend = in_memory_backend().await;

    // Insert more notes than a single page holds so the cursor must be walked.
    let total = 25usize;
    for i in 0..total {
        backend
            .create_note(Note::new(format!("Note {i:02}"), ""))
            .await
            .unwrap();
    }

    // Walk every page with a small page size and collect the ids in order.
    let page_size = 10u32;
    let mut seen = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let (page, next) = backend.list_notes(page_size, token).await.unwrap();
        assert!(
            page.len() <= page_size as usize,
            "page must never exceed page_size"
        );
        seen.extend(page.iter().map(|n| n.id));
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }

    // Every note must appear exactly once across all pages.
    assert_eq!(
        seen.len(),
        total,
        "every note must be returned exactly once"
    );
    let unique: std::collections::HashSet<_> = seen.iter().copied().collect();
    assert_eq!(unique.len(), total, "no note may appear on two pages");

    // The keyset order (created_at ASC, id ASC) must be stable across the walk.
    let (all, _) = backend.list_notes(total as u32 + 5, None).await.unwrap();
    let all_ids: Vec<_> = all.iter().map(|n| n.id).collect();
    assert_eq!(seen, all_ids, "paged order must match single-shot order");
}

// ── Concurrency test ──────────────────────────────────────────────────────────

/// Many writers hitting the same `DbBackend` concurrently must all succeed.
///
/// `DbBackend` wraps every mutation in a `BEGIN IMMEDIATE … COMMIT` transaction on a
/// single shared connection, so without serialisation a second `BEGIN` arriving before
/// the first `COMMIT` fails with "cannot start a transaction within a transaction".
/// This test runs on a multi-threaded runtime to maximise the chance of interleaving.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_note_creates_all_succeed() {
    use std::sync::Arc;

    let backend = Arc::new(in_memory_backend().await);

    let mut handles = Vec::new();
    for i in 0..50u32 {
        let b = Arc::clone(&backend);
        handles.push(tokio::spawn(async move {
            b.create_note(Note::new(format!("concurrent {i}"), ""))
                .await
        }));
    }

    let mut ok = 0usize;
    for h in handles {
        h.await
            .unwrap()
            .expect("concurrent create_note must succeed");
        ok += 1;
    }
    assert_eq!(ok, 50, "all concurrent creates must commit");

    // All 50 notes must be queryable afterwards (none lost to a failed transaction).
    let (notes, _) = backend.list_notes(100, None).await.unwrap();
    assert_eq!(notes.len(), 50);
}

/// Concurrent readers and writers must all make progress and complete — the read/write
/// guard around the shared connection must never deadlock (a reader must not block a
/// reader, and the read and write sides must not be acquired re-entrantly by one task).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reads_and_writes_make_progress() {
    use std::sync::Arc;

    let backend = Arc::new(in_memory_backend().await);

    // Seed a note so reads have something to return.
    let seed = Note::new("seed", "");
    let seed_id = seed.id;
    backend.create_note(seed).await.unwrap();

    let mut handles = Vec::new();
    for i in 0..20u32 {
        let writer = Arc::clone(&backend);
        handles.push(tokio::spawn(async move {
            writer
                .create_note(Note::new(format!("w{i}"), ""))
                .await
                .map(|_| ())
        }));
        let reader = Arc::clone(&backend);
        handles.push(tokio::spawn(async move {
            // Mix point reads and list reads to exercise both read paths.
            let _ = reader.read_note(seed_id).await;
            reader.list_notes(10, None).await.map(|_| ())
        }));
    }

    for h in handles {
        h.await.unwrap().expect("no read or write may fail or hang");
    }

    let (notes, _) = backend.list_notes(100, None).await.unwrap();
    assert_eq!(notes.len(), 21, "seed + 20 writers");
}

#[tokio::test]
async fn note_alias_bookmarks_links_round_trip() {
    use keeplin_core::links::{Bookmark, LinkSource, NoteLink};

    let backend = in_memory_backend().await;
    let mut note = Note::new("titled", "###Bookmark1 and a [link](#other)");
    note.alias = Some("note3".to_string());
    note.bookmarks = vec![Bookmark {
        number: 1,
        text: "Bookmark1".to_string(),
        alias: "Custom".to_string(),
    }];
    note.links = vec![NoteLink {
        source: LinkSource::Content,
        raw: "#other".to_string(),
        target_note_id: None,
    }];
    let created = backend.create_note(note.clone()).await.unwrap();
    // Content is preserved verbatim; `create_note` additionally stamps the version vector
    // and author for conflict resolution, so compare the content fields explicitly.
    assert_eq!(created.id, note.id);
    assert_eq!(created.title, note.title);
    assert_eq!(created.body, note.body);
    assert_eq!(created.alias, note.alias);
    assert_eq!(created.bookmarks, note.bookmarks);
    assert_eq!(created.links, note.links);
    assert!(
        !created.vv.is_empty(),
        "create_note stamps a version vector"
    );
    assert!(
        !created.last_writer.is_empty(),
        "create_note records the author"
    );

    // Read back: alias, bookmarks and links survive the SQLite columns.
    let read = backend.read_note(note.id).await.unwrap();
    assert_eq!(read.alias.as_deref(), Some("note3"));
    assert_eq!(read.bookmarks, note.bookmarks);
    assert_eq!(read.links, note.links);

    // Update the alias and a bookmark; verify persistence.
    let mut edited = read;
    edited.alias = Some("renamed".to_string());
    edited.bookmarks[0].alias = "Edited".to_string();
    backend.update_note(edited.clone()).await.unwrap();
    let reread = backend.read_note(note.id).await.unwrap();
    assert_eq!(reread.alias.as_deref(), Some("renamed"));
    assert_eq!(reread.bookmarks[0].alias, "Edited");
}

#[tokio::test]
async fn notebook_alias_round_trip() {
    let backend = in_memory_backend().await;
    let mut nb = Notebook::new("Work");
    nb.alias = Some("notebook1".to_string());
    backend.create_notebook(nb.clone()).await.unwrap();
    let read = backend.read_notebook(nb.id).await.unwrap();
    assert_eq!(read.alias.as_deref(), Some("notebook1"));

    let (list, _) = backend.list_notebooks(10, None).await.unwrap();
    assert_eq!(list[0].alias.as_deref(), Some("notebook1"));
}

#[tokio::test]
async fn indexed_backlinks_track_writes_and_deletes() {
    use keeplin_core::links::{LinkSource, NoteLink};

    let backend = in_memory_backend().await;
    let target = backend.create_note(Note::new("target", "")).await.unwrap();

    let link_to = |id| NoteLink {
        source: LinkSource::Content,
        raw: "#x".to_string(),
        target_note_id: Some(id),
    };

    let mut src1 = Note::new("src1", "");
    src1.links = vec![link_to(target.id)];
    let src1 = backend.create_note(src1).await.unwrap();

    let mut src2 = Note::new("src2", "");
    src2.links = vec![link_to(target.id)];
    let src2 = backend.create_note(src2).await.unwrap();

    // An unrelated note must not appear as a backlink.
    backend.create_note(Note::new("other", "")).await.unwrap();

    let (back, _) = backend.note_backlinks(target.id, 0, None).await.unwrap();
    assert_eq!(back.len(), 2, "both sources link to target");

    // Removing src1's link (update) drops it from the index.
    let mut s = src1.clone();
    s.links.clear();
    backend.update_note(s).await.unwrap();
    let (back, _) = backend.note_backlinks(target.id, 0, None).await.unwrap();
    assert_eq!(back.len(), 1);

    // Soft-deleting src2 excludes it from backlinks (the JOIN filters deleted sources).
    backend.delete_note(src2.id).await.unwrap();
    let (back, _) = backend.note_backlinks(target.id, 0, None).await.unwrap();
    assert!(back.is_empty());
}

#[tokio::test]
async fn backlinks_are_paginated() {
    use keeplin_core::links::{LinkSource, NoteLink};

    let backend = in_memory_backend().await;
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
    assert!(next2.is_none(), "no third page");

    // The two pages cover all three distinct sources without overlap.
    let ids: std::collections::HashSet<_> = p1.iter().chain(&p2).map(|n| n.id).collect();
    assert_eq!(ids.len(), 3);
}
