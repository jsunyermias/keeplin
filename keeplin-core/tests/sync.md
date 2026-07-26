# `tests/sync.rs` — cross-device change-propagation tests

Self-contained companion for `keeplin-core/tests/sync.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file
must be able to understand it without opening anything else, so project-wide
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

use chrono::{Duration, Utc};
use keeplin_core::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag, SYSTEM_RESOURCE_NOTE_ID},
    storage::{
        db::DbBackend, fs::FsBackend, NoteRepository, NotebookRepository, ResourceRepository,
        SyncBackend, TagRepository,
    },
};
use tempfile::tempdir;
```

**What it does** — Integration tests modelling **two independent devices**, each
backed by its own `DbBackend` database file, verifying that changes recorded on
one device can be collected with `SyncBackend::get_changes_since` and replayed
on the other with `SyncBackend::apply_change` to reach a convergent state.
Version-vector conflict semantics are pinned here — including the concurrent
equal-timestamp case that bare `updated_at` last-write-wins would diverge on —
plus the tombstone rules that stop a stale edit from resurrecting a delete (and
vice versa). A couple of cases run on `FsBackend` to confirm it resolves the
same way through the shared `note_log::resolve`/`merge` primitives.

**Repeated context** — conflict resolution is unified on version vectors: the
`(timestamp, device_id)` pair breaks ties deterministically, so both sides of a
concurrent edit pick the same winner. `apply_change` does not re-journal, so
each device's journal holds only its own local edits — the exchange loops in
these tests rely on that. `FsBackend` resolves note conflicts through per-note
version-vector logs rather than wire `Change::NoteUpdate` records, so its LWW
equivalent lives in `fs_two_device_concurrent_edits_converge` in
`tests/fs_backend.rs`, not here.

---

## fn device

**Identification** — `async fn device() -> DbBackend`. Marker `// md:fn device`.

**Code** — complete and verbatim:

```rust
// md:fn device
async fn device() -> DbBackend {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(db_path, "", "").await.unwrap()
}
```

**What it does** — A standalone offline `DbBackend` on a temp `.db` file (empty
server URL/token → no WebSocket). The temp dir is leaked (`std::mem::forget`) so
it outlives the open connection for the test's duration.

**Used by** — every `db_*` test and the first two.

---

## fn create_propagates_between_devices

**Identification** — `#[tokio::test]`. Marker
`// md:fn create_propagates_between_devices`.

**Code** — complete and verbatim:

```rust
// md:fn create_propagates_between_devices
#[tokio::test]
async fn create_propagates_between_devices() {
    let a = device().await;
    let b = device().await;

    let note = Note::new("Shared", "from A");
    let id = note.id;
    a.create_note(note).await.unwrap();

    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let changes = a.get_changes_since(epoch).await.unwrap();
    assert!(!changes.is_empty(), "device A must have recorded a change");
    for c in changes {
        b.apply_change(c).await.unwrap();
    }

    let read = b.read_note(id).await.unwrap();
    assert_eq!(read.title, "Shared");
    assert_eq!(read.body, "from A");
}
```

**What it does** — A creates a note; collecting all of A's changes since epoch
and applying them on B makes B read the same title/body back.

---

## fn stale_remote_update_does_not_clobber_newer_local

**Identification** — `#[tokio::test]`. Marker
`// md:fn stale_remote_update_does_not_clobber_newer_local`.

**Code** — complete and verbatim:

```rust
// md:fn stale_remote_update_does_not_clobber_newer_local
#[tokio::test]
async fn stale_remote_update_does_not_clobber_newer_local() {
    let local = device().await;

    let mut note = Note::new("Title", "current local body");
    let id = note.id;
    note.updated_at = Utc::now();
    local.create_note(note.clone()).await.unwrap();

    let mut stale = note.clone();
    stale.body = "stale remote body".to_string();
    stale.updated_at = Utc::now() - Duration::minutes(1);

    local
        .apply_change(Change::NoteUpdate { note: stale })
        .await
        .unwrap();

    let read = local.read_note(id).await.unwrap();
    assert_eq!(
        read.body, "current local body",
        "a stale remote update must not overwrite a newer local edit"
    );
}
```

**What it does** — Applying a remote `Change::NoteUpdate` whose `updated_at` is
a minute older than the local record is a no-op: the newer local body survives
(last-write-wins by timestamp).

---

## fn db_stale_delete_does_not_override_newer_edit

**Identification** — `#[tokio::test]`. Marker
`// md:fn db_stale_delete_does_not_override_newer_edit`.

**Code** — complete and verbatim:

```rust
// md:fn db_stale_delete_does_not_override_newer_edit
#[tokio::test]
async fn db_stale_delete_does_not_override_newer_edit() {
    let local = device().await;
    let mut note = Note::new("Title", "current body");
    let id = note.id;
    note.updated_at = Utc::now();
    local.create_note(note).await.unwrap();

    local
        .apply_change(Change::NoteDelete {
            id,
            deleted_at: Utc::now() - Duration::minutes(1),
            vv: Default::default(),
            last_writer: String::new(),
        })
        .await
        .unwrap();

    let read = local.read_note(id).await.unwrap();
    assert!(
        read.deleted_at.is_none(),
        "a stale delete must not tombstone a newer note"
    );
}
```

**What it does** — A `Change::NoteDelete` dated a minute before the local edit
must not tombstone the newer note (`deleted_at` stays `None`).

---

## fn db_stale_update_does_not_resurrect_tombstone

**Identification** — `#[tokio::test]`. Marker
`// md:fn db_stale_update_does_not_resurrect_tombstone`.

**Code** — complete and verbatim:

```rust
// md:fn db_stale_update_does_not_resurrect_tombstone
#[tokio::test]
async fn db_stale_update_does_not_resurrect_tombstone() {
    let local = device().await;
    let mut note = Note::new("Title", "original body");
    let id = note.id;
    note.updated_at = Utc::now() - Duration::minutes(5);
    local.create_note(note.clone()).await.unwrap();

    local.delete_note(id).await.unwrap();

    let mut stale = note.clone();
    stale.body = "resurrected?".to_string();
    local
        .apply_change(Change::NoteUpdate { note: stale })
        .await
        .unwrap();

    let read = local.read_note(id).await.unwrap();
    assert!(
        read.deleted_at.is_some(),
        "a stale update must not resurrect a tombstoned note"
    );
}
```

**What it does** — After a local delete (tombstone dated "now"), a stale peer
update from before the delete must not revive the note (`deleted_at` stays
set).

---

## fn db_concurrent_equal_timestamp_edits_converge

**Identification** — `#[tokio::test]`. Marker
`// md:fn db_concurrent_equal_timestamp_edits_converge`.

**Code** — complete and verbatim:

```rust
// md:fn db_concurrent_equal_timestamp_edits_converge
#[tokio::test]
async fn db_concurrent_equal_timestamp_edits_converge() {
    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let a = device().await;
    let b = device().await;

    let base = a.create_note(Note::new("t", "base")).await.unwrap();
    let id = base.id;
    for c in a.get_changes_since(epoch).await.unwrap() {
        b.apply_change(c).await.unwrap();
    }

    let t = Utc::now();
    let mut ea = base.clone();
    ea.body = "from A".to_string();
    ea.updated_at = t;
    a.update_note(ea).await.unwrap();

    let mut eb = b.read_note(id).await.unwrap();
    eb.body = "from B".to_string();
    eb.updated_at = t;
    b.update_note(eb).await.unwrap();

    let a_changes = a.get_changes_since(epoch).await.unwrap();
    let b_changes = b.get_changes_since(epoch).await.unwrap();
    for c in b_changes {
        a.apply_change(c).await.unwrap();
    }
    for c in a_changes {
        b.apply_change(c).await.unwrap();
    }

    let body_a = a.read_note(id).await.unwrap().body;
    let body_b = b.read_note(id).await.unwrap().body;
    assert_eq!(
        body_a, body_b,
        "concurrent equal-timestamp edits must converge"
    );
    assert!(body_a == "from A" || body_a == "from B");
}
```

**What it does** — The divergence case bare LWW cannot handle: from a shared
baseline (created on A, replicated to B), both devices edit the same note with
the **identical** `updated_at`, then exchange changes both ways. Under strict
`>` comparison each device would keep its own edit — permanent divergence. The
version vector's `(timestamp, device_id)` tiebreak makes both devices pick the
same body (either "from A" or "from B", but the **same** on both).

---

## fn db_concurrent_notebook_edits_converge

**Identification** — `#[tokio::test]`. Marker
`// md:fn db_concurrent_notebook_edits_converge`.

**Code** — complete and verbatim:

```rust
// md:fn db_concurrent_notebook_edits_converge
#[tokio::test]
async fn db_concurrent_notebook_edits_converge() {
    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let a = device().await;
    let b = device().await;

    let base = a.create_notebook(Notebook::new("base")).await.unwrap();
    let id = base.id;
    for c in a.get_changes_since(epoch).await.unwrap() {
        b.apply_change(c).await.unwrap();
    }

    let t = Utc::now();
    let mut ea = base.clone();
    ea.title = "from A".to_string();
    ea.updated_at = t;
    a.update_notebook(ea).await.unwrap();

    let mut eb = b.read_notebook(id).await.unwrap();
    eb.title = "from B".to_string();
    eb.updated_at = t;
    b.update_notebook(eb).await.unwrap();

    let a_changes = a.get_changes_since(epoch).await.unwrap();
    let b_changes = b.get_changes_since(epoch).await.unwrap();
    for c in b_changes {
        a.apply_change(c).await.unwrap();
    }
    for c in a_changes {
        b.apply_change(c).await.unwrap();
    }

    let title_a = a.read_notebook(id).await.unwrap().title;
    let title_b = b.read_notebook(id).await.unwrap().title;
    assert_eq!(title_a, title_b, "concurrent notebook edits must converge");
    assert!(title_a == "from A" || title_a == "from B");
}
```

**What it does** — The same equal-timestamp convergence for **notebooks**,
exercising the `DbBackend` notebook `apply_change` arm specifically: concurrent
same-`updated_at` title edits converge to one deterministic winner.

---

## fn fs_tombstones_resolve_by_timestamp

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_tombstones_resolve_by_timestamp`.

**Code** — complete and verbatim:

```rust
// md:fn fs_tombstones_resolve_by_timestamp
#[tokio::test]
async fn fs_tombstones_resolve_by_timestamp() {
    let dir = tempdir().unwrap();
    let backend = FsBackend::new(dir.path()).await.unwrap();

    let mut a = Note::new("A", "body");
    let a_id = a.id;
    a.updated_at = Utc::now();
    backend.create_note(a).await.unwrap();
    backend
        .apply_change(Change::NoteDelete {
            id: a_id,
            deleted_at: Utc::now() - Duration::minutes(1),
            vv: Default::default(),
            last_writer: String::new(),
        })
        .await
        .unwrap();
    assert!(backend.read_note(a_id).await.unwrap().deleted_at.is_none());

    let mut b = Note::new("B", "body");
    let b_id = b.id;
    b.updated_at = Utc::now() - Duration::minutes(5);
    backend.create_note(b.clone()).await.unwrap();
    backend.delete_note(b_id).await.unwrap();
    let mut stale = b.clone();
    stale.body = "resurrected?".to_string();
    backend
        .apply_change(Change::NoteUpdate { note: stale })
        .await
        .unwrap();
    assert!(backend.read_note(b_id).await.unwrap().deleted_at.is_some());
}
```

**What it does** — Tombstone semantics hold on `FsBackend` too: (a) a stale
`NoteDelete` (a minute older than the note) does not tombstone it; (b) a stale
`NoteUpdate` cannot resurrect a newer local delete.

---

## fn db_concurrent_note_tag_add_remove_converges

**Identification** — `#[tokio::test]`. Marker
`// md:fn db_concurrent_note_tag_add_remove_converges`.

**Code** — complete and verbatim:

```rust
// md:fn db_concurrent_note_tag_add_remove_converges
#[tokio::test]
async fn db_concurrent_note_tag_add_remove_converges() {
    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let a = device().await;
    let b = device().await;

    let note = a.create_note(Note::new("n", "")).await.unwrap();
    let tag = a.create_tag(Tag::new("t")).await.unwrap();
    a.add_note_tag(NoteTag {
        note_id: note.id,
        tag_id: tag.id,
    })
    .await
    .unwrap();
    for c in a.get_changes_since(epoch).await.unwrap() {
        b.apply_change(c).await.unwrap();
    }
    assert_eq!(a.list_note_tags(note.id, 0, None).await.unwrap().0.len(), 1);
    assert_eq!(b.list_note_tags(note.id, 0, None).await.unwrap().0.len(), 1);

    a.remove_note_tag(note.id, tag.id).await.unwrap();
    b.add_note_tag(NoteTag {
        note_id: note.id,
        tag_id: tag.id,
    })
    .await
    .unwrap();

    let a_changes = a.get_changes_since(epoch).await.unwrap();
    let b_changes = b.get_changes_since(epoch).await.unwrap();
    for c in b_changes {
        a.apply_change(c).await.unwrap();
    }
    for c in a_changes {
        b.apply_change(c).await.unwrap();
    }

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
    assert_eq!(present_a, present_b, "concurrent add/remove must converge");
}
```

**What it does** — From a shared baseline with the tag attached, A detaches
while B re-attaches concurrently; after exchanging changes both ways, both
devices agree on the association's final presence. Before Phase 3, associations
carried no version (add = INSERT OR IGNORE, remove = DELETE), so the outcome
was order-dependent and could differ between devices.

---

## fn db_resource_delete_propagates_and_converges

**Identification** — `#[tokio::test]`. Marker
`// md:fn db_resource_delete_propagates_and_converges`.

**Code** — complete and verbatim:

```rust
// md:fn db_resource_delete_propagates_and_converges
#[tokio::test]
async fn db_resource_delete_propagates_and_converges() {
    let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    let a = device().await;
    let b = device().await;

    let res = Resource::new(SYSTEM_RESOURCE_NOTE_ID, "f", "text/plain", "f.txt", 3);
    let id = res.id;
    a.create_resource(res, b"abc".to_vec()).await.unwrap();
    for c in a.get_changes_since(epoch).await.unwrap() {
        b.apply_change(c).await.unwrap();
    }
    assert!(b.read_resource(id).await.is_ok(), "create must propagate");

    a.delete_resource(id).await.unwrap();
    for c in a.get_changes_since(epoch).await.unwrap() {
        b.apply_change(c).await.unwrap();
    }

    for backend in [&a, &b] {
        assert!(
            matches!(
                backend.read_resource(id).await,
                Err(StorageError::NotFound(_))
            ),
            "a soft-deleted resource reads as NotFound"
        );
        assert!(
            backend.list_resources(0, None).await.unwrap().0.is_empty(),
            "a soft-deleted resource is excluded from listings"
        );
    }
}
```

**What it does** — A resource create syncs A→B; the origin then soft-deletes
(a versioned tombstone) and the tombstone propagates: both devices read
`StorageError::NotFound` and exclude the resource from listings — instead of
the old order-dependent hard delete.

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

- `device()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `create_propagates_between_devices()` — defined here (EXTRACTED; file-local)
- `stale_remote_update_does_not_clobber_newer_local()` — defined here (EXTRACTED; file-local)
- `db_stale_delete_does_not_override_newer_edit()` — defined here (EXTRACTED; file-local)
- `db_stale_update_does_not_resurrect_tombstone()` — defined here (EXTRACTED; file-local)
- `db_concurrent_equal_timestamp_edits_converge()` — defined here (EXTRACTED; file-local)
- `db_concurrent_notebook_edits_converge()` — defined here (EXTRACTED; file-local)
- `fs_tombstones_resolve_by_timestamp()` — defined here (EXTRACTED; file-local)
- `db_concurrent_note_tag_add_remove_converges()` — defined here (EXTRACTED; file-local)
- `db_resource_delete_propagates_and_converges()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: references×1; e.g. `DbBackend`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Two independent backends converge by exchanging `Change`s via `get_changes_since`/`apply_change` — version-vector semantics (dominance, concurrency tiebreak) are pinned here.
- Convergence must be order-independent: applying the same change sets in different orders yields the same final state.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `fn device` | `// md:fn device` |
| 3 | `fn create_propagates_between_devices` | `// md:fn create_propagates_between_devices` |
| 4 | `fn stale_remote_update_does_not_clobber_newer_local` | `// md:fn stale_remote_update_does_not_clobber_newer_local` |
| 5 | `fn db_stale_delete_does_not_override_newer_edit` | `// md:fn db_stale_delete_does_not_override_newer_edit` |
| 6 | `fn db_stale_update_does_not_resurrect_tombstone` | `// md:fn db_stale_update_does_not_resurrect_tombstone` |
| 7 | `fn db_concurrent_equal_timestamp_edits_converge` | `// md:fn db_concurrent_equal_timestamp_edits_converge` |
| 8 | `fn db_concurrent_notebook_edits_converge` | `// md:fn db_concurrent_notebook_edits_converge` |
| 9 | `fn fs_tombstones_resolve_by_timestamp` | `// md:fn fs_tombstones_resolve_by_timestamp` |
| 10 | `fn db_concurrent_note_tag_add_remove_converges` | `// md:fn db_concurrent_note_tag_add_remove_converges` |
| 11 | `fn db_resource_delete_propagates_and_converges` | `// md:fn db_resource_delete_propagates_and_converges` |