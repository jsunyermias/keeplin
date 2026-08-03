// md:Overview
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::error::StorageError;
use crate::models::{now, Change, Note, NoteTag, Notebook, Resource, Tag, SYSTEM_RESOURCE_NOTE_ID};
use crate::storage::note_log::{self, VersionVector};
use crate::storage::{
    NoteRepository, NotebookRepository, ResourceRepository, SyncBackend, TagRepository,
};

use super::io::atomic_write;
use super::resources::content_hash;
use super::FsBackend;

// md:fn concurrent_same_note_updates_keep_every_log_entry
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_same_note_updates_keep_every_log_entry() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(FsBackend::new(dir.path()).await.unwrap());
    let note = backend.create_note(Note::new("t", "v0")).await.unwrap();
    let id = note.id;

    let updates = 20usize;
    let mut handles = Vec::new();
    for i in 0..updates {
        let b = Arc::clone(&backend);
        let mut edited = note.clone();
        handles.push(tokio::spawn(async move {
            edited.body = format!("v{i}");
            b.update_note(edited).await.unwrap();
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    let logs = backend.read_note_logs(id).await.unwrap();
    let total: usize = logs.iter().map(|l| l.len()).sum();
    assert_eq!(total, 1 + updates, "create + {updates} updates, none lost");
}

// md:fn read_does_not_rewrite_projection
#[tokio::test]
async fn read_does_not_rewrite_projection() {
    let dir = tempfile::tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let note = backend.create_note(Note::new("t", "body")).await.unwrap();
    let meta = backend.note_meta_path(note.id);
    let md = backend.note_md_path(note.id);

    tokio::fs::remove_file(&meta).await.unwrap();
    tokio::fs::remove_file(&md).await.unwrap();

    let read = backend.read_note(note.id).await.unwrap();
    assert_eq!(read.body, "body");
    assert!(!meta.exists(), "read must not rewrite meta.ndjson");
    assert!(!md.exists(), "read must not rewrite note.md");
}

// md:fn list_notes_pages_match_full_walk
#[tokio::test]
async fn list_notes_pages_match_full_walk() {
    let dir = tempfile::tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let total = 23usize;
    let mut created = Vec::new();
    for i in 0..total {
        created.push(
            backend
                .create_note(Note::new(format!("t{i}"), "b"))
                .await
                .unwrap(),
        );
    }
    backend.delete_note(created[5].id).await.unwrap();
    created.remove(5);
    created.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

    let mut walked = Vec::new();
    let mut token = None;
    loop {
        let (page, next) = backend.list_notes(7, token).await.unwrap();
        assert!(page.len() <= 7);
        walked.extend(page);
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    assert_eq!(
        walked.iter().map(|n| n.id).collect::<Vec<_>>(),
        created.iter().map(|n| n.id).collect::<Vec<_>>(),
        "paged walk must reproduce the full (created_at, id) order"
    );
}

// md:fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones
#[tokio::test]
async fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones() {
    let dir = tempfile::tempdir().unwrap();
    let note_id = {
        let be = FsBackend::new(dir.path()).await.unwrap();
        be.create_note(Note::new("t", "b")).await.unwrap().id
    };
    let planted = [
        dir.path()
            .join("notes")
            .join(note_id.to_string())
            .join("meta.tmp"),
        dir.path().join("notebooks").join("junk.tmp"),
        dir.path().join(".keeplin").join("sync_state.tmp"),
        dir.path().join(".keeplin").join("offsets").join("dev.tmp"),
    ];
    for p in &planted {
        std::fs::write(p, b"junk").unwrap();
    }
    let syncthing = dir
        .path()
        .join("notebooks")
        .join(".syncthing.abc.ndjson.tmp");
    std::fs::write(&syncthing, b"in-flight transfer").unwrap();

    let be = FsBackend::new(dir.path()).await.unwrap();
    for p in &planted {
        assert!(!p.exists(), "must be swept: {}", p.display());
    }
    assert!(syncthing.exists(), "Syncthing temp must be left alone");
    assert_eq!(be.read_note(note_id).await.unwrap().body, "b");
}

// md:fn failed_atomic_write_cleans_up_its_temp_file
#[tokio::test]
async fn failed_atomic_write_cleans_up_its_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("blocked");
    std::fs::create_dir(&dest).unwrap();

    assert!(atomic_write(&dest, b"payload").await.is_err());
    assert!(
        !dest.with_extension("tmp").exists(),
        "temp file must be removed after a failed write"
    );
    assert!(dest.is_dir(), "destination must be untouched");
}

// md:fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it
#[tokio::test]
async fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it() {
    let dir = tempfile::tempdir().unwrap();
    let be = FsBackend::new(dir.path()).await.unwrap();
    let note = be.create_note(Note::new("n", "")).await.unwrap();
    let tag = be.create_tag(Tag::new("t")).await.unwrap();
    be.add_note_tag(NoteTag {
        note_id: note.id,
        tag_id: tag.id,
    })
    .await
    .unwrap();

    let path = dir
        .path()
        .join("note_tags")
        .join(note.id.to_string())
        .join(tag.id.to_string());
    std::fs::write(&path, b"not ndjson at all").unwrap();

    let (tags, _) = be.list_note_tags(note.id, 0, None).await.unwrap();
    assert_eq!(tags.len(), 1, "corrupt state must not hide the association");

    let mut vv = VersionVector::new();
    note_log::increment(&mut vv, "peer");
    be.apply_change(Change::NoteTagRemove {
        note_id: note.id,
        tag_id: tag.id,
        updated_at: now(),
        vv,
        last_writer: "peer".to_string(),
    })
    .await
    .unwrap();
    let (tags, _) = be.list_note_tags(note.id, 0, None).await.unwrap();
    assert!(
        tags.is_empty(),
        "a versioned remove must supersede the corrupt marker"
    );
}

// md:fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair
#[tokio::test]
async fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair() {
    let dir = tempfile::tempdir().unwrap();
    let be = FsBackend::new(dir.path()).await.unwrap();
    let nb = be.create_notebook(Notebook::new("kept")).await.unwrap();
    be.create_tag(Tag::new("t")).await.unwrap();
    let log_path = be.device_log_path();
    let before = tokio::fs::read_to_string(&log_path).await.unwrap();

    let sidecar = be.notebook_path(nb.id);
    std::fs::write(&sidecar, b"definitely not ndjson").unwrap();
    {
        let _guard = be.global_log_lock.lock().await;
        be.compact_global_log_locked().await.unwrap();
    }
    let after = tokio::fs::read_to_string(&log_path).await.unwrap();
    assert_eq!(before, after, "journal must not be rewritten while corrupt");
    assert_eq!(
        be.read_own_epoch().await.unwrap(),
        0,
        "no snapshot generation was produced"
    );

    let mut bytes = serde_json::to_vec(&nb).unwrap();
    bytes.push(b'\n');
    std::fs::write(&sidecar, bytes).unwrap();
    {
        let _guard = be.global_log_lock.lock().await;
        be.compact_global_log_locked().await.unwrap();
    }
    assert_eq!(
        be.read_own_epoch().await.unwrap(),
        1,
        "compaction resumes once the sidecar is readable"
    );
    let snapshot = tokio::fs::read_to_string(&log_path).await.unwrap();
    assert!(
        snapshot.contains(&nb.id.to_string()),
        "the repaired notebook is present in the snapshot"
    );
}

// md:fn detects_syncthing_conflict_copies_without_removing_them
#[tokio::test]
async fn detects_syncthing_conflict_copies_without_removing_them() {
    let dir = tempfile::tempdir().unwrap();
    let note_id = {
        let be = FsBackend::new(dir.path()).await.unwrap();
        be.create_note(Note::new("t", "b")).await.unwrap().id
    };
    let conflicts = [
        dir.path()
            .join(".keeplin")
            .join("device_id.sync-conflict-20260702-120000-AAAAAAA"),
        dir.path()
            .join("notebooks")
            .join("junk.sync-conflict-20260702-120000-BBBBBBB.ndjson"),
        dir.path()
            .join("notes")
            .join(note_id.to_string())
            .join("log.dev.sync-conflict-20260702-120000-CCCCCCC.ndjson"),
    ];
    for p in &conflicts {
        std::fs::write(p, b"conflict copy").unwrap();
    }

    let found = FsBackend::scan_sync_conflicts(dir.path()).await;
    assert_eq!(
        found.len(),
        conflicts.len(),
        "all copies detected: {found:?}"
    );

    let be = FsBackend::new(dir.path()).await.unwrap();
    for p in &conflicts {
        assert!(
            p.exists(),
            "conflict copy must be preserved: {}",
            p.display()
        );
    }
    assert_eq!(be.read_note(note_id).await.unwrap().body, "b");
}

// md:fn purge_reclaims_old_tombstoned_payloads_only
#[tokio::test]
async fn purge_reclaims_old_tombstoned_payloads_only() {
    let dir = tempfile::tempdir().unwrap();
    let be = FsBackend::new(dir.path()).await.unwrap();

    let dead = Resource::new(SYSTEM_RESOURCE_NOTE_ID, "dead", "text/plain", "d.txt", 4);
    let dead_id = dead.id;
    be.create_resource(dead, b"dead".to_vec()).await.unwrap();
    be.delete_resource(dead_id).await.unwrap();

    let live = Resource::new(SYSTEM_RESOURCE_NOTE_ID, "live", "text/plain", "l.txt", 4);
    let live_id = live.id;
    be.create_resource(live, b"live".to_vec()).await.unwrap();

    let epoch = DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    assert_eq!(be.purge_deleted_resources(epoch).await.unwrap(), 0);
    let dead_blob = be.resource_blob_path(SYSTEM_RESOURCE_NOTE_ID, &content_hash(b"dead"));
    assert!(dead_blob.exists());

    assert_eq!(be.purge_deleted_resources(now()).await.unwrap(), 1);
    assert!(!dead_blob.exists(), "dead bytes freed");
    assert!(
        be.resource_meta_path(SYSTEM_RESOURCE_NOTE_ID, dead_id)
            .exists(),
        "tombstone metadata must survive the purge"
    );
    assert!(matches!(
        be.read_resource(dead_id).await,
        Err(StorageError::NotFound(_))
    ));
    let (_, bytes) = be.read_resource(live_id).await.unwrap();
    assert_eq!(bytes, b"live", "live resources are untouched");

    assert_eq!(be.purge_deleted_resources(now()).await.unwrap(), 0);
    let mut revived = Resource::new(SYSTEM_RESOURCE_NOTE_ID, "revived", "text/plain", "r.txt", 3);
    revived.id = dead_id;
    be.create_resource(revived, b"new".to_vec()).await.unwrap();
    let (_, bytes) = be.read_resource(dead_id).await.unwrap();
    assert_eq!(bytes, b"new");
}

// md:fn attachments_live_as_content_hashed_knrs_in_their_note_folder
#[tokio::test]
async fn attachments_live_as_content_hashed_knrs_in_their_note_folder() {
    let dir = tempfile::tempdir().unwrap();
    let be = FsBackend::new(dir.path()).await.unwrap();
    let note = be.create_note(Note::new("host", "body")).await.unwrap();

    let payload = b"\x89PNG\r\n original picture bytes".to_vec();
    let resource = Resource::new(note.id, "pic", "image/png", "pic.png", payload.len() as u64);
    let id = resource.id;
    be.create_resource(resource, payload.clone()).await.unwrap();

    let hash = content_hash(&payload);
    let note_res = dir
        .path()
        .join("notes")
        .join(note.id.to_string())
        .join("resources");
    let blob = note_res.join(format!("{hash}.knrs"));
    assert_eq!(
        tokio::fs::read(&blob).await.unwrap(),
        payload,
        "the .knrs blob is the original bytes verbatim"
    );
    assert!(
        note_res.join(format!("{id}.meta.ndjson")).exists(),
        "metadata sidecar sits beside the blob"
    );
    assert!(
        !dir.path().join("resources").exists(),
        "no global resource pool is created"
    );

    let (loaded, bytes) = be.read_resource(id).await.unwrap();
    assert_eq!(loaded.note_id, note.id);
    assert_eq!(bytes, payload);
}

// md:fn identical_attachments_in_a_note_share_one_blob
#[tokio::test]
async fn identical_attachments_in_a_note_share_one_blob() {
    let dir = tempfile::tempdir().unwrap();
    let be = FsBackend::new(dir.path()).await.unwrap();
    let note = be.create_note(Note::new("host", "body")).await.unwrap();

    let payload = b"shared attachment payload".to_vec();
    let first = Resource::new(
        note.id,
        "one",
        "text/plain",
        "one.txt",
        payload.len() as u64,
    );
    let first_id = first.id;
    be.create_resource(first, payload.clone()).await.unwrap();
    let second = Resource::new(
        note.id,
        "two",
        "text/plain",
        "two.txt",
        payload.len() as u64,
    );
    let second_id = second.id;
    be.create_resource(second, payload.clone()).await.unwrap();

    let note_res = dir
        .path()
        .join("notes")
        .join(note.id.to_string())
        .join("resources");
    let mut blobs = 0usize;
    let mut rd = tokio::fs::read_dir(&note_res).await.unwrap();
    while let Some(entry) = rd.next_entry().await.unwrap() {
        if entry.file_name().to_string_lossy().ends_with(".knrs") {
            blobs += 1;
        }
    }
    assert_eq!(blobs, 1, "identical content deduplicates to a single blob");

    be.delete_resource(first_id).await.unwrap();
    assert_eq!(
        be.purge_deleted_resources(now()).await.unwrap(),
        0,
        "the shared blob is retained while a live sibling references it"
    );
    let (_, bytes) = be.read_resource(second_id).await.unwrap();
    assert_eq!(
        bytes, payload,
        "the surviving attachment still reads its bytes"
    );

    be.delete_resource(second_id).await.unwrap();
    assert_eq!(
        be.purge_deleted_resources(now()).await.unwrap(),
        1,
        "with no live reference the shared blob is finally reclaimed"
    );
}

// md:fn fresh_store_is_stamped_current_version
#[tokio::test]
async fn fresh_store_is_stamped_current_version() {
    let dir = tempfile::tempdir().unwrap();
    let be = FsBackend::new(dir.path()).await.unwrap();
    let stamp = tokio::fs::read_to_string(be.format_version_path())
        .await
        .unwrap();
    assert_eq!(
        stamp.trim().parse::<u32>().unwrap(),
        FsBackend::FORMAT_VERSION,
        "a brand-new store starts stamped at the current format version"
    );
}

// md:fn migrates_a_legacy_stamp_and_preserves_data
#[tokio::test]
async fn migrates_a_legacy_stamp_and_preserves_data() {
    let dir = tempfile::tempdir().unwrap();

    let note_id = {
        let be = FsBackend::new(dir.path()).await.unwrap();
        let note = be.create_note(Note::new("legacy", "kept")).await.unwrap();
        tokio::fs::write(be.format_version_path(), "1")
            .await
            .unwrap();
        note.id
    };

    let be = FsBackend::new(dir.path()).await.unwrap();
    let stamp = tokio::fs::read_to_string(be.format_version_path())
        .await
        .unwrap();
    assert_eq!(
        stamp.trim().parse::<u32>().unwrap(),
        FsBackend::FORMAT_VERSION
    );
    assert_eq!(be.read_note(note_id).await.unwrap().body, "kept");
}

// md:fn refuses_to_open_a_newer_format
#[tokio::test]
async fn refuses_to_open_a_newer_format() {
    let dir = tempfile::tempdir().unwrap();
    {
        let be = FsBackend::new(dir.path()).await.unwrap();
        let future = (FsBackend::FORMAT_VERSION + 1).to_string();
        tokio::fs::write(be.format_version_path(), future)
            .await
            .unwrap();
    }
    let err = match FsBackend::new(dir.path()).await {
        Ok(_) => panic!("opening a newer on-disk format must be refused"),
        Err(e) => e,
    };
    assert!(
        matches!(err, StorageError::InvalidState(ref m) if m.contains("newer than this build")),
        "got: {err:?}"
    );
}
