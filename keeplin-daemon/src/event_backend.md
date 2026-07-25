# `event_backend.rs` — `EventBackend` change-publishing decorator

Self-contained companion for `keeplin-daemon/src/event_backend.rs`. It documents
**every code block of the source file, in source order** — a reader with only this
file must be able to understand it without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::broadcast;
use uuid::Uuid;

use keeplin_core::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
    storage::{NoteRepository, NotebookRepository, ResourceRepository, SyncBackend, TagRepository},
};
```

**What it does** — `EventBackend<B>` wraps any `B: StorageBackend` and, after
every **successful** mutation, publishes the corresponding `Change` to a
`tokio::sync::broadcast` channel — the live WebSocket feed. Reads delegate
unchanged. Because it is itself a `StorageBackend`, one instance sits behind
**both** the gRPC service and the REST API, so a mutation from either surface
emits exactly one event. Placement: **outside** any `EncryptedBackend`
(`EventBackend<EncryptedBackend<Fs|Db>>`) — it publishes the value *returned*
by the inner backend, already decrypted, so subscribers receive plaintext; the
daemon is the trust boundary (at-rest encryption protects the disk, not
connected clients). Delivery is **lossy, best-effort**: a lagging subscriber
sees `Lagged` rather than blocking writers; the feed is a notification stream,
not a durable log — the authoritative history is the sync change journal.
Publishing never blocks a mutation.

**Dependencies** — `tokio::sync::broadcast`, `async_trait`, `chrono`, `uuid`,
keeplin-core's error/model/storage-trait types.

**Used by** — the daemon's stack assembly (`main.rs`), with a `tx` clone in the
REST `AppState` from which each WebSocket connection derives its receiver.

**Repeated context** — decorator conventions restated: implement every
sub-trait, delegate defaulted trait methods (`note_backlinks`) explicitly so
inner indexes are reached, publish only after the inner call succeeds.

---

## EventBackend

**Identification** — `pub struct EventBackend<B>`; marker `// md:EventBackend`.

**Code** — complete and verbatim:

```rust
// md:EventBackend
pub struct EventBackend<B> {
    inner: B,
    tx: broadcast::Sender<Change>,
}
```

**What it does** — `inner: B` (persists first; events only after success) and
`tx: broadcast::Sender<Change>` (the daemon keeps another clone in the REST
`AppState`).

**Dependencies** — `broadcast`. **Used by** — `main.rs`.
**Repeated context** — none.

---

## impl EventBackend

**Identification** — inherent impl; marker `// md:impl EventBackend`. Two
methods.

**Code** — container: members documented as sub-blocks below: fn new, fn publish.

### fn new

**Identification** — `pub fn new(inner: B, tx: broadcast::Sender<Change>) -> Self`;
marker `// md:impl EventBackend > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl EventBackend > fn new
    pub fn new(inner: B, tx: broadcast::Sender<Change>) -> Self {
        Self { inner, tx }
    }
```

**What it does** — Wraps `inner`. `tx` is created once in `main`
(`broadcast::channel(capacity)`); pass a clone here and keep another for the
WebSocket route's `tx.subscribe()` calls.

### fn publish

**Identification** — `fn publish(&self, change: Change)`; marker
`// md:impl EventBackend > fn publish`.

**Code** — complete and verbatim:

```rust
    // md:impl EventBackend > fn publish
    fn publish(&self, change: Change) {
        let _ = self.tx.send(change);
    }
```

**What it does** — Sends one change, discarding the only possible error —
"no active receivers", the normal state when no WebSocket client is
connected.

---

## impl NoteRepository for EventBackend

**Identification** — marker `// md:impl NoteRepository for EventBackend`.

**Code** — complete and verbatim:

```rust
// md:impl NoteRepository for EventBackend
#[async_trait]
impl<B: NoteRepository> NoteRepository for EventBackend<B> {
    async fn create_note(&self, note: Note) -> Result<Note, StorageError> {
        let stored = self.inner.create_note(note).await?;
        self.publish(Change::NoteCreate {
            note: stored.clone(),
        });
        Ok(stored)
    }

    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        self.inner.read_note(id).await
    }

    async fn update_note(&self, note: Note) -> Result<Note, StorageError> {
        let stored = self.inner.update_note(note).await?;
        self.publish(Change::NoteUpdate {
            note: stored.clone(),
        });
        Ok(stored)
    }

    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_note(id).await?;
        self.publish(Change::NoteDelete {
            id,
            deleted_at: Utc::now(),
            vv: Default::default(),
            last_writer: String::new(),
        });
        Ok(())
    }

    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        self.inner.list_notes(page_size, page_token).await
    }

    async fn note_backlinks(
        &self,
        target_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        self.inner
            .note_backlinks(target_id, page_size, page_token)
            .await
    }

    async fn list_notes_in_notebook(
        &self,
        notebook_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        self.inner
            .list_notes_in_notebook(notebook_id, page_size, page_token)
            .await
    }

    async fn list_starred_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        self.inner.list_starred_notes(page_size, page_token).await
    }

    async fn notebook_sort_profile(
        &self,
        notebook_id: Uuid,
    ) -> Result<keeplin_core::storage::NotebookSortProfile, StorageError> {
        self.inner.notebook_sort_profile(notebook_id).await
    }
}
```

**What it does** — `create_note`/`update_note` delegate then publish
`NoteCreate`/`NoteUpdate` with the **stored** (returned, decrypted) copy;
`delete_note` publishes a `NoteDelete` with empty vv/writer — the feed is a
best-effort notification (clients reload via REST/gRPC), so
conflict-resolution metadata is not needed; `read_note`, the listings,
`note_backlinks` (explicit delegation for inner indexes), and
`notebook_sort_profile` delegate silently.

**Dependencies** — `publish`, the inner backend.

**Used by** — all note traffic in the daemon.

**Repeated context** — publish-after-success: a failed mutation emits nothing.

---

## impl NotebookRepository for EventBackend

**Identification** — marker `// md:impl NotebookRepository for EventBackend`.

**Code** — complete and verbatim:

```rust
// md:impl NotebookRepository for EventBackend
#[async_trait]
impl<B: NotebookRepository> NotebookRepository for EventBackend<B> {
    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        let stored = self.inner.create_notebook(notebook).await?;
        self.publish(Change::NotebookCreate {
            notebook: stored.clone(),
        });
        Ok(stored)
    }

    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        self.inner.read_notebook(id).await
    }

    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        let stored = self.inner.update_notebook(notebook).await?;
        self.publish(Change::NotebookUpdate {
            notebook: stored.clone(),
        });
        Ok(stored)
    }

    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_notebook(id).await?;
        self.publish(Change::NotebookDelete {
            id,
            deleted_at: Utc::now(),
            vv: Default::default(),
            last_writer: String::new(),
        });
        Ok(())
    }

    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        self.inner.list_notebooks(page_size, page_token).await
    }
}
```

**What it does** — Same pattern for notebooks: create/update publish the
stored copy, delete publishes an empty-metadata tombstone, reads/listings
silent.

---

## impl TagRepository for EventBackend

**Identification** — marker `// md:impl TagRepository for EventBackend`.

**Code** — complete and verbatim:

```rust
// md:impl TagRepository for EventBackend
#[async_trait]
impl<B: TagRepository> TagRepository for EventBackend<B> {
    async fn create_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        let stored = self.inner.create_tag(tag).await?;
        self.publish(Change::TagCreate {
            tag: stored.clone(),
        });
        Ok(stored)
    }

    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        self.inner.read_tag(id).await
    }

    async fn update_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        let stored = self.inner.update_tag(tag).await?;
        self.publish(Change::TagUpdate {
            tag: stored.clone(),
        });
        Ok(stored)
    }

    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_tag(id).await?;
        self.publish(Change::TagDelete {
            id,
            deleted_at: Utc::now(),
            vv: Default::default(),
            last_writer: String::new(),
        });
        Ok(())
    }

    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        self.inner.list_tags(page_size, page_token).await
    }

    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        let (note_id, tag_id) = (note_tag.note_id, note_tag.tag_id);
        self.inner.add_note_tag(note_tag).await?;
        self.publish(Change::NoteTagAdd {
            note_id,
            tag_id,
            updated_at: chrono::Utc::now(),
            vv: Default::default(),
            last_writer: String::new(),
        });
        Ok(())
    }

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        self.inner.remove_note_tag(note_id, tag_id).await?;
        self.publish(Change::NoteTagRemove {
            note_id,
            tag_id,
            updated_at: chrono::Utc::now(),
            vv: Default::default(),
            last_writer: String::new(),
        });
        Ok(())
    }

    async fn list_note_tags(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        self.inner
            .list_note_tags(note_id, page_size, page_token)
            .await
    }
}
```

**What it does** — Same pattern for tags; `add_note_tag`/`remove_note_tag`
publish `NoteTagAdd`/`NoteTagRemove` with fresh timestamps and empty version
metadata (notification only); `list_note_tags` silent.

---

## impl ResourceRepository for EventBackend

**Identification** — marker `// md:impl ResourceRepository for EventBackend`.

**Code** — complete and verbatim:

```rust
// md:impl ResourceRepository for EventBackend
#[async_trait]
impl<B: ResourceRepository> ResourceRepository for EventBackend<B> {
    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let stored = self.inner.create_resource(resource, data).await?;
        self.publish(Change::ResourceCreate {
            resource: stored.clone(),
            data: None,
        });
        Ok(stored)
    }

    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        self.inner.read_resource(id).await
    }

    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_resource(id).await?;
        self.publish(Change::ResourceDelete {
            id,
            deleted_at: chrono::Utc::now(),
            vv: Default::default(),
            last_writer: String::new(),
        });
        Ok(())
    }

    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        self.inner.list_resources(page_size, page_token).await
    }

    async fn purge_deleted_resources(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, StorageError> {
        self.inner.purge_deleted_resources(older_than).await
    }
}
```

**What it does** — `create_resource` publishes `ResourceCreate` with
**`data: None`** — the feed carries metadata only; subscribers fetch bytes via
`GET /api/resources/:id/data` (keeps the channel light, matches `FsBackend`'s
journal); `delete_resource` publishes the tombstone;
`purge_deleted_resources` delegates silently (maintenance — the deletions were
published when they happened); reads/listings silent.

---

## impl SyncBackend for EventBackend

**Identification** — marker `// md:impl SyncBackend for EventBackend`.

**Code** — complete and verbatim:

```rust
// md:impl SyncBackend for EventBackend
#[async_trait]
impl<B: SyncBackend> SyncBackend for EventBackend<B> {
    async fn get_device_id(&self) -> Result<String, StorageError> {
        self.inner.get_device_id().await
    }

    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
        self.inner.get_last_sync_time().await
    }

    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        self.inner.update_sync_time(ts).await
    }

    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError> {
        self.inner.get_changes_since(since).await
    }

    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        self.inner.apply_change(change).await
    }

    async fn send_changes(&self, changes: Vec<Change>) -> Result<(), StorageError> {
        self.inner.send_changes(changes).await
    }

    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        self.inner.receive_changes().await
    }

    async fn prune_change_journal(&self, older_than: DateTime<Utc>) -> Result<u64, StorageError> {
        self.inner.prune_change_journal(older_than).await
    }
}
```

**What it does** — All eight methods delegate without publishing: sync moves
changes that were already (or will be) published by the CRUD methods —
emitting here would duplicate events.

---

## impl HistoryRepository for EventBackend

**Identification** — marker `// md:impl HistoryRepository for EventBackend`.

**Code** — complete and verbatim:

```rust
// md:impl HistoryRepository for EventBackend
#[async_trait]
impl<B: keeplin_core::storage::HistoryRepository> keeplin_core::storage::HistoryRepository
    for EventBackend<B>
{
    async fn note_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<keeplin_core::storage::EntityVersion<Note>>, StorageError> {
        self.inner.note_history(id, limit).await
    }

    async fn notebook_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<keeplin_core::storage::EntityVersion<Notebook>>, StorageError> {
        self.inner.notebook_history(id, limit).await
    }
}
```

**What it does** — Pure delegation of `note_history`/`notebook_history`.

---

## mod tests

**Identification** — `#[cfg(test)] mod tests`; marker `// md:mod tests`. One
helper + three tests over `EventBackend<FsBackend>`.

**Code** — container: members documented as sub-blocks below: fn backend, fn create_update_delete_emit_changes, fn reads_do_not_emit_changes, fn failed_mutation_emits_nothing.

**What it does** — Pins publish-on-success, silence on reads, and silence on
failure.

The explicit `imports` leaf below preserves the test-module dependency preamble
verbatim.

### imports

**Identification** — test-module dependencies; marker `// md:mod tests > imports`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > imports
    use super::*;
    use keeplin_core::storage::fs::FsBackend;
    use tokio::sync::broadcast::error::TryRecvError;
```

**What it does** — Brings the parent module API and test-only dependencies into
scope.

**Dependencies** —

- `super::*` — the items under test from the parent module; expects: the parent keeps them at module scope; a rename or a move into a submodule breaks these tests at compile time, which is the intended early signal.
- `keeplin_core::storage::fs::FsBackend` — a real filesystem-backed store built over a temporary directory; expects: it honours the same repository traits as production, so what passes here says something about the real backend.
- `tokio::sync::broadcast::error::TryRecvError` — distinguishes empty from lagged when draining the channel without blocking; expects: the variants keep their meaning: `Empty` is 'nothing yet', `Lagged` is 'events were dropped' — collapsing them would hide real loss.

**Used by** — every block of `mod tests` in this file: `fn backend`, `fn create_update_delete_emit_changes`, `fn reads_do_not_emit_changes`, `fn failed_mutation_emits_nothing`. Nothing outside the module can use it: the preamble is private to `mod tests`.

**Repeated context** — This preamble is a leaf block, not scaffolding: only the `mod` declaration, its attributes and its braces are exempt from coverage, so these `use` lines carry their own marker and are verified verbatim against the source (template v2.5.0, RULE 6). Changing an import here without updating this fence fails `scripts/check-docs.sh`.

### fn backend

**Identification** — helper; marker `// md:mod tests > fn backend`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn backend
    async fn backend() -> (EventBackend<FsBackend>, broadcast::Receiver<Change>) {
        let dir = tempfile::tempdir().unwrap();
        let fs = FsBackend::new(dir.path()).await.unwrap();
        let (tx, rx) = broadcast::channel(16);
        std::mem::forget(dir);
        (EventBackend::new(fs, tx), rx)
    }
```

**What it does** — An `EventBackend<FsBackend>` over a leaked tempdir plus the
matching receiver (capacity 16).

### fn create_update_delete_emit_changes

**Identification** — tokio test; marker
`// md:mod tests > fn create_update_delete_emit_changes`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn create_update_delete_emit_changes
    #[tokio::test]
    async fn create_update_delete_emit_changes() {
        let (be, mut rx) = backend().await;

        let note = Note::new("title", "body");
        let id = note.id;
        let stored = be.create_note(note).await.unwrap();
        match rx.try_recv().unwrap() {
            Change::NoteCreate { note } => assert_eq!(note.id, stored.id),
            other => panic!("expected NoteCreate, got {other:?}"),
        }

        let mut edited = stored.clone();
        edited.title = "new".into();
        be.update_note(edited).await.unwrap();
        assert!(matches!(rx.try_recv().unwrap(), Change::NoteUpdate { .. }));

        be.delete_note(id).await.unwrap();
        match rx.try_recv().unwrap() {
            Change::NoteDelete { id: deleted, .. } => assert_eq!(deleted, id),
            other => panic!("expected NoteDelete, got {other:?}"),
        }
    }
```

**What it does** — Create/update/delete each emit their variant, with the
delete carrying the right id.

### fn reads_do_not_emit_changes

**Identification** — tokio test; marker
`// md:mod tests > fn reads_do_not_emit_changes`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn reads_do_not_emit_changes
    #[tokio::test]
    async fn reads_do_not_emit_changes() {
        let (be, mut rx) = backend().await;
        let stored = be.create_note(Note::new("t", "b")).await.unwrap();
        let _ = rx.try_recv().unwrap();

        be.read_note(stored.id).await.unwrap();
        be.list_notes(10, None).await.unwrap();
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }
```

**What it does** — After draining the create event, `read_note` and
`list_notes` leave the channel `Empty`.

### fn failed_mutation_emits_nothing

**Identification** — tokio test; marker
`// md:mod tests > fn failed_mutation_emits_nothing`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn failed_mutation_emits_nothing
    #[tokio::test]
    async fn failed_mutation_emits_nothing() {
        let (be, mut rx) = backend().await;
        let ghost = Note::new("t", "b");
        assert!(be.update_note(ghost).await.is_err());
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }
```

**What it does** — Updating a nonexistent note fails and publishes no event.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `EventBackend<B>` — defined here (EXTRACTED)
- the six trait implementations (implements×6) (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs`, `models.rs`, `storage/backend.rs` (EXTRACTED: references×37/×30/×3)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/main.rs` — stack assembly (INFERRED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `EventBackend` | `// md:EventBackend` |
| 3 | `impl EventBackend` (container) | `// md:impl EventBackend` |
| 4 | `fn new` | `// md:impl EventBackend > fn new` |
| 5 | `fn publish` | `// md:impl EventBackend > fn publish` |
| 6 | `impl NoteRepository for EventBackend` | `// md:impl NoteRepository for EventBackend` |
| 7 | `impl NotebookRepository for EventBackend` | `// md:impl NotebookRepository for EventBackend` |
| 8 | `impl TagRepository for EventBackend` | `// md:impl TagRepository for EventBackend` |
| 9 | `impl ResourceRepository for EventBackend` | `// md:impl ResourceRepository for EventBackend` |
| 10 | `impl SyncBackend for EventBackend` | `// md:impl SyncBackend for EventBackend` |
| 11 | `impl HistoryRepository for EventBackend` | `// md:impl HistoryRepository for EventBackend` |
| 12 | `mod tests` (container) | `// md:mod tests` |
| 13 | `imports` | `// md:mod tests > imports` |
| 14 | `fn backend` | `// md:mod tests > fn backend` |
| 15 | `fn create_update_delete_emit_changes` | `// md:mod tests > fn create_update_delete_emit_changes` |
| 16 | `fn reads_do_not_emit_changes` | `// md:mod tests > fn reads_do_not_emit_changes` |
| 17 | `fn failed_mutation_emits_nothing` | `// md:mod tests > fn failed_mutation_emits_nothing` |
