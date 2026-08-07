# `storage/fs/tests.rs` — filesystem backend regression tests

Self-contained companion for `keeplin-core/src/storage/fs/tests.rs`. It documents every test block in source order with complete code embedded.

---

## Overview

**Identification** — test imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Imports the filesystem backend, its focused helpers, domain models, repository traits, and test utilities used by the regression suite.

**Dependencies** — every binding above is used by at least one test below; expects: the split keeps sibling-only helpers reachable through `super` and the public repository behavior unchanged.

**Used by** — every test in this file.

**Repeated context** — tests moved out of the former inline `mod tests`; assertions and fixtures are unchanged.

---

## fn concurrent_same_note_updates_keep_every_log_entry

**Identification** — multi-thread tokio test; marker
`// md:fn concurrent_same_note_updates_keep_every_log_entry`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Regression for the lost-update race: 20 concurrent updates
to one note all land in the single-device log (create + 20 entries, none
dropped by a racing rename) — the `note_write_lock` guarantee.

---

## fn read_does_not_rewrite_projection

**Identification** — tokio test; marker
`// md:fn read_does_not_rewrite_projection`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Delete the projection files; `read_note` still answers
from the logs and does **not** recreate `note.md`/`meta.ndjson` (reads are
pure).

---

## fn list_notes_pages_match_full_walk

**Identification** — tokio test; marker
`// md:fn list_notes_pages_match_full_walk`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — 23 notes (one deleted) walked in pages of 7 reproduce the
full `(created_at, id)` order — the heap `PageCollector` paginates exactly
like sort-then-`paginate`.

---

## fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones

**Identification** — tokio test; marker
`// md:fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Planted `*.tmp` files in every managed dir are swept on
startup; a `.syncthing.*.tmp` survives; the store still reads.

---

## fn failed_atomic_write_cleans_up_its_temp_file

**Identification** — tokio test; marker
`// md:fn failed_atomic_write_cleans_up_its_temp_file`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A rename-blocked `atomic_write` errors, removes its temp
file, and leaves the destination untouched.

---

## fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it

**Identification** — tokio test; marker
`// md:fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A corrupted association file still lists as attached
(least harm), and a versioned peer remove supersedes the epoch-0 fallback
marker.

---

## fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair

**Identification** — tokio test; marker
`// md:fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — With a corrupted notebook sidecar the journal is not
rewritten and no epoch is produced; after repair, compaction produces epoch 1
containing the notebook.

---

## fn detects_syncthing_conflict_copies_without_removing_them

**Identification** — tokio test; marker
`// md:fn detects_syncthing_conflict_copies_without_removing_them`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Conflict copies in `.keeplin/`, `notebooks/`, and a note
dir are all detected, never deleted, and never block startup.

---

## fn purge_reclaims_old_tombstoned_payloads_only

**Identification** — tokio test; marker
`// md:fn purge_reclaims_old_tombstoned_payloads_only`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A pre-tombstone cutoff purges nothing; a later cutoff frees
exactly the dead `{hash}.knrs` blob while the tombstone sidecar survives and live
resources are untouched; purge is idempotent and the id can be recreated with new
content (a fresh hash, hence a fresh blob).

---

## fn attachments_live_as_content_hashed_knrs_in_their_note_folder

**Identification** — tokio test; marker
`// md:fn attachments_live_as_content_hashed_knrs_in_their_note_folder`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The layout contract (issue #127): a created attachment lands as
`notes/{note}/resources/{hash}.knrs` (bytes identical to the original) beside its
`{id}.meta.ndjson` sidecar, and **no** global `resources/` pool is created; the
attachment then reads back through `read_resource` with its `note_id` and bytes
intact.

---

## fn identical_attachments_in_a_note_share_one_blob

**Identification** — tokio test; marker
`// md:fn identical_attachments_in_a_note_share_one_blob`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Content dedup + reference-counted purge (issue #127): two live
resources with identical bytes in one note share a single `{hash}.knrs`; deleting
and purging one frees nothing while the other is live (the surviving attachment
still reads), and only once no live resource references the hash does the purge
reclaim the blob.

---

## fn fresh_store_is_stamped_current_version

**Identification** — tokio test; marker
`// md:fn fresh_store_is_stamped_current_version`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A brand-new store starts stamped `FORMAT_VERSION`.

---

## fn current_store_opens_without_rewriting_stamp

**Identification** — tokio test; marker
`// md:fn current_store_opens_without_rewriting_stamp`.

**Code** — complete and verbatim:

```rust
// md:fn current_store_opens_without_rewriting_stamp
#[tokio::test]
async fn current_store_opens_without_rewriting_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let stamp_path = {
        let backend = FsBackend::new(dir.path()).await.unwrap();
        backend.format_version_path()
    };
    let current_with_newline = format!("{}\n", FsBackend::FORMAT_VERSION);
    tokio::fs::write(&stamp_path, current_with_newline.as_bytes())
        .await
        .unwrap();

    FsBackend::new(dir.path()).await.unwrap();

    assert_eq!(
        tokio::fs::read(&stamp_path).await.unwrap(),
        current_with_newline.as_bytes()
    );
}
```

**What it does** — Reopens a current-format store and proves the accepted stamp
is not normalized or rewritten as a side effect.

---

## fn refuses_missing_format_stamp_without_creating_one

**Identification** — tokio test; marker
`// md:fn refuses_missing_format_stamp_without_creating_one`.

**Code** — complete and verbatim:

```rust
// md:fn refuses_missing_format_stamp_without_creating_one
#[tokio::test]
async fn refuses_missing_format_stamp_without_creating_one() {
    let dir = tempfile::tempdir().unwrap();
    let metadata_dir = dir.path().join(".keeplin");
    let stamp_path = metadata_dir.join("format_version");
    tokio::fs::create_dir_all(&metadata_dir).await.unwrap();
    tokio::fs::write(metadata_dir.join("device_id"), "existing-device")
        .await
        .unwrap();
    tokio::fs::create_dir_all(dir.path().join("notes/existing-note"))
        .await
        .unwrap();
    tokio::fs::write(
        dir.path().join("notes/existing-note/note.md"),
        "existing note",
    )
    .await
    .unwrap();

    let err = match FsBackend::new(dir.path()).await {
        Ok(_) => panic!("opening an existing store with a missing format stamp must be refused"),
        Err(err) => err,
    };
    let message = err.to_string().to_lowercase();
    assert!(
        message.contains("missing"),
        "error must name missing stamp: {message}"
    );
    assert!(
        message.contains(&FsBackend::FORMAT_VERSION.to_string()),
        "error must name expected version: {message}"
    );
    assert!(message.contains("manual recovery"), "{message}");
    assert!(message.contains("new store"), "{message}");
    assert!(message.contains("backup"), "{message}");
    assert!(!stamp_path.exists());
    assert_eq!(
        tokio::fs::read(metadata_dir.join("device_id"))
            .await
            .unwrap(),
        b"existing-device"
    );
}
```

**What it does** — Treats a device-bearing store without a format stamp as
existing legacy data, refuses it with every required recovery choice, and
proves no stamp is invented.

---

## fn refuses_pre_v8_store_without_touching_legacy_attachment

**Identification** — tokio test; marker
`// md:fn refuses_pre_v8_store_without_touching_legacy_attachment`.

**Code** — complete and verbatim:

```rust
// md:fn refuses_pre_v8_store_without_touching_legacy_attachment
#[tokio::test]
async fn refuses_pre_v8_store_without_touching_legacy_attachment() {
    let dir = tempfile::tempdir().unwrap();
    let note = Note::new("legacy", "kept");
    let attachment = b"pre-v8 attachment bytes";
    let resource = Resource::new(
        note.id,
        "legacy attachment",
        "application/octet-stream",
        "legacy.bin",
        attachment.len() as u64,
    );
    let stamp_path = dir.path().join(".keeplin").join("format_version");
    let note_dir = dir.path().join("notes").join(note.id.to_string());
    let resource_dir = dir.path().join("resources").join(resource.id.to_string());
    let attachment_path = resource_dir.join("data");
    let orphan_path = note_dir.join("preserved.tmp");

    tokio::fs::create_dir_all(stamp_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(
        stamp_path.parent().unwrap().join("device_id"),
        "legacy-device",
    )
    .await
    .unwrap();
    tokio::fs::create_dir_all(&note_dir).await.unwrap();
    tokio::fs::create_dir_all(&resource_dir).await.unwrap();
    tokio::fs::write(&stamp_path, b"7").await.unwrap();
    tokio::fs::write(note_dir.join("note.md"), note.body.as_bytes())
        .await
        .unwrap();
    let mut projected_note = note.clone();
    projected_note.body.clear();
    let mut note_meta = serde_json::to_vec(&serde_json::json!({
        "note": projected_note,
        "vv": VersionVector::new(),
    }))
    .unwrap();
    note_meta.push(b'\n');
    tokio::fs::write(note_dir.join("meta.ndjson"), &note_meta)
        .await
        .unwrap();
    let mut resource_meta = serde_json::to_vec(&resource).unwrap();
    resource_meta.push(b'\n');
    tokio::fs::write(resource_dir.join("meta.ndjson"), &resource_meta)
        .await
        .unwrap();
    tokio::fs::write(&attachment_path, attachment)
        .await
        .unwrap();
    tokio::fs::write(&orphan_path, b"pre-v8 tmp bytes")
        .await
        .unwrap();

    let err = match FsBackend::new(dir.path()).await {
        Ok(_) => panic!("opening a format 7 store must be refused"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(
        message.contains('7'),
        "error must name version 7: {message}"
    );
    assert!(
        message.contains('8'),
        "error must name version 8: {message}"
    );
    assert!(message.contains("manual recovery"), "{message}");
    assert!(message.contains("new store"), "{message}");
    assert!(message.contains("backup"), "{message}");
    assert_eq!(tokio::fs::read(&stamp_path).await.unwrap(), b"7");
    assert_eq!(
        tokio::fs::read(stamp_path.parent().unwrap().join("device_id"))
            .await
            .unwrap(),
        b"legacy-device"
    );
    assert_eq!(
        tokio::fs::read(note_dir.join("note.md")).await.unwrap(),
        note.body.as_bytes()
    );
    assert_eq!(
        tokio::fs::read(note_dir.join("meta.ndjson")).await.unwrap(),
        note_meta
    );
    assert_eq!(
        tokio::fs::read(resource_dir.join("meta.ndjson"))
            .await
            .unwrap(),
        resource_meta
    );
    assert_eq!(tokio::fs::read(&attachment_path).await.unwrap(), attachment);
    assert_eq!(
        tokio::fs::read(&orphan_path).await.unwrap(),
        b"pre-v8 tmp bytes"
    );
}
```

**What it does** — Builds a real v7 predecessor tree directly: a coherent note
projection and a serialized resource sidecar live beside attachment bytes in
the old global `resources/{id}/` pool. Opening must refuse versions 7 versus 8
and leave the stamp, device id, note body, both metadata sidecars, attachment,
and a sweep-shaped `*.tmp` sentinel byte-for-byte unchanged.

---

## fn refuses_unparsable_format_stamp_without_relabelling_it

**Identification** — tokio test; marker
`// md:fn refuses_unparsable_format_stamp_without_relabelling_it`.

**Code** — complete and verbatim:

```rust
// md:fn refuses_unparsable_format_stamp_without_relabelling_it
#[tokio::test]
async fn refuses_unparsable_format_stamp_without_relabelling_it() {
    let dir = tempfile::tempdir().unwrap();
    let stamp_path = dir.path().join(".keeplin").join("format_version");
    tokio::fs::create_dir_all(stamp_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(
        stamp_path.parent().unwrap().join("device_id"),
        "existing-device",
    )
    .await
    .unwrap();
    tokio::fs::write(&stamp_path, b"not-a-version")
        .await
        .unwrap();

    let err = match FsBackend::new(dir.path()).await {
        Ok(_) => panic!("opening a store with an unparsable format stamp must be refused"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(
        message.to_lowercase().contains("unparsable"),
        "error must name the unparsable stamp: {message}"
    );
    assert!(
        message.contains('8'),
        "error must name version 8: {message}"
    );
    assert_eq!(
        tokio::fs::read(&stamp_path).await.unwrap(),
        b"not-a-version"
    );
    assert_eq!(
        tokio::fs::read(stamp_path.parent().unwrap().join("device_id"))
            .await
            .unwrap(),
        b"existing-device"
    );
}
```

**What it does** — An unparsable format stamp must produce an explicit error
that names the parsing failure and expected version 8, without rewriting the
stamp as an invented legacy version or changing the existing device id.

---

## fn refuses_unstamped_store_content_without_device_id

**Identification** — tokio test; marker
`// md:fn refuses_unstamped_store_content_without_device_id`.

**Code** — complete and verbatim:

```rust
// md:fn refuses_unstamped_store_content_without_device_id
#[tokio::test]
async fn refuses_unstamped_store_content_without_device_id() {
    let dir = tempfile::tempdir().unwrap();
    let note_path = dir.path().join("notes/note-1/note.md");
    let attachment_path = dir.path().join("resources/resource-1/data");
    tokio::fs::create_dir_all(note_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::create_dir_all(attachment_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&note_path, b"existing note")
        .await
        .unwrap();
    tokio::fs::write(&attachment_path, b"existing attachment")
        .await
        .unwrap();

    let err = match FsBackend::new(dir.path()).await {
        Ok(_) => panic!("opening unstamped store content without a device id must be refused"),
        Err(err) => err,
    };
    let message = err.to_string().to_lowercase();
    assert!(message.contains("missing"), "{message}");
    assert!(!dir.path().join(".keeplin/format_version").exists());
    assert!(!dir.path().join(".keeplin/device_id").exists());
    assert_eq!(tokio::fs::read(&note_path).await.unwrap(), b"existing note");
    assert_eq!(
        tokio::fs::read(&attachment_path).await.unwrap(),
        b"existing attachment"
    );
}
```

**What it does** — Builds a store containing a note and legacy attachment but
neither a format stamp nor a device id. Opening must report the missing stamp,
must not stamp the content as current, and must preserve both payloads.

**Dependencies** —

- `FsBackend::new` — exercises startup classification; expects store content, rather than device-id presence, to prevent fresh-store stamping.
- `tokio::fs` — creates and verifies the fixture; expects reads and writes to preserve exact payload bytes.

**Used by** — regression coverage for ADR 0016's no-silent-relabel invariant.

**Repeated context** — Device identity and filesystem-format identity are independent.

---

## fn refuses_unstamped_sync_state_without_device_id

**Identification** — tokio test; marker
`// md:fn refuses_unstamped_sync_state_without_device_id`.

**Code** — complete and verbatim:

```rust
// md:fn refuses_unstamped_sync_state_without_device_id
#[tokio::test]
async fn refuses_unstamped_sync_state_without_device_id() {
    let dir = tempfile::tempdir().unwrap();
    let metadata_dir = dir.path().join(".keeplin");
    let sync_state_path = metadata_dir.join("sync_state.ndjson");
    tokio::fs::create_dir_all(&metadata_dir).await.unwrap();
    tokio::fs::write(&sync_state_path, b"existing sync state")
        .await
        .unwrap();

    let err = match FsBackend::new(dir.path()).await {
        Ok(_) => panic!("opening unstamped sync state without a device id must be refused"),
        Err(err) => err,
    };
    let message = err.to_string().to_lowercase();
    assert!(message.contains("missing"), "{message}");
    assert!(!metadata_dir.join("format_version").exists());
    assert!(!metadata_dir.join("device_id").exists());
    assert_eq!(
        tokio::fs::read(&sync_state_path).await.unwrap(),
        b"existing sync state"
    );
}
```

**What it does** — Proves the sync-state sidecar alone makes an unstamped store non-fresh. Startup
must refuse it before creating a device id and must preserve its bytes.

**Dependencies** —

- `FsBackend::new` — exercises exact-file freshness detection; expects `.keeplin/sync_state.ndjson` to count without making a device id count.
- `tokio::fs` — creates and verifies the fixture; expects byte-exact reads and writes.

**Used by** — regression coverage for historical/current writer-path completeness.

**Repeated context** — Device identity and filesystem-format identity are independent.

---

## fn refuses_to_open_a_newer_format

**Identification** — tokio test; marker
`// md:fn refuses_to_open_a_newer_format`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A stamp of `FORMAT_VERSION + 1` is refused with the
"newer than this build" `InvalidState`.

---

## Graph context

Repo-tooling metadata, not a code block.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- filesystem backend regression tests — exercise the split modules through unchanged behavior (INFERRED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/fs/` — implementation under test (INFERRED)

**Direct dependents** (files whose symbols reference this one)

- none; test-only module.

**Invariants** (the rules this file must keep true)

- Assertions and fixture operations remain identical to the pre-split inline tests.
- `FsBackend::FORMAT_VERSION` remains 8.

---

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|

| 1 | `Overview` | `// md:Overview` |
| 2 | `fn concurrent_same_note_updates_keep_every_log_entry` | `// md:fn concurrent_same_note_updates_keep_every_log_entry` |
| 3 | `fn read_does_not_rewrite_projection` | `// md:fn read_does_not_rewrite_projection` |
| 4 | `fn list_notes_pages_match_full_walk` | `// md:fn list_notes_pages_match_full_walk` |
| 5 | `fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones` | `// md:fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones` |
| 6 | `fn failed_atomic_write_cleans_up_its_temp_file` | `// md:fn failed_atomic_write_cleans_up_its_temp_file` |
| 7 | `fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it` | `// md:fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it` |
| 8 | `fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair` | `// md:fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair` |
| 9 | `fn detects_syncthing_conflict_copies_without_removing_them` | `// md:fn detects_syncthing_conflict_copies_without_removing_them` |
| 10 | `fn purge_reclaims_old_tombstoned_payloads_only` | `// md:fn purge_reclaims_old_tombstoned_payloads_only` |
| 11 | `fn attachments_live_as_content_hashed_knrs_in_their_note_folder` | `// md:fn attachments_live_as_content_hashed_knrs_in_their_note_folder` |
| 12 | `fn identical_attachments_in_a_note_share_one_blob` | `// md:fn identical_attachments_in_a_note_share_one_blob` |
| 13 | `fn fresh_store_is_stamped_current_version` | `// md:fn fresh_store_is_stamped_current_version` |
| 14 | `fn current_store_opens_without_rewriting_stamp` | `// md:fn current_store_opens_without_rewriting_stamp` |
| 15 | `fn refuses_missing_format_stamp_without_creating_one` | `// md:fn refuses_missing_format_stamp_without_creating_one` |
| 16 | `fn refuses_pre_v8_store_without_touching_legacy_attachment` | `// md:fn refuses_pre_v8_store_without_touching_legacy_attachment` |
| 17 | `fn refuses_unparsable_format_stamp_without_relabelling_it` | `// md:fn refuses_unparsable_format_stamp_without_relabelling_it` |
| 18 | `fn refuses_unstamped_store_content_without_device_id` | `// md:fn refuses_unstamped_store_content_without_device_id` |
| 19 | `fn refuses_unstamped_sync_state_without_device_id` | `// md:fn refuses_unstamped_sync_state_without_device_id` |
| 20 | `fn refuses_to_open_a_newer_format` | `// md:fn refuses_to_open_a_newer_format` |
