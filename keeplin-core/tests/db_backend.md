# `tests/db_backend.rs` — DbBackend integration tests

Self-contained companion for `keeplin-core/tests/db_backend.rs`. It documents
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

use keeplin_core::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag, SYSTEM_RESOURCE_NOTE_ID},
    storage::{
        db::DbBackend, NoteRepository, NotebookRepository, ResourceRepository, SyncBackend,
        TagRepository,
    },
};
use tempfile::tempdir;
```

**What it does** — Integration tests for `DbBackend`, the LibSQL-backed storage
implementation. Every test uses `in_memory_backend` (a temp-file database with
an empty `server_url`, so no WebSocket is attempted) and covers the complete
`StorageBackend` API at the SQLite level: CRUD for all entities, soft-deletion
semantics, the `entity_changes` change journal (including the
no-re-journal-on-apply invariant and pruning), device-ID persistence, keyset
pagination, backlink indexing, ordering/starring fields, tombstone-first
races, and write concurrency. WebSocket sync paths live in `ws_sync.rs`.

**Repeated context** — soft delete everywhere: `delete_*` stamps `deleted_at`
(a tombstone that still reads back but is excluded from listings); reading a
missing entity is `StorageError::NotFound`. The journal records the **original
operation type** (create vs update). Keyset pagination orders by
`(created_at, id)`.

---

## fn in_memory_backend

**Identification** — `async fn in_memory_backend() -> DbBackend`. Marker
`// md:fn in_memory_backend`.

**Code** — complete and verbatim:

```rust
// md:fn in_memory_backend
async fn in_memory_backend() -> DbBackend {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    std::mem::forget(dir);
    DbBackend::new(db_path, "", "").await.unwrap()
}
```

**What it does** — A `DbBackend` on a temp-file database with empty
`server_url`/`auth_token` (offline mode — no WebSocket). The tempdir is leaked
with `std::mem::forget` so the directory outlives the open database file; the
OS cleans it up at process exit.

**Used by** — every test except `device_id_is_stable`.

---

## fn create_and_read_note

**Identification** — `#[tokio::test]`. Marker `// md:fn create_and_read_note`.

**Code** — complete and verbatim:

```rust
// md:fn create_and_read_note
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
```

**What it does** — Basic note round-trip: create, read back title and body.

---

## fn update_note

**Identification** — `#[tokio::test]`. Marker `// md:fn update_note`.

**Code** — complete and verbatim:

```rust
// md:fn update_note
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
```

**What it does** — Update persists the new title.

---

## fn delete_note_soft_deletes

**Identification** — `#[tokio::test]`. Marker `// md:fn delete_note_soft_deletes`.

**Code** — complete and verbatim:

```rust
// md:fn delete_note_soft_deletes
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
```

**What it does** — Of two notes, deleting one leaves exactly the survivor in
the listing.

---

## fn read_nonexistent_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn read_nonexistent_returns_not_found`.

**Code** — complete and verbatim:

```rust
// md:fn read_nonexistent_returns_not_found
#[tokio::test]
async fn read_nonexistent_returns_not_found() {
    let backend = in_memory_backend().await;
    let id = uuid::Uuid::new_v4();
    let err = backend.read_note(id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}
```

**What it does** — Reading a random UUID → `StorageError::NotFound`.

---

## fn device_id_is_stable

**Identification** — `#[tokio::test]`. Marker `// md:fn device_id_is_stable`.

**Code** — complete and verbatim:

```rust
// md:fn device_id_is_stable
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
```

**What it does** — Two `DbBackend` openings of the same `.db` file return the
same persisted device id.

---

## fn sync_state_round_trips

**Identification** — `#[tokio::test]`. Marker `// md:fn sync_state_round_trips`.

**Code** — complete and verbatim:

```rust
// md:fn sync_state_round_trips
#[tokio::test]
async fn sync_state_round_trips() {
    let backend = in_memory_backend().await;

    let ts = chrono::Utc::now();
    backend.update_sync_time(ts).await.unwrap();
    let read = backend.get_last_sync_time().await.unwrap();
    assert_eq!(read.timestamp(), ts.timestamp());
}
```

**What it does** — `update_sync_time`/`get_last_sync_time` round-trip at
second precision.

---

## fn get_changes_since_returns_updated_notes

**Identification** — `#[tokio::test]`. Marker
`// md:fn get_changes_since_returns_updated_notes`.

**Code** — complete and verbatim:

```rust
// md:fn get_changes_since_returns_updated_notes
#[tokio::test]
async fn get_changes_since_returns_updated_notes() {
    use keeplin_core::models::Change;

    let backend = in_memory_backend().await;
    let before = chrono::Utc::now() - chrono::Duration::seconds(1);

    let note = Note::new("New note", "Body");
    backend.create_note(note).await.unwrap();

    let changes = backend.get_changes_since(before).await.unwrap();
    assert!(!changes.is_empty());
    assert!(matches!(changes[0], Change::NoteCreate { .. }));
}
```

**What it does** — A note created after `since` appears in the change list as
`Change::NoteCreate`, not `NoteUpdate` — the `entity_changes` journal records
the original operation type.

---

## fn prune_change_journal_removes_rows_older_than_cutoff

**Identification** — `#[tokio::test]`. Marker
`// md:fn prune_change_journal_removes_rows_older_than_cutoff`.

**Code** — complete and verbatim:

```rust
// md:fn prune_change_journal_removes_rows_older_than_cutoff
#[tokio::test]
async fn prune_change_journal_removes_rows_older_than_cutoff() {
    let backend = in_memory_backend().await;
    let epoch = chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap();

    backend.create_note(Note::new("a", "")).await.unwrap();
    backend.create_note(Note::new("b", "")).await.unwrap();
    assert_eq!(backend.get_changes_since(epoch).await.unwrap().len(), 2);

    let removed = backend.prune_change_journal(epoch).await.unwrap();
    assert_eq!(removed, 0);
    assert_eq!(backend.get_changes_since(epoch).await.unwrap().len(), 2);

    let future = chrono::Utc::now() + chrono::Duration::days(1);
    let removed = backend.prune_change_journal(future).await.unwrap();
    assert_eq!(removed, 2);
    assert!(backend.get_changes_since(epoch).await.unwrap().is_empty());
}
```

**What it does** — With two journaled creates: a cutoff in the past removes 0
rows (journal untouched); a cutoff in the future removes both and reports the
count, leaving the journal empty.

---

## fn apply_change_is_not_re_journaled

**Identification** — `#[tokio::test]`. Marker
`// md:fn apply_change_is_not_re_journaled`.

**Code** — complete and verbatim:

```rust
// md:fn apply_change_is_not_re_journaled
#[tokio::test]
async fn apply_change_is_not_re_journaled() {
    use keeplin_core::models::Change;

    let backend = in_memory_backend().await;
    let epoch = chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap();

    let remote = Note::new("remote", "from a peer");
    let remote_id = remote.id;
    backend
        .apply_change(Change::NoteCreate { note: remote })
        .await
        .unwrap();
    assert_eq!(backend.read_note(remote_id).await.unwrap().title, "remote");

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
```

**What it does** — The journal holds only changes that **originated on this
device**: a `NoteCreate` ingested via `apply_change` (a remote change from the
relay) is applied to the tables (readable) but never enters the journal — so it
is never re-sent to the relay — while a locally created note does. Pins the
invariant documented on `DbBackend::apply_change`.

---

## fn update_nonexistent_note_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn update_nonexistent_note_returns_not_found`.

**Code** — complete and verbatim:

```rust
// md:fn update_nonexistent_note_returns_not_found
#[tokio::test]
async fn update_nonexistent_note_returns_not_found() {
    let backend = in_memory_backend().await;
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
    let backend = in_memory_backend().await;
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
    let backend = in_memory_backend().await;
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
    let backend = in_memory_backend().await;
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
    let backend = in_memory_backend().await;
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
    let backend = in_memory_backend().await;
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
    let backend = in_memory_backend().await;
    let nb = Notebook::new("Personal");
    let id = nb.id;
    backend.create_notebook(nb).await.unwrap();

    let read = backend.read_notebook(id).await.unwrap();
    assert_eq!(read.title, "Personal");
    assert!(read.deleted_at.is_none());
}
```

**What it does** — Notebook round-trip; `deleted_at` starts `None`.

---

## fn delete_notebook_soft_deletes

**Identification** — `#[tokio::test]`. Marker
`// md:fn delete_notebook_soft_deletes`.

**Code** — complete and verbatim:

```rust
// md:fn delete_notebook_soft_deletes
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
    let backend = in_memory_backend().await;
    let tag = Tag::new("async");
    let id = tag.id;
    backend.create_tag(tag).await.unwrap();

    let read = backend.read_tag(id).await.unwrap();
    assert_eq!(read.title, "async");
}
```

**What it does** — Tag round-trip.

---

## fn add_and_list_note_tags

**Identification** — `#[tokio::test]`. Marker `// md:fn add_and_list_note_tags`.

**Code** — complete and verbatim:

```rust
// md:fn add_and_list_note_tags
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
    let backend = in_memory_backend().await;
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

    backend.delete_note(note_id).await.unwrap();
    let err = backend
        .add_note_tag(NoteTag { note_id, tag_id })
        .await
        .unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)), "got: {err}");

    let (tags, _) = backend.list_note_tags(note_id, 0, None).await.unwrap();
    assert!(tags.is_empty());
}
```

**What it does** — `add_note_tag` with a nonexistent note or tag id, or a
soft-deleted note, fails with `NotFound` — no dangling association is created
(the listing stays empty).

---

## fn pagination_walks_notes_sharing_a_created_at

**Identification** — `#[tokio::test]`. Marker
`// md:fn pagination_walks_notes_sharing_a_created_at`.

**Code** — complete and verbatim:

```rust
// md:fn pagination_walks_notes_sharing_a_created_at
#[tokio::test]
async fn pagination_walks_notes_sharing_a_created_at() {
    let backend = in_memory_backend().await;
    let shared_ts = chrono::Utc::now();
    let mut expected = Vec::new();
    for i in 0..3 {
        let mut note = Note::new(format!("n{i}"), "");
        note.created_at = shared_ts;
        expected.push(note.id);
        backend.create_note(note).await.unwrap();
    }
    expected.sort();

    let mut seen = Vec::new();
    let mut token = None;
    loop {
        let (page, next) = backend.list_notes(1, token).await.unwrap();
        assert!(page.len() <= 1);
        seen.extend(page.into_iter().map(|n| n.id));
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    assert_eq!(
        seen, expected,
        "every note exactly once, in (created_at, id) order"
    );
}
```

**What it does** — Keyset pagination visits every row exactly once even when
three rows share one `created_at` — the case relying on the cursor's
`created_at = ?` equality branch, which in turn relies on the fixed-precision
timestamp format. Walked page size 1; order is `(created_at, id)`.

---

## fn remove_note_tag

**Identification** — `#[tokio::test]`. Marker `// md:fn remove_note_tag`.

**Code** — complete and verbatim:

```rust
// md:fn remove_note_tag
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
```

**What it does** — Detach after attach; the listing is empty again.

---

## fn purge_reclaims_old_tombstoned_payloads_only

**Identification** — `#[tokio::test]`. Marker
`// md:fn purge_reclaims_old_tombstoned_payloads_only`.

**Code** — complete and verbatim:

```rust
// md:fn purge_reclaims_old_tombstoned_payloads_only
#[tokio::test]
async fn purge_reclaims_old_tombstoned_payloads_only() {
    let backend = in_memory_backend().await;

    let dead = Resource::new(SYSTEM_RESOURCE_NOTE_ID, "dead", "text/plain", "d.txt", 4);
    let dead_id = dead.id;
    backend
        .create_resource(dead, b"dead".to_vec())
        .await
        .unwrap();
    backend.delete_resource(dead_id).await.unwrap();

    let live = Resource::new(SYSTEM_RESOURCE_NOTE_ID, "live", "text/plain", "l.txt", 4);
    let live_id = live.id;
    backend
        .create_resource(live, b"live".to_vec())
        .await
        .unwrap();

    let epoch = chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap();
    assert_eq!(backend.purge_deleted_resources(epoch).await.unwrap(), 0);
    assert_eq!(
        backend
            .purge_deleted_resources(chrono::Utc::now())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        backend
            .purge_deleted_resources(chrono::Utc::now())
            .await
            .unwrap(),
        0
    );
    assert!(matches!(
        backend.read_resource(dead_id).await,
        Err(StorageError::NotFound(_))
    ));
    let (_, bytes) = backend.read_resource(live_id).await.unwrap();
    assert_eq!(bytes, b"live");
}
```

**What it does** — `purge_deleted_resources`: a cutoff before the tombstone
purges nothing; one after it frees exactly the dead payload (count 1); the call
is idempotent (second run counts 0); the tombstone still reads as `NotFound`
and the live resource's bytes are untouched.

---

## fn create_and_read_resource

**Identification** — `#[tokio::test]`. Marker `// md:fn create_and_read_resource`.

**Code** — complete and verbatim:

```rust
// md:fn create_and_read_resource
#[tokio::test]
async fn create_and_read_resource() {
    let backend = in_memory_backend().await;

    let data = b"binary content".to_vec();
    let res = Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        "img",
        "image/png",
        "img.png",
        data.len() as u64,
    );
    let id = res.id;
    backend.create_resource(res, data.clone()).await.unwrap();

    let (meta, bytes) = backend.read_resource(id).await.unwrap();
    assert_eq!(meta.title, "img");
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
    let backend = in_memory_backend().await;

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

**What it does** — Three resources list as metadata (no payloads inline).

---

## fn delete_resource

**Identification** — `#[tokio::test]`. Marker `// md:fn delete_resource`.

**Code** — complete and verbatim:

```rust
// md:fn delete_resource
#[tokio::test]
async fn delete_resource() {
    let backend = in_memory_backend().await;

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
    let backend = in_memory_backend().await;

    let total = 25usize;
    for i in 0..total {
        backend
            .create_note(Note::new(format!("Note {i:02}"), ""))
            .await
            .unwrap();
    }

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

**What it does** — 25 notes walked with page size 10: no page exceeds the
size, every note appears exactly once, and the paged order equals the
single-shot order (stable keyset `created_at ASC, id ASC`).

---

## fn concurrent_note_creates_all_succeed

**Identification** — `#[tokio::test(flavor = "multi_thread", worker_threads =
4)]`. Marker `// md:fn concurrent_note_creates_all_succeed`.

**Code** — complete and verbatim:

```rust
// md:fn concurrent_note_creates_all_succeed
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

    let (notes, _) = backend.list_notes(100, None).await.unwrap();
    assert_eq!(notes.len(), 50);
}
```

**What it does** — 50 concurrent `create_note` tasks all commit. `DbBackend`
wraps every mutation in `BEGIN IMMEDIATE … COMMIT` on a single shared
connection, so without serialisation a second `BEGIN` before the first `COMMIT`
would fail ("cannot start a transaction within a transaction"). All 50 notes
are queryable afterwards.

---

## fn concurrent_reads_and_writes_make_progress

**Identification** — `#[tokio::test(flavor = "multi_thread", worker_threads =
4)]`. Marker `// md:fn concurrent_reads_and_writes_make_progress`.

**Code** — complete and verbatim:

```rust
// md:fn concurrent_reads_and_writes_make_progress
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reads_and_writes_make_progress() {
    use std::sync::Arc;

    let backend = Arc::new(in_memory_backend().await);

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
```

**What it does** — 20 writers interleaved with 20 readers (point reads + list
reads) all complete — the read/write guard around the shared connection must
never deadlock (a reader must not block a reader; the two sides are never
acquired re-entrantly by one task). Final count is seed + 20.

---

## fn note_alias_bookmarks_links_round_trip

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_alias_bookmarks_links_round_trip`.

**Code** — complete and verbatim:

```rust
// md:fn note_alias_bookmarks_links_round_trip
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

    let read = backend.read_note(note.id).await.unwrap();
    assert_eq!(read.alias.as_deref(), Some("note3"));
    assert_eq!(read.bookmarks, note.bookmarks);
    assert_eq!(read.links, note.links);

    let mut edited = read;
    edited.alias = Some("renamed".to_string());
    edited.bookmarks[0].alias = "Edited".to_string();
    backend.update_note(edited.clone()).await.unwrap();
    let reread = backend.read_note(note.id).await.unwrap();
    assert_eq!(reread.alias.as_deref(), Some("renamed"));
    assert_eq!(reread.bookmarks[0].alias, "Edited");
}
```

**What it does** — Alias, bookmarks, and links persist through the SQLite
columns: create preserves the content fields verbatim while stamping `vv` and
`last_writer` (asserted non-empty); read-back matches; editing the alias and a
bookmark alias persists.

---

## fn notebook_alias_round_trip

**Identification** — `#[tokio::test]`. Marker `// md:fn notebook_alias_round_trip`.

**Code** — complete and verbatim:

```rust
// md:fn notebook_alias_round_trip
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
```

**What it does** — A notebook alias survives read and listing.

---

## fn indexed_backlinks_track_writes_and_deletes

**Identification** — `#[tokio::test]`. Marker
`// md:fn indexed_backlinks_track_writes_and_deletes`.

**Code** — complete and verbatim:

```rust
// md:fn indexed_backlinks_track_writes_and_deletes
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

    backend.create_note(Note::new("other", "")).await.unwrap();

    let (back, _) = backend.note_backlinks(target.id, 0, None).await.unwrap();
    assert_eq!(back.len(), 2, "both sources link to target");

    let mut s = src1.clone();
    s.links.clear();
    backend.update_note(s).await.unwrap();
    let (back, _) = backend.note_backlinks(target.id, 0, None).await.unwrap();
    assert_eq!(back.len(), 1);

    backend.delete_note(src2.id).await.unwrap();
    let (back, _) = backend.note_backlinks(target.id, 0, None).await.unwrap();
    assert!(back.is_empty());
}
```

**What it does** — The backlink index: two sources linking to a target both
appear (an unrelated note does not); clearing src1's links via update drops it
from the index; soft-deleting src2 excludes it too (the JOIN filters deleted
sources) — backlinks end empty.

---

## fn backlinks_are_paginated

**Identification** — `#[tokio::test]`. Marker `// md:fn backlinks_are_paginated`.

**Code** — complete and verbatim:

```rust
// md:fn backlinks_are_paginated
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

    let ids: std::collections::HashSet<_> = p1.iter().chain(&p2).map(|n| n.id).collect();
    assert_eq!(ids.len(), 3);
}
```

**What it does** — Three backlinks walked with page size 2: pages of 2 and 1,
no third page, and the union covers all three sources without overlap.

---

## fn ordering_fields_round_trip_and_manual_order_query

**Identification** — `#[tokio::test]`. Marker
`// md:fn ordering_fields_round_trip_and_manual_order_query`.

**Code** — complete and verbatim:

```rust
// md:fn ordering_fields_round_trip_and_manual_order_query
#[tokio::test]
async fn ordering_fields_round_trip_and_manual_order_query() {
    let backend = in_memory_backend().await;
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

    let (page, next) = backend
        .list_notes_in_notebook(nb.id, 0, None)
        .await
        .unwrap();
    let titles: Vec<&str> = page.iter().map(|n| n.title.as_str()).collect();
    assert_eq!(titles, ["pinned", "legacy", "normal"]);
    assert!(next.is_none());

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
    assert_eq!(profile.min_key, Some(5));
    assert_eq!(profile.max_normal_key, Some(1500));
}
```

**What it does** — Issues #49–#52: `is_pinned`/`sort_key` round-trip; the
manual order query returns pinned band first, then the legacy `sort_key 0`
sentinel (ordering as the effective `NORMAL_START = 1000`), then 1500 — and
cursor pagination walks the identical order one note at a time.
`list_starred_notes` spans notebooks and excludes everything unstarred (the
starred note lives in the Inbox). `notebook_sort_profile` summarises the
notebook for placement (pinned keys, min key, max normal key).

---

## fn sync_applied_change_carries_ordering_fields

**Identification** — `#[tokio::test]`. Marker
`// md:fn sync_applied_change_carries_ordering_fields`.

**Code** — complete and verbatim:

```rust
// md:fn sync_applied_change_carries_ordering_fields
#[tokio::test]
async fn sync_applied_change_carries_ordering_fields() {
    let backend = in_memory_backend().await;

    let mut note = Note::new("from peer", "body");
    note.is_pinned = true;
    note.is_starred = true;
    note.sort_key = 7;
    note.vv.insert("peer".to_string(), 1);
    note.last_writer = "peer".to_string();
    backend
        .apply_change(Change::NoteCreate { note: note.clone() })
        .await
        .unwrap();

    let read = backend.read_note(note.id).await.unwrap();
    assert!(read.is_pinned);
    assert!(read.is_starred);
    assert_eq!(read.sort_key, 7);
    let (stars, _) = backend.list_starred_notes(0, None).await.unwrap();
    assert_eq!(stars.len(), 1, "sync-applied stars are queryable");
}
```

**What it does** — Issue #55: an `apply_change`-ingested note carries
`is_pinned`/`is_starred`/`sort_key` intact (whole-note version-vector
resolution treats them like any other field), and sync-applied stars are
queryable via `list_starred_notes`.

---

## fn delete_for_unknown_entity_leaves_a_tombstone_blocking_a_stale_create

**Identification** — `#[tokio::test]`. Marker
`// md:fn delete_for_unknown_entity_leaves_a_tombstone_blocking_a_stale_create`.

**Code** — complete and verbatim:

```rust
// md:fn delete_for_unknown_entity_leaves_a_tombstone_blocking_a_stale_create
#[tokio::test]
async fn delete_for_unknown_entity_leaves_a_tombstone_blocking_a_stale_create() {
    let backend = in_memory_backend().await;
    let vv = |dev: &str, n: u64| std::collections::BTreeMap::from([(dev.to_string(), n)]);
    let ts = chrono::Utc::now();

    let note_id = uuid::Uuid::new_v4();
    backend
        .apply_change(Change::NoteDelete {
            id: note_id,
            deleted_at: ts,
            vv: vv("peer", 2),
            last_writer: "peer".into(),
        })
        .await
        .unwrap();
    let mut stale = Note::new("resurrected?", "body");
    stale.id = note_id;
    stale.vv = vv("peer", 1);
    stale.last_writer = "peer".into();
    backend
        .apply_change(Change::NoteCreate { note: stale })
        .await
        .unwrap();
    let (notes, _) = backend.list_notes(0, None).await.unwrap();
    assert!(
        !notes.iter().any(|n| n.id == note_id),
        "a stale create must not resurrect a note deleted before it was known"
    );
    assert!(backend
        .read_note(note_id)
        .await
        .unwrap()
        .deleted_at
        .is_some());

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
    assert!(!nbs.iter().any(|n| n.id == nb_id), "notebook stays deleted");

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

**What it does** — Issue #71, for all four versioned entity types (note,
notebook, tag, resource): a delete arriving for an entity this backend has
never seen (peer vv `{peer:2}`) inserts a minimal tombstone (each `apply_change`
arm does this when the `UPDATE` hits no row), so the causally older create
(vv `{peer:1}`) that arrives afterwards loses against the stored tombstone —
nothing is resurrected, listings stay empty, and the note's tombstone reads
back with `deleted_at` set.

---

## fn note_delete_cascades_to_attachments_and_restore_recovers_dragged

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_delete_cascades_to_attachments_and_restore_recovers_dragged`.

**Code** — complete and verbatim:

```rust
// md:fn note_delete_cascades_to_attachments_and_restore_recovers_dragged
#[tokio::test]
async fn note_delete_cascades_to_attachments_and_restore_recovers_dragged() {
    let backend = in_memory_backend().await;
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

**What it does** — The core cascade guarantee (issue #125) on `DbBackend`: deleting a note
soft-deletes its live attachments (`r1`, `r2` read back `NotFound`), and restoring the note
(an `update_note` clearing `deleted_at`) revives **only** the ones the note dragged down —
those stamped with the note's tombstone ts. `r3`, deleted directly beforehand with its own
distinct ts, keeps its tombstone through the restore.

**Dependencies** —
- `delete_note`/`update_note` — apply the cascade stamp/un-stamp; expect the un-stamp to match
  resources on `deleted_at = <the note's prior tombstone ts>`.
- `create_resource`/`read_resource`/`delete_resource` — attachment lifecycle.

**Used by** — n/a (integration test).

**Repeated context** — soft-delete everywhere; a tombstoned resource reads back `NotFound`.

---

## fn moving_note_between_notebooks_leaves_attachments_untouched

**Identification** — `#[tokio::test]`. Marker
`// md:fn moving_note_between_notebooks_leaves_attachments_untouched`.

**Code** — complete and verbatim:

```rust
// md:fn moving_note_between_notebooks_leaves_attachments_untouched
#[tokio::test]
async fn moving_note_between_notebooks_leaves_attachments_untouched() {
    let backend = in_memory_backend().await;
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

**What it does** — Confirms the "move ≠ touch attachments" invariant: the link is to the note,
not the notebook, so changing a note's `notebook_id` leaves its resources' `note_id` and
`deleted_at` untouched.

**Dependencies** —
- `update_note` — no path here writes `resources`; expects the notebook move to be resource-inert.

**Used by** — n/a (integration test).

**Repeated context** — `note_id` is immutable after creation; attachments are never reparented.

---

## fn list_resources_for_note_orders_and_excludes_others

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_resources_for_note_orders_and_excludes_others`.

**Code** — complete and verbatim:

```rust
// md:fn list_resources_for_note_orders_and_excludes_others
#[tokio::test]
async fn list_resources_for_note_orders_and_excludes_others() {
    let backend = in_memory_backend().await;
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

**What it does** — `list_resources_for_note` returns exactly the target note's attachments in
`created_at` order, excluding another note's attachments and system-sentinel resources.

**Dependencies** —
- `list_resources_for_note` — native filtered query; expects `WHERE note_id = ? ORDER BY
  created_at, id` and that a real note id never equals `SYSTEM_RESOURCE_NOTE_ID`.

**Used by** — n/a (integration test).

**Repeated context** — pagination/order key is `(created_at, id)`; per-note listings exclude
the system sentinel.

---

## fn resource_note_id_round_trips

**Identification** — `#[tokio::test]`. Marker `// md:fn resource_note_id_round_trips`.

**Code** — complete and verbatim:

```rust
// md:fn resource_note_id_round_trips
#[tokio::test]
async fn resource_note_id_round_trips() {
    let backend = in_memory_backend().await;
    let note = backend.create_note(Note::new("N", "")).await.unwrap();
    let created = backend
        .create_resource(
            Resource::new(note.id, "a", "text/plain", "a.txt", 1),
            vec![1],
        )
        .await
        .unwrap();
    assert_eq!(created.note_id, note.id);
    let (read, _) = backend.read_resource(created.id).await.unwrap();
    assert_eq!(read.note_id, note.id, "note_id survives create + read");
}
```

**What it does** — `note_id` survives the full `create_resource` → `read_resource` round trip
through the SQLite column and `row_to_resource`.

**Dependencies** —
- `create_resource`/`read_resource` — persist and read `note_id`; expect the INSERT column and
  the `row_to_resource` mapping to stay in sync.

**Used by** — n/a (integration test).

**Repeated context** — `note_id` is plaintext, stored as `TEXT` like every UUID in the schema.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

<!-- Data source: CI artifact or local graphify-out/graph.json from this exact commit.
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. Never
     present inference as fact. -->
**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `in_memory_backend()` — defined here (EXTRACTED; the shared fixture)
- the 37 `#[tokio::test]` functions — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/db/mod.rs` — defines `DbBackend` (INFERRED: the test reaches it through the fully-qualified `keeplin_core::storage::db::DbBackend`, which the AST pass does not link)
- `keeplin-core/src/models.rs` — entities and `Change` (EXTRACTED: references)
- `keeplin-core/src/error.rs` — `StorageError` (EXTRACTED: references)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- The full `StorageBackend` surface of `DbBackend` is covered at the SQLite level; WebSocket paths belong to `ws_sync.rs`.
- The journal invariants (original op type recorded; `apply_change` never re-journals; prune respects the cutoff) are pinned here.
- Tombstone-first races (#71) must stay covered for all four versioned entity types.
- Concurrency tests must run on a multi-threaded runtime to exercise real interleavings.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `fn in_memory_backend` | `// md:fn in_memory_backend` |
| 3 | `fn create_and_read_note` | `// md:fn create_and_read_note` |
| 4 | `fn update_note` | `// md:fn update_note` |
| 5 | `fn delete_note_soft_deletes` | `// md:fn delete_note_soft_deletes` |
| 6 | `fn list_notes_excludes_deleted` | `// md:fn list_notes_excludes_deleted` |
| 7 | `fn read_nonexistent_returns_not_found` | `// md:fn read_nonexistent_returns_not_found` |
| 8 | `fn device_id_is_stable` | `// md:fn device_id_is_stable` |
| 9 | `fn sync_state_round_trips` | `// md:fn sync_state_round_trips` |
| 10 | `fn get_changes_since_returns_updated_notes` | `// md:fn get_changes_since_returns_updated_notes` |
| 11 | `fn prune_change_journal_removes_rows_older_than_cutoff` | `// md:fn prune_change_journal_removes_rows_older_than_cutoff` |
| 12 | `fn apply_change_is_not_re_journaled` | `// md:fn apply_change_is_not_re_journaled` |
| 13 | `fn update_nonexistent_note_returns_not_found` | `// md:fn update_nonexistent_note_returns_not_found` |
| 14 | `fn delete_nonexistent_note_returns_not_found` | `// md:fn delete_nonexistent_note_returns_not_found` |
| 15 | `fn update_nonexistent_notebook_returns_not_found` | `// md:fn update_nonexistent_notebook_returns_not_found` |
| 16 | `fn delete_nonexistent_notebook_returns_not_found` | `// md:fn delete_nonexistent_notebook_returns_not_found` |
| 17 | `fn update_nonexistent_tag_returns_not_found` | `// md:fn update_nonexistent_tag_returns_not_found` |
| 18 | `fn delete_nonexistent_tag_returns_not_found` | `// md:fn delete_nonexistent_tag_returns_not_found` |
| 19 | `fn create_and_read_notebook` | `// md:fn create_and_read_notebook` |
| 20 | `fn delete_notebook_soft_deletes` | `// md:fn delete_notebook_soft_deletes` |
| 21 | `fn create_and_read_tag` | `// md:fn create_and_read_tag` |
| 22 | `fn add_and_list_note_tags` | `// md:fn add_and_list_note_tags` |
| 23 | `fn add_note_tag_rejects_missing_or_deleted_ends` | `// md:fn add_note_tag_rejects_missing_or_deleted_ends` |
| 24 | `fn pagination_walks_notes_sharing_a_created_at` | `// md:fn pagination_walks_notes_sharing_a_created_at` |
| 25 | `fn remove_note_tag` | `// md:fn remove_note_tag` |
| 26 | `fn purge_reclaims_old_tombstoned_payloads_only` | `// md:fn purge_reclaims_old_tombstoned_payloads_only` |
| 27 | `fn create_and_read_resource` | `// md:fn create_and_read_resource` |
| 28 | `fn list_resources_excludes_data` | `// md:fn list_resources_excludes_data` |
| 29 | `fn delete_resource` | `// md:fn delete_resource` |
| 30 | `fn list_notes_paginates_without_duplicates_or_gaps` | `// md:fn list_notes_paginates_without_duplicates_or_gaps` |
| 31 | `fn concurrent_note_creates_all_succeed` | `// md:fn concurrent_note_creates_all_succeed` |
| 32 | `fn concurrent_reads_and_writes_make_progress` | `// md:fn concurrent_reads_and_writes_make_progress` |
| 33 | `fn note_alias_bookmarks_links_round_trip` | `// md:fn note_alias_bookmarks_links_round_trip` |
| 34 | `fn notebook_alias_round_trip` | `// md:fn notebook_alias_round_trip` |
| 35 | `fn indexed_backlinks_track_writes_and_deletes` | `// md:fn indexed_backlinks_track_writes_and_deletes` |
| 36 | `fn backlinks_are_paginated` | `// md:fn backlinks_are_paginated` |
| 37 | `fn ordering_fields_round_trip_and_manual_order_query` | `// md:fn ordering_fields_round_trip_and_manual_order_query` |
| 38 | `fn sync_applied_change_carries_ordering_fields` | `// md:fn sync_applied_change_carries_ordering_fields` |
| 39 | `fn delete_for_unknown_entity_leaves_a_tombstone_blocking_a_stale_create` | `// md:fn delete_for_unknown_entity_leaves_a_tombstone_blocking_a_stale_create` |
| 40 | `fn note_delete_cascades_to_attachments_and_restore_recovers_dragged` | `// md:fn note_delete_cascades_to_attachments_and_restore_recovers_dragged` |
| 41 | `fn moving_note_between_notebooks_leaves_attachments_untouched` | `// md:fn moving_note_between_notebooks_leaves_attachments_untouched` |
| 42 | `fn list_resources_for_note_orders_and_excludes_others` | `// md:fn list_resources_for_note_orders_and_excludes_others` |
| 43 | `fn resource_note_id_round_trips` | `// md:fn resource_note_id_round_trips` |