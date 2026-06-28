use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
};

#[async_trait]
pub trait StorageBackend: Send + Sync + 'static {
    // ── Notes ────────────────────────────────────────────────────────────────

    async fn create_note(&self, note: Note) -> Result<Note, StorageError>;
    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError>;
    async fn update_note(&self, note: Note) -> Result<Note, StorageError>;
    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError>;
    async fn list_notes(&self) -> Result<Vec<Note>, StorageError>;

    // ── Notebooks ─────────────────────────────────────────────────────────────

    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError>;
    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError>;
    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError>;
    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError>;
    async fn list_notebooks(&self) -> Result<Vec<Notebook>, StorageError>;

    // ── Tags ──────────────────────────────────────────────────────────────────

    async fn create_tag(&self, tag: Tag) -> Result<Tag, StorageError>;
    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError>;
    async fn update_tag(&self, tag: Tag) -> Result<Tag, StorageError>;
    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError>;
    async fn list_tags(&self) -> Result<Vec<Tag>, StorageError>;

    // ── Note–Tag relations ────────────────────────────────────────────────────

    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError>;
    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError>;
    async fn list_note_tags(&self, note_id: Uuid) -> Result<Vec<Tag>, StorageError>;

    // ── Resources ─────────────────────────────────────────────────────────────

    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError>;
    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError>;
    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError>;
    async fn list_resources(&self) -> Result<Vec<Resource>, StorageError>;

    // ── Synchronisation ───────────────────────────────────────────────────────

    /// Return all changes that happened after `since`.
    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError>;

    /// Apply a single incoming change locally.
    async fn apply_change(&self, change: Change) -> Result<(), StorageError>;

    /// Read the persisted last-sync timestamp (epoch start if never synced).
    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError>;

    /// Persist a new last-sync timestamp.
    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError>;

    /// Send local changes to the remote peer.
    async fn send_changes(&self, changes: Vec<Change>) -> Result<(), StorageError>;

    /// Receive incoming changes from the remote peer.
    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError>;

    /// Return the stable identifier for this device.
    async fn get_device_id(&self) -> Result<String, StorageError>;
}
