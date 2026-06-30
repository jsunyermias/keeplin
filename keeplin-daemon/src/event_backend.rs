//! Change-publishing [`StorageBackend`] decorator for the live WebSocket feed.
//!
//! [`EventBackend<B>`] wraps any `B: StorageBackend` and, after every **successful**
//! mutation, publishes the corresponding [`Change`] to a [`tokio::sync::broadcast`]
//! channel. Read operations delegate to the inner backend unchanged. Because it is a
//! `StorageBackend` itself (it implements all five sub-traits, like
//! [`keeplin_core::encryption::EncryptedBackend`]), a single `EventBackend` instance can
//! sit behind **both** the gRPC service and the REST API — so a mutation from either
//! surface emits exactly one event.
//!
//! # Placement in the decorator stack
//!
//! `EventBackend` is wrapped **outside** any `EncryptedBackend`, i.e.
//! `EventBackend<EncryptedBackend<FsBackend>>`. Its create/update methods publish the
//! value **returned** by the inner backend, which `EncryptedBackend` has already
//! decrypted — so WebSocket subscribers receive plaintext changes. The daemon is the
//! trust boundary; at-rest encryption protects the disk, not connected API clients.
//!
//! # Delivery semantics
//!
//! The broadcast channel is **lossy and best-effort**: a subscriber that falls behind
//! the channel capacity sees a `Lagged` error rather than blocking writers. The feed is
//! a notification stream, not a durable log — the authoritative history is the
//! per-device change journal used by sync. Publishing never blocks a mutation: a send to
//! a channel with no live receivers simply returns an error that is ignored.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::broadcast;
use uuid::Uuid;

use keeplin_core::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
    storage::{NoteRepository, NotebookRepository, ResourceRepository, SyncBackend, TagRepository},
};

/// Wraps a [`StorageBackend`] and broadcasts a [`Change`] after each successful mutation.
pub struct EventBackend<B> {
    /// The underlying backend that actually persists data. Every operation delegates here
    /// first; events are only published once the inner call succeeds.
    inner: B,
    /// The broadcast sender that mutations are published to. The daemon keeps another clone
    /// of this same channel in the REST `AppState`, from which each WebSocket connection
    /// derives its own receiver.
    tx: broadcast::Sender<Change>,
}

impl<B> EventBackend<B> {
    /// Wraps `inner`, publishing changes to `tx`.
    ///
    /// `tx` is created once in `main` (`broadcast::channel(capacity)`); pass a clone here
    /// and keep another clone around (the daemon stores it in the REST `AppState`) so the
    /// WebSocket route can hand each connection a fresh `tx.subscribe()` receiver.
    pub fn new(inner: B, tx: broadcast::Sender<Change>) -> Self {
        Self { inner, tx }
    }

    /// Publishes one change, ignoring the "no active receivers" error.
    ///
    /// `broadcast::Sender::send` only fails when there are zero live receivers, which is
    /// the normal state when no WebSocket clients are connected. That is not an error
    /// condition for a mutation, so the result is intentionally discarded.
    fn publish(&self, change: Change) {
        let _ = self.tx.send(change);
    }
}

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
        // Delegate so an inner indexed backend (e.g. DbBackend) is reached.
        self.inner
            .note_backlinks(target_id, page_size, page_token)
            .await
    }
}

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
        self.publish(Change::NoteTagAdd { note_id, tag_id });
        Ok(())
    }

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        self.inner.remove_note_tag(note_id, tag_id).await?;
        self.publish(Change::NoteTagRemove { note_id, tag_id });
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

#[async_trait]
impl<B: ResourceRepository> ResourceRepository for EventBackend<B> {
    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let stored = self.inner.create_resource(resource, data).await?;
        // The feed carries metadata only (`data: None`); subscribers fetch the bytes via
        // `GET /api/resources/:id/data`. This keeps the broadcast channel lightweight and
        // matches `FsBackend`, which also omits payloads from its change journal.
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
        self.publish(Change::ResourceDelete { id });
        Ok(())
    }

    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        self.inner.list_resources(page_size, page_token).await
    }
}

// Synchronisation methods carry no user-visible mutation of their own — they move changes
// that have already been (or will be) published by the CRUD methods above — so they
// delegate to the inner backend without emitting anything onto the feed.
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

#[cfg(test)]
mod tests {
    use super::*;
    use keeplin_core::storage::fs::FsBackend;
    use tokio::sync::broadcast::error::TryRecvError;

    async fn backend() -> (EventBackend<FsBackend>, broadcast::Receiver<Change>) {
        let dir = tempfile::tempdir().unwrap();
        let fs = FsBackend::new(dir.path()).await.unwrap();
        let (tx, rx) = broadcast::channel(16);
        // Keep the tempdir alive for the duration of the test by leaking it; the OS
        // reclaims it when the test process exits.
        std::mem::forget(dir);
        (EventBackend::new(fs, tx), rx)
    }

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

    #[tokio::test]
    async fn reads_do_not_emit_changes() {
        let (be, mut rx) = backend().await;
        let stored = be.create_note(Note::new("t", "b")).await.unwrap();
        // Drain the create event.
        let _ = rx.try_recv().unwrap();

        be.read_note(stored.id).await.unwrap();
        be.list_notes(10, None).await.unwrap();
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[tokio::test]
    async fn failed_mutation_emits_nothing() {
        let (be, mut rx) = backend().await;
        // Updating a note that does not exist must fail and publish no event.
        let ghost = Note::new("t", "b");
        assert!(be.update_note(ghost).await.is_err());
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }
}
