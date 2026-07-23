# `tests/fs_backend.rs` — FsBackend integration tests

Self-contained companion for `keeplin-core/tests/fs_backend.rs`. It documents
**every code block of the source file, in source order** — a reader with only this
file must be able to understand it without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Block header>`; grep it in either direction. Each section covers
**Identification**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the crate doc and the imports. Marker
`// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview

use chrono::Utc;
use keeplin_core::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag, SYSTEM_RESOURCE_NOTE_ID},
    storage::{
        fs::FsBackend, NoteRepository, NotebookRepository, ResourceRepository, SyncBackend,
        TagRepository,
    },
};
use tempfile::tempdir;
```

**What it does** — Integration tests for `FsBackend`, the filesystem-backed
storage implementation. Every test creates a fresh tempdir, roots a new
`FsBackend` there, and exercises the full `StorageBackend` API against real
files on disk: CRUD happy paths and error paths (`NotFound`), soft-deletion,
device-id persistence, keyset pagination, the **version-vector note model**
(three-file layout, per-device note logs, Syncthing-style log replication,
causal and concurrent convergence, log compaction), the **global NDJSON
journal** with generation-epoch snapshot compaction, the in-memory note index,
ordering/starring fields, and tombstone-first races (#71).

**Repeated context** — FsBackend layout: each note lives in `notes/{id}/` as
`note.md` (body), `meta.ndjson` (metadata projection), and one
`log.{device}.ndjson` per device (the **single-writer source of truth**);
global entities journal to `logs/{device}.log` (NDJSON). Replication copies
**only** the single-writer logs — projections are per-device caches regenerated
on sync. Conflicts resolve through `note_log::resolve` with the
`(timestamp, device_id)` tiebreak.

---

## fn create_and_read_note

**Identification** — `#[tokio::test]`. Marker `// md:fn create_and_read_note`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Note round-trip: create returns the same id; read returns
title and body.

---

## fn update_note

**Identification** — `#[tokio::test]`. Marker `// md:fn update_note`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Update returns and persists the new title.

---

## fn delete_note_soft_deletes

**Identification** — `#[tokio::test]`. Marker `// md:fn delete_note_soft_deletes`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A deleted note disappears from `list_notes`.

---

## fn list_notes_excludes_deleted

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_notes_excludes_deleted`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Of two notes, deleting one leaves only the survivor listed.

---

## fn read_nonexistent_note_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn read_nonexistent_note_returns_not_found`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Reading a random UUID → `StorageError::NotFound`.

---

## fn device_id_is_stable_across_instances

**Identification** — `#[tokio::test]`. Marker
`// md:fn device_id_is_stable_across_instances`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Two `FsBackend`s over the same root return the same
persisted device id (`.keeplin/device_id`).

---

## fn sync_state_persists

**Identification** — `#[tokio::test]`. Marker `// md:fn sync_state_persists`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `update_sync_time`/`get_last_sync_time` round-trip. The
timestamp is serialised as RFC-3339, which may lose sub-second precision, so
the comparison is at second granularity.

---

## fn get_changes_since_scans_other_device_logs

**Identification** — `#[tokio::test]`. Marker
`// md:fn get_changes_since_scans_other_device_logs`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Simulates a log file written by a **different** device and
replicated into `logs/` by Syncthing (its name differs from this device's own
log, so it is not skipped): `get_changes_since` parses it and yields the
`Change::NoteCreate`.

---

## fn update_nonexistent_note_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn update_nonexistent_note_returns_not_found`.

**Code** — complete and verbatim:

```rust
// md:fn update_nonexistent_note_returns_not_found
#[tokio::test]
async fn update_nonexistent_note_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let note = Note::new("Ghost", "");
    let err = backend.update_note(note).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}
```

**What it does** — Error path: update of an unknown note → `NotFound`.

---

## fn delete_nonexistent_note_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn delete_nonexistent_note_returns_not_found`.

**Code** — complete and verbatim:

```rust
// md:fn delete_nonexistent_note_returns_not_found
#[tokio::test]
async fn delete_nonexistent_note_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let id = uuid::Uuid::new_v4();
    let err = backend.delete_note(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}
```

**What it does** — Delete of an unknown note → `NotFound`.

---

## fn update_nonexistent_notebook_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn update_nonexistent_notebook_returns_not_found`.

**Code** — complete and verbatim:

```rust
// md:fn update_nonexistent_notebook_returns_not_found
#[tokio::test]
async fn update_nonexistent_notebook_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let nb = Notebook::new("Ghost");
    let err = backend.update_notebook(nb).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}
```

**What it does** — Update of an unknown notebook → `NotFound`.

---

## fn delete_nonexistent_notebook_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn delete_nonexistent_notebook_returns_not_found`.

**Code** — complete and verbatim:

```rust
// md:fn delete_nonexistent_notebook_returns_not_found
#[tokio::test]
async fn delete_nonexistent_notebook_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let id = uuid::Uuid::new_v4();
    let err = backend.delete_notebook(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}
```

**What it does** — Delete of an unknown notebook → `NotFound`.

---

## fn update_nonexistent_tag_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn update_nonexistent_tag_returns_not_found`.

**Code** — complete and verbatim:

```rust
// md:fn update_nonexistent_tag_returns_not_found
#[tokio::test]
async fn update_nonexistent_tag_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let tag = Tag::new("ghost");
    let err = backend.update_tag(tag).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}
```

**What it does** — Update of an unknown tag → `NotFound`.

---

## fn delete_nonexistent_tag_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn delete_nonexistent_tag_returns_not_found`.

**Code** — complete and verbatim:

```rust
// md:fn delete_nonexistent_tag_returns_not_found
#[tokio::test]
async fn delete_nonexistent_tag_returns_not_found() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let id = uuid::Uuid::new_v4();
    let err = backend.delete_tag(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}
```

**What it does** — Delete of an unknown tag → `NotFound`.

---

## fn create_and_read_notebook

**Identification** — `#[tokio::test]`. Marker `// md:fn create_and_read_notebook`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Notebook round-trip; `deleted_at` starts `None`.

---

## fn list_notebooks_includes_created

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_notebooks_includes_created`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Regression: the sidecar is written as `{id}.ndjson`, so the
listing must filter on that extension — a previous `.json` filter matched
nothing and returned an empty list.

---

## fn delete_notebook_soft_deletes

**Identification** — `#[tokio::test]`. Marker
`// md:fn delete_notebook_soft_deletes`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A deleted notebook leaves the listing but a direct read
still returns the tombstone with `deleted_at` set.

---

## fn create_and_read_tag

**Identification** — `#[tokio::test]`. Marker `// md:fn create_and_read_tag`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Tag round-trip.

---

## fn list_tags_includes_created

**Identification** — `#[tokio::test]`. Marker `// md:fn list_tags_includes_created`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Regression: the same `.ndjson`-vs-`.json` listing bug as
notebooks, for tags.

---

## fn add_and_list_note_tags

**Identification** — `#[tokio::test]`. Marker `// md:fn add_and_list_note_tags`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Attach a tag; `list_note_tags` returns it.

---

## fn add_note_tag_rejects_missing_or_deleted_ends

**Identification** — `#[tokio::test]`. Marker
`// md:fn add_note_tag_rejects_missing_or_deleted_ends`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `add_note_tag` with a nonexistent note or tag, or a
soft-deleted tag, fails with `NotFound` — no dangling association is created.

---

## fn remove_note_tag

**Identification** — `#[tokio::test]`. Marker `// md:fn remove_note_tag`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Detach after attach; the listing is empty again.

---

## fn create_and_read_resource

**Identification** — `#[tokio::test]`. Marker `// md:fn create_and_read_resource`.

**Code** — complete and verbatim:

```rust
// md:fn create_and_read_resource
#[tokio::test]
async fn create_and_read_resource() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let data = b"hello world".to_vec();
    let res = Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        "attachment",
        "text/plain",
        "hello.txt",
        data.len() as u64,
    );
    let id = res.id;
    backend.create_resource(res, data.clone()).await.unwrap();

    let (meta, bytes) = backend.read_resource(id).await.unwrap();
    assert_eq!(meta.title, "attachment");
    assert_eq!(bytes, data);
}
```

**What it does** — Resource metadata + bytes round-trip.

---

## fn list_resources_excludes_data

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_resources_excludes_data`.

**Code** — complete and verbatim:

```rust
// md:fn list_resources_excludes_data
#[tokio::test]
async fn list_resources_excludes_data() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    for i in 0..3u8 {
        let data = vec![i];
        let res = Resource::new(
            SYSTEM_RESOURCE_NOTE_ID,
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
```

**What it does** — Three resources list as metadata only.

---

## fn delete_resource

**Identification** — `#[tokio::test]`. Marker `// md:fn delete_resource`.

**Code** — complete and verbatim:

```rust
// md:fn delete_resource
#[tokio::test]
async fn delete_resource() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let res = Resource::new(SYSTEM_RESOURCE_NOTE_ID, "doc", "text/plain", "doc.txt", 0);
    let id = res.id;
    backend.create_resource(res, vec![]).await.unwrap();
    backend.delete_resource(id).await.unwrap();

    let err = backend.read_resource(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}
```

**What it does** — A deleted resource reads back as `NotFound`.

---

## fn list_notes_paginates_without_duplicates_or_gaps

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_notes_paginates_without_duplicates_or_gaps`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — 23 notes walked with page size 7: no page exceeds the size,
every note appears exactly once, and the paged order equals the single-shot
order.

---

## fn replicate_note

**Identification** — `async fn replicate_note(from_root, to_root, id)`. Marker
`// md:fn replicate_note`.

**Code** — complete and verbatim:

```rust
// md:fn replicate_note
async fn replicate_note(from_root: &std::path::Path, to_root: &std::path::Path, id: uuid::Uuid) {
    let from = from_root.join("notes").join(id.to_string());
    let to = to_root.join("notes").join(id.to_string());
    tokio::fs::create_dir_all(&to).await.unwrap();
    let mut rd = tokio::fs::read_dir(&from).await.unwrap();
    while let Some(e) = rd.next_entry().await.unwrap() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("log.") && name.ends_with(".ndjson") {
            tokio::fs::copy(e.path(), to.join(&name)).await.unwrap();
        }
    }
}
```

**What it does** — Simulates Syncthing replicating one note between roots by
copying **only** its per-device `log.*.ndjson` files (the single-writer
source of truth) — never the local projections.

**Used by** — the two-device note tests and the note-log compaction test.

---

## fn fs_note_uses_three_file_layout

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_note_uses_three_file_layout`.

**Code** — complete and verbatim:

```rust
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
        ndir.join("meta.ndjson").exists(),
        "meta.ndjson must exist"
    );

    let mut found_log = false;
    for e in std::fs::read_dir(&ndir).unwrap() {
        let n = e.unwrap().file_name().to_string_lossy().into_owned();
        if n.starts_with("log.") && n.ends_with(".ndjson") {
            found_log = true;
        }
    }
    assert!(found_log, "a per-device log file must exist");

    let body = std::fs::read_to_string(ndir.join("note.md")).unwrap();
    assert_eq!(body, "# Markdown body");
}
```

**What it does** — After a create, `notes/{id}/` contains `note.md`,
`meta.ndjson`, and a per-device `log.*.ndjson`; the markdown body is stored
verbatim (unencrypted backend).

---

## fn fs_on_disk_serialization_is_ndjson

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_on_disk_serialization_is_ndjson`.

**Code** — complete and verbatim:

```rust
// md:fn fs_on_disk_serialization_is_ndjson
#[tokio::test]
async fn fs_on_disk_serialization_is_ndjson() {
    use keeplin_core::storage::note_log::NoteLogEntry;
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let note = Note::new("Title", "body v0");
    let id = note.id;
    backend.create_note(note.clone()).await.unwrap();
    for i in 1..=3 {
        let mut edited = note.clone();
        edited.body = format!("body v{i}");
        backend.update_note(edited).await.unwrap();
    }
    let notebook = backend.create_notebook(Notebook::new("nb")).await.unwrap();
    let tag = backend.create_tag(Tag::new("t")).await.unwrap();
    backend
        .create_resource(
            Resource::new(id, "r", "text/plain", "r.txt", 3),
            b"abc".to_vec(),
        )
        .await
        .unwrap();

    let ndir = dir.path().join("notes").join(id.to_string());
    let mut log_path = None;
    for e in std::fs::read_dir(&ndir).unwrap() {
        let n = e.unwrap().file_name().to_string_lossy().into_owned();
        if n.starts_with("log.") && n.ends_with(".ndjson") {
            log_path = Some(ndir.join(n));
        }
    }
    let log_bytes = std::fs::read(log_path.expect("per-device log")).unwrap();
    let lines: Vec<&[u8]> = log_bytes
        .split(|b| *b == b'\n')
        .filter(|l| !l.iter().all(u8::is_ascii_whitespace))
        .collect();
    assert!(
        lines.len() >= 4,
        "one NDJSON line per log entry, got {}",
        lines.len()
    );
    for line in &lines {
        serde_json::from_slice::<NoteLogEntry>(line).unwrap();
    }

    let meta_bytes = std::fs::read(ndir.join("meta.ndjson")).unwrap();
    assert_eq!(
        meta_bytes.iter().filter(|b| **b == b'\n').count(),
        1,
        "a single-entity sidecar is one JSON object on one line"
    );
    serde_json::from_slice::<serde_json::Value>(&meta_bytes).unwrap();

    for sidecar in [
        dir.path()
            .join("notebooks")
            .join(format!("{}.ndjson", notebook.id)),
        dir.path().join("tags").join(format!("{}.ndjson", tag.id)),
    ] {
        let bytes = std::fs::read(&sidecar).unwrap();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap();
    }

    let mut stack = vec![dir.path().to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                assert!(
                    !p.to_string_lossy().ends_with(".msgpack"),
                    "no msgpack files may remain: {}",
                    p.display()
                );
            }
        }
    }
}
```

**What it does** — Locks in the on-disk NDJSON format (issue #126): the
per-device note log is multi-line NDJSON (one JSON `NoteLogEntry` per line, ≥4
after create + 3 edits), each entity sidecar (`meta.ndjson`, `notebooks/{id}.ndjson`,
`tags/{id}.ndjson`) is a single JSON object on one line, and a recursive walk of
the store root asserts **no** `.msgpack` file survives anywhere.

**Used by** — the test harness (`cargo test`).

**Repeated context** — Single-entity sidecars are one NDJSON line; per-device
logs are one entry per line. MessagePack is fully retired from the fs backend.

---

## fn fs_two_device_causal_sync

**Identification** — `#[tokio::test]`. Marker `// md:fn fs_two_device_causal_sync`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A creates; the log replicates A→B and B reads the note. B
then edits **causally** (having seen A's version) and replicates back: the
causal edit wins on A with no conflict.

---

## fn fs_two_device_concurrent_edits_converge

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_two_device_concurrent_edits_converge`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Concurrent edits with no exchange between them, then
cross-replication of both logs: both devices converge to the **same** winner
(deterministic last-write-wins by timestamp, then device id). This is the
FsBackend counterpart of `sync.rs`'s equal-timestamp DbBackend test —
FsBackend resolves through per-note logs, not wire `Change` records.

---

## fn note_alias_bookmarks_links_persist_in_meta

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_alias_bookmarks_links_persist_in_meta`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Alias, bookmarks, and a manual link survive the round-trip
through `log.{device}.ndjson` + `meta.ndjson` (reads materialise from the
per-device log); a second backend over the same root (a different "device")
materialises the same state from the replicated log.

---

## fn backlinks_default_scan_is_paginated

**Identification** — `#[tokio::test]`. Marker
`// md:fn backlinks_default_scan_is_paginated`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Three backlinks walked with page size 2: pages of 2 and 1,
no third page, union covers all three sources without overlap.

---

## fn fs_notebook_concurrent_equal_timestamp_edits_converge

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_notebook_concurrent_equal_timestamp_edits_converge`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Two `FsBackend` devices edit the same notebook with the
**identical** `updated_at` (exchanged via `apply_change`): version-vector
`resolve` picks one deterministic winner on both sides. Under the old
`updated_at`-only `>` comparison, equal timestamps meant neither device applied
the other's edit — permanent divergence.

---

## fn fs_concurrent_note_tag_add_remove_converges

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_concurrent_note_tag_add_remove_converges`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Concurrent attach-vs-detach of a note↔tag association
converges on `FsBackend` exactly as on `DbBackend` (the FS mirror of
`sync::db_concurrent_note_tag_add_remove_converges`): after exchanging
replicated logs both ways and draining sync, both devices agree on the
association's final presence.

---

## fn note_log_len

**Identification** — `async fn note_log_len(root, id) -> usize`. Marker
`// md:fn note_log_len`.

**Code** — complete and verbatim:

```rust
// md:fn note_log_len
async fn note_log_len(root: &std::path::Path, id: uuid::Uuid) -> usize {
    use keeplin_core::storage::note_log::NoteLogEntry;
    let dir = root.join("notes").join(id.to_string());
    let mut rd = tokio::fs::read_dir(&dir).await.unwrap();
    while let Some(e) = rd.next_entry().await.unwrap() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("log.") && name.ends_with(".ndjson") {
            let bytes = tokio::fs::read(e.path()).await.unwrap();
            let entries = bytes
                .split(|b| *b == b'\n')
                .filter(|l| !l.iter().all(u8::is_ascii_whitespace))
                .map(|l| serde_json::from_slice::<NoteLogEntry>(l).unwrap())
                .collect::<Vec<_>>();
            return entries.len();
        }
    }
    panic!("no per-device note log found for {id}");
}
```

**What it does** — Counts the entries in a note's single per-device
`log.*.ndjson` (one JSON `NoteLogEntry` per line); panics if no log
exists.

**Used by** — `fs_note_log_compacts_and_still_converges`.

---

## fn fs_note_log_compacts_and_still_converges

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_note_log_compacts_and_still_converges`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — 1000 edits on one note keep its per-device log bounded to at
most the compaction threshold (+1): each time it grows past 256 entries it is
collapsed back to its frontier, rather than growing to 1001. Reads still return
the latest body; the compacted log replicates to a fresh peer that converges;
and a delete after all that churn still produces a tombstone carrying the
recovered content (the newest upsert was retained by compaction) that
propagates.

---

## fn replicate_logs

**Identification** — `async fn replicate_logs(from, to)`. Marker
`// md:fn replicate_logs`.

**Code** — complete and verbatim:

```rust
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
                if name.starts_with("log.") && name.ends_with(".ndjson") {
                    tokio::fs::copy(f.path(), to_note_dir.join(&name))
                        .await
                        .unwrap();
                }
            }
        }
    }
}
```

**What it does** — Simulates Syncthing replicating one device's **single-writer
log files**: every global `logs/*.log` plus every per-note
`notes/{id}/log.*.ndjson`. Each has a single writer, so this never conflicts.
Projections (`note.md`, `meta.ndjson`) are **not** copied — they are
per-device caches the receiver regenerates from the logs on sync.

**Used by** — the global-log and note-index tests.

---

## fn drain_sync

**Identification** — `async fn drain_sync(b: &FsBackend)`. Marker
`// md:fn drain_sync`.

**Code** — complete and verbatim:

```rust
// md:fn drain_sync
async fn drain_sync(b: &FsBackend) {
    for c in b.receive_changes().await.unwrap() {
        b.apply_change(c).await.unwrap();
    }
}
```

**What it does** — Pulls (`receive_changes`) and applies every change a device
can currently see from its peers' replicated logs.

---

## fn own_log_stats

**Identification** — `async fn own_log_stats(root, backend) -> (u64, usize)`.
Marker `// md:fn own_log_stats`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Parses a device's own global `logs/{device}.log` text
directly (no `FsBackend` internals): returns the generation epoch (from the
`__keeplin_epoch__` header line) and the count of change entries.

**Used by** — the two global-log compaction tests.

---

## fn fs_global_log_compacts_and_peer_converges

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_global_log_compacts_and_peer_converges`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The global NDJSON journal is bounded by generation-epoch
snapshot compaction: after a synced two-notebook baseline, churning one
notebook 600 times (past the 512 threshold) and deleting the other leaves A's
log with an epoch ≥ 1 and far fewer entries than the ~601 mutations (each
entity collapsed to one snapshot entry). Peer B — which synced only the
epoch-0 baseline — detects the new generation, re-reads the snapshot, and
converges: the live notebook at `x600`, the deleted one tombstoned.

---

## fn fs_global_log_snapshot_covers_all_entity_types

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_global_log_snapshot_covers_all_entity_types`.

**Code** — complete and verbatim:

```rust
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
    let res = Resource::new(SYSTEM_RESOURCE_NOTE_ID, "f", "text/plain", "f.txt", 3);
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
```

**What it does** — The compaction snapshot covers **every** globally-journalled
entity type: after forcing a compaction, a brand-new peer that receives only
the post-compaction snapshot still reconstructs the notebook (latest title),
the tag, the resource, and the note↔tag association.

---

## fn ordering_fields_round_trip_and_manual_order_query

**Identification** — `#[tokio::test]`. Marker
`// md:fn ordering_fields_round_trip_and_manual_order_query`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Issues #49–#52 on `FsBackend`: `is_pinned`/`sort_key`
round-trip; single-note-page cursor pagination walks pinned band first, then
the legacy `sort_key 0` sentinel (effective 1000), then 1500;
`list_starred_notes` returns only the starred Inbox note;
`notebook_sort_profile` reports pinned keys and max normal key.

---

## fn note_index_reflects_local_writes_after_it_is_built

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_index_reflects_local_writes_after_it_is_built`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The in-memory note index is built lazily on the first
listing, then maintained in place: a create after the build appears
(incremental insert) and a delete disappears (incremental remove) — no listing
re-reads every note's logs.

---

## fn note_index_reflects_changes_pulled_from_a_peer

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_index_reflects_changes_pulled_from_a_peer`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — With B's index warmed while empty, a peer note (starred)
replicated and drained through a sync cycle flows through the same
`persist_note_projection` choke point, so it appears in both `list_notes` and
`list_starred_notes`.

---

## fn delete_for_unknown_sidecar_entity_leaves_a_tombstone_blocking_a_stale_create

**Identification** — `#[tokio::test]`. Marker
`// md:fn delete_for_unknown_sidecar_entity_leaves_a_tombstone_blocking_a_stale_create`.

**Code** — complete and verbatim:

```rust
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
        note_id: SYSTEM_RESOURCE_NOTE_ID,
        title: "resurrected?".into(),
        mime_type: "text/plain".into(),
        file_name: "f.txt".into(),
        size: 3,
        duration_ms: None,
        dimensions: None,
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
```

**What it does** — Issue #71 on `FsBackend`, for the sidecar entity types
(notebook, tag, resource): a delete arriving for an unknown entity writes a
minimal tombstone sidecar, so the causally older create (peer vv `{peer:1}` vs
the delete's `{peer:2}`) loses in `resolve` instead of resurrecting the
entity. Note deletes converge through the Syncthing-replicated per-note logs,
not this apply path — covered by the two-device convergence tests.

---

## fn fs_note_delete_cascades_to_attachments_and_restore_recovers_dragged

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_note_delete_cascades_to_attachments_and_restore_recovers_dragged`.

**Code** — complete and verbatim:

```rust
// md:fn fs_note_delete_cascades_to_attachments_and_restore_recovers_dragged
#[tokio::test]
async fn fs_note_delete_cascades_to_attachments_and_restore_recovers_dragged() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let note = backend.create_note(Note::new("N", "")).await.unwrap();

    let r1 = backend
        .create_resource(
            Resource::new(note.id, "a", "text/plain", "a.txt", 1),
            vec![1],
        )
        .await
        .unwrap();
    let r2 = backend
        .create_resource(
            Resource::new(note.id, "b", "text/plain", "b.txt", 1),
            vec![2],
        )
        .await
        .unwrap();
    let r3 = backend
        .create_resource(
            Resource::new(note.id, "c", "text/plain", "c.txt", 1),
            vec![3],
        )
        .await
        .unwrap();

    backend.delete_resource(r3.id).await.unwrap();
    backend.delete_note(note.id).await.unwrap();

    assert!(matches!(
        backend.read_resource(r1.id).await,
        Err(StorageError::NotFound(_))
    ));
    assert!(matches!(
        backend.read_resource(r2.id).await,
        Err(StorageError::NotFound(_))
    ));

    let mut revived = backend.read_note(note.id).await.unwrap();
    revived.deleted_at = None;
    backend.update_note(revived).await.unwrap();

    assert!(
        backend.read_resource(r1.id).await.is_ok(),
        "r1 dragged back"
    );
    assert!(
        backend.read_resource(r2.id).await.is_ok(),
        "r2 dragged back"
    );
    assert!(
        matches!(
            backend.read_resource(r3.id).await,
            Err(StorageError::NotFound(_))
        ),
        "a directly-deleted attachment keeps its own tombstone through the restore"
    );
}
```

**What it does** — The `FsBackend` twin of the db cascade test (issue #125): deleting a note
stamps its live attachments (`cascade_stamp_resources`), and reviving the note un-stamps only
the ones it dragged (`cascade_unstamp_resources`, matching on the note's prior tombstone ts).
`r3`, deleted directly with its own ts, survives the restore.

**Dependencies** —
- `delete_note`/`update_note` — the sidecar cascade helpers; expect the un-stamp to match on the
  note's prior `deleted_at` read via `merge_note`.

**Used by** — n/a (integration test).

**Repeated context** — the cascade rewrites only the resource sidecar's `deleted_at`, no vv bump
or journal entry.

---

## fn fs_moving_note_between_notebooks_leaves_attachments_untouched

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_moving_note_between_notebooks_leaves_attachments_untouched`.

**Code** — complete and verbatim:

```rust
// md:fn fs_moving_note_between_notebooks_leaves_attachments_untouched
#[tokio::test]
async fn fs_moving_note_between_notebooks_leaves_attachments_untouched() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let mut note = backend.create_note(Note::new("N", "")).await.unwrap();
    let res = backend
        .create_resource(
            Resource::new(note.id, "a", "text/plain", "a.txt", 1),
            vec![1],
        )
        .await
        .unwrap();

    note.notebook_id = uuid::Uuid::new_v4();
    backend.update_note(note.clone()).await.unwrap();

    let (read, _) = backend.read_resource(res.id).await.unwrap();
    assert_eq!(read.note_id, note.id, "note_id is unchanged by a move");
    assert!(
        read.deleted_at.is_none(),
        "a move never tombstones attachments"
    );
}
```

**What it does** — The "move ≠ touch attachments" invariant on `FsBackend`: changing a note's
`notebook_id` leaves its resources' `note_id` and `deleted_at` intact.

**Dependencies** —
- `update_note` — the revive-cascade only fires on a tombstone→live transition, which a plain
  move is not; expects a move to be resource-inert.

**Used by** — n/a (integration test).

**Repeated context** — the attachment link is to the note, not the notebook.

---

## fn fs_list_resources_for_note_orders_and_excludes_others

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_list_resources_for_note_orders_and_excludes_others`.

**Code** — complete and verbatim:

```rust
// md:fn fs_list_resources_for_note_orders_and_excludes_others
#[tokio::test]
async fn fs_list_resources_for_note_orders_and_excludes_others() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();
    let a = backend.create_note(Note::new("A", "")).await.unwrap();
    let b = backend.create_note(Note::new("B", "")).await.unwrap();

    let a1 = backend
        .create_resource(
            Resource::new(a.id, "a1", "text/plain", "a1.txt", 1),
            vec![1],
        )
        .await
        .unwrap();
    let a2 = backend
        .create_resource(
            Resource::new(a.id, "a2", "text/plain", "a2.txt", 1),
            vec![2],
        )
        .await
        .unwrap();
    backend
        .create_resource(
            Resource::new(b.id, "b1", "text/plain", "b1.txt", 1),
            vec![3],
        )
        .await
        .unwrap();
    backend
        .create_resource(
            Resource::new(SYSTEM_RESOURCE_NOTE_ID, "sys", "text/plain", "s.txt", 1),
            vec![4],
        )
        .await
        .unwrap();

    let (listed, _) = backend
        .list_resources_for_note(a.id, 0, None)
        .await
        .unwrap();
    let ids: Vec<_> = listed.iter().map(|r| r.id).collect();
    assert_eq!(
        ids,
        vec![a1.id, a2.id],
        "only note A's attachments, in created_at order, no note B or system resources"
    );
}
```

**What it does** — `list_resources_for_note` on `FsBackend` returns exactly the target note's
attachments in `created_at` order, excluding another note's and system-sentinel resources.

**Dependencies** —
- `list_resources_for_note` — the native filtered dir scan; expects `r.note_id == note_id` plus
  the `(created_at, id)` sort.

**Used by** — n/a (integration test).

**Repeated context** — per-note listings exclude `SYSTEM_RESOURCE_NOTE_ID`.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `replicate_note()`, `replicate_logs()`, `drain_sync()`, `note_log_len()`, `own_log_stats()` — helpers defined here (EXTRACTED; file-local)
- the 40 `#[tokio::test]` functions — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: references; e.g. `FsBackend`)
- `keeplin-core/src/storage/note_log.rs` — `NoteLogEntry` and the resolve/merge semantics exercised via replication (EXTRACTED: references)
- `keeplin-core/src/models.rs` — entities and `Change` (EXTRACTED: references)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Replication copies only single-writer logs; projections are regenerated — the tests must never copy `note.md`/`meta.ndjson` between roots.
- Convergence (causal, concurrent equal-timestamp, add/remove races) and both compaction bounds (per-note 256, global 512 with epoch snapshots) are pinned here.
- Tombstone-first races (#71) stay covered for the sidecar entity types; note deletes converge through the per-note logs.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `fn create_and_read_note` | `// md:fn create_and_read_note` |
| 3 | `fn update_note` | `// md:fn update_note` |
| 4 | `fn delete_note_soft_deletes` | `// md:fn delete_note_soft_deletes` |
| 5 | `fn list_notes_excludes_deleted` | `// md:fn list_notes_excludes_deleted` |
| 6 | `fn read_nonexistent_note_returns_not_found` | `// md:fn read_nonexistent_note_returns_not_found` |
| 7 | `fn device_id_is_stable_across_instances` | `// md:fn device_id_is_stable_across_instances` |
| 8 | `fn sync_state_persists` | `// md:fn sync_state_persists` |
| 9 | `fn get_changes_since_scans_other_device_logs` | `// md:fn get_changes_since_scans_other_device_logs` |
| 10 | `fn update_nonexistent_note_returns_not_found` | `// md:fn update_nonexistent_note_returns_not_found` |
| 11 | `fn delete_nonexistent_note_returns_not_found` | `// md:fn delete_nonexistent_note_returns_not_found` |
| 12 | `fn update_nonexistent_notebook_returns_not_found` | `// md:fn update_nonexistent_notebook_returns_not_found` |
| 13 | `fn delete_nonexistent_notebook_returns_not_found` | `// md:fn delete_nonexistent_notebook_returns_not_found` |
| 14 | `fn update_nonexistent_tag_returns_not_found` | `// md:fn update_nonexistent_tag_returns_not_found` |
| 15 | `fn delete_nonexistent_tag_returns_not_found` | `// md:fn delete_nonexistent_tag_returns_not_found` |
| 16 | `fn create_and_read_notebook` | `// md:fn create_and_read_notebook` |
| 17 | `fn list_notebooks_includes_created` | `// md:fn list_notebooks_includes_created` |
| 18 | `fn delete_notebook_soft_deletes` | `// md:fn delete_notebook_soft_deletes` |
| 19 | `fn create_and_read_tag` | `// md:fn create_and_read_tag` |
| 20 | `fn list_tags_includes_created` | `// md:fn list_tags_includes_created` |
| 21 | `fn add_and_list_note_tags` | `// md:fn add_and_list_note_tags` |
| 22 | `fn add_note_tag_rejects_missing_or_deleted_ends` | `// md:fn add_note_tag_rejects_missing_or_deleted_ends` |
| 23 | `fn remove_note_tag` | `// md:fn remove_note_tag` |
| 24 | `fn create_and_read_resource` | `// md:fn create_and_read_resource` |
| 25 | `fn list_resources_excludes_data` | `// md:fn list_resources_excludes_data` |
| 26 | `fn delete_resource` | `// md:fn delete_resource` |
| 27 | `fn list_notes_paginates_without_duplicates_or_gaps` | `// md:fn list_notes_paginates_without_duplicates_or_gaps` |
| 28 | `fn replicate_note` | `// md:fn replicate_note` |
| 29 | `fn fs_note_uses_three_file_layout` | `// md:fn fs_note_uses_three_file_layout` |
| 30 | `fn fs_on_disk_serialization_is_ndjson` | `// md:fn fs_on_disk_serialization_is_ndjson` |
| 31 | `fn fs_two_device_causal_sync` | `// md:fn fs_two_device_causal_sync` |
| 32 | `fn fs_two_device_concurrent_edits_converge` | `// md:fn fs_two_device_concurrent_edits_converge` |
| 33 | `fn note_alias_bookmarks_links_persist_in_meta` | `// md:fn note_alias_bookmarks_links_persist_in_meta` |
| 34 | `fn backlinks_default_scan_is_paginated` | `// md:fn backlinks_default_scan_is_paginated` |
| 35 | `fn fs_notebook_concurrent_equal_timestamp_edits_converge` | `// md:fn fs_notebook_concurrent_equal_timestamp_edits_converge` |
| 36 | `fn fs_concurrent_note_tag_add_remove_converges` | `// md:fn fs_concurrent_note_tag_add_remove_converges` |
| 37 | `fn note_log_len` | `// md:fn note_log_len` |
| 38 | `fn fs_note_log_compacts_and_still_converges` | `// md:fn fs_note_log_compacts_and_still_converges` |
| 39 | `fn replicate_logs` | `// md:fn replicate_logs` |
| 40 | `fn drain_sync` | `// md:fn drain_sync` |
| 41 | `fn own_log_stats` | `// md:fn own_log_stats` |
| 42 | `fn fs_global_log_compacts_and_peer_converges` | `// md:fn fs_global_log_compacts_and_peer_converges` |
| 43 | `fn fs_global_log_snapshot_covers_all_entity_types` | `// md:fn fs_global_log_snapshot_covers_all_entity_types` |
| 44 | `fn ordering_fields_round_trip_and_manual_order_query` | `// md:fn ordering_fields_round_trip_and_manual_order_query` |
| 45 | `fn note_index_reflects_local_writes_after_it_is_built` | `// md:fn note_index_reflects_local_writes_after_it_is_built` |
| 46 | `fn note_index_reflects_changes_pulled_from_a_peer` | `// md:fn note_index_reflects_changes_pulled_from_a_peer` |
| 47 | `fn delete_for_unknown_sidecar_entity_leaves_a_tombstone_blocking_a_stale_create` | `// md:fn delete_for_unknown_sidecar_entity_leaves_a_tombstone_blocking_a_stale_create` |
| 48 | `fn fs_note_delete_cascades_to_attachments_and_restore_recovers_dragged` | `// md:fn fs_note_delete_cascades_to_attachments_and_restore_recovers_dragged` |
| 49 | `fn fs_moving_note_between_notebooks_leaves_attachments_untouched` | `// md:fn fs_moving_note_between_notebooks_leaves_attachments_untouched` |
| 50 | `fn fs_list_resources_for_note_orders_and_excludes_others` | `// md:fn fs_list_resources_for_note_orders_and_excludes_others` |