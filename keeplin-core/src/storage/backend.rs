//! The `StorageBackend` trait — the single contract that every storage implementation
//! must satisfy.
//!
//! Programming against this trait rather than a concrete type means the daemon, the
//! sync engine, and the encryption layer all remain independent of which storage
//! mechanism is in use at runtime.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
};

/// Async interface for all CRUD and synchronisation operations.
///
/// Every method is `async` (via [`async_trait`]) and returns
/// `Result<T, `[`StorageError`]`>`. The `Send + Sync + 'static` bounds ensure the
/// trait can be held behind an `Arc`, passed into `tokio::spawn`, and stored in a
/// `tonic` server struct.
///
/// ## Idempotency requirement
///
/// `apply_change` **must** be idempotent: applying the same `Change` twice must produce
/// the same result as applying it once. All built-in implementations satisfy this by
/// using `INSERT OR IGNORE` / `INSERT OR REPLACE` for creates and no-op deletes.
#[async_trait]
pub trait StorageBackend: Send + Sync + 'static {
    // ── Notes ────────────────────────────────────────────────────────────────

    /// Persists a new note and returns the stored copy.
    ///
    /// The returned `Note` may differ from the input if the backend sets extra fields
    /// (e.g. `EncryptedBackend` returns the decrypted copy after storing the
    /// encrypted one).
    async fn create_note(&self, note: Note) -> Result<Note, StorageError>;

    /// Fetches a note by its UUID.
    ///
    /// Returns [`StorageError::NotFound`] if no note with the given `id` exists or if
    /// the note has been soft-deleted.
    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError>;

    /// Overwrites all fields of an existing note and returns the updated copy.
    ///
    /// Returns [`StorageError::NotFound`] if no note with the same `id` is stored.
    async fn update_note(&self, note: Note) -> Result<Note, StorageError>;

    /// Soft-deletes a note by setting its `deleted_at` timestamp to now.
    ///
    /// Returns [`StorageError::NotFound`] if no note with the given `id` exists.
    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError>;

    /// Returns all notes that have not been soft-deleted, in an unspecified order.
    async fn list_notes(&self) -> Result<Vec<Note>, StorageError>;

    // ── Notebooks ─────────────────────────────────────────────────────────────

    /// Persists a new notebook and returns the stored copy.
    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError>;

    /// Fetches a notebook by its UUID. Returns [`StorageError::NotFound`] if absent.
    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError>;

    /// Overwrites a notebook's fields. Returns [`StorageError::NotFound`] if absent.
    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError>;

    /// Soft-deletes a notebook. Returns [`StorageError::NotFound`] if absent.
    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError>;

    /// Returns all notebooks that have not been soft-deleted.
    async fn list_notebooks(&self) -> Result<Vec<Notebook>, StorageError>;

    // ── Tags ──────────────────────────────────────────────────────────────────

    /// Persists a new tag and returns the stored copy.
    async fn create_tag(&self, tag: Tag) -> Result<Tag, StorageError>;

    /// Fetches a tag by its UUID. Returns [`StorageError::NotFound`] if absent.
    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError>;

    /// Overwrites a tag's fields. Returns [`StorageError::NotFound`] if absent.
    async fn update_tag(&self, tag: Tag) -> Result<Tag, StorageError>;

    /// Soft-deletes a tag. Returns [`StorageError::NotFound`] if absent.
    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError>;

    /// Returns all tags that have not been soft-deleted.
    async fn list_tags(&self) -> Result<Vec<Tag>, StorageError>;

    // ── Note–Tag relations ────────────────────────────────────────────────────

    /// Attaches `note_tag.tag_id` to `note_tag.note_id`.
    ///
    /// Implementations must be idempotent: attaching a tag that is already attached
    /// must not return an error.
    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError>;

    /// Detaches the tag identified by `tag_id` from the note identified by `note_id`.
    ///
    /// Returns successfully even if the association did not exist (idempotent).
    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError>;

    /// Returns all tags currently attached to the note identified by `note_id`.
    async fn list_note_tags(&self, note_id: Uuid) -> Result<Vec<Tag>, StorageError>;

    // ── Resources ─────────────────────────────────────────────────────────────

    /// Stores resource metadata alongside its binary payload and returns the metadata.
    ///
    /// `data` is the raw binary content of the file (e.g. PNG bytes, PDF bytes).
    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError>;

    /// Returns both the metadata and the binary payload for a resource.
    ///
    /// Returns [`StorageError::NotFound`] if no resource with the given `id` exists.
    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError>;

    /// Permanently removes a resource and its binary payload (hard delete).
    ///
    /// Returns [`StorageError::NotFound`] if no resource with the given `id` exists.
    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError>;

    /// Returns metadata for all resources, without their binary payloads.
    ///
    /// To read the binary payload, call `read_resource` with the individual resource's
    /// UUID.
    async fn list_resources(&self) -> Result<Vec<Resource>, StorageError>;

    // ── Synchronisation ───────────────────────────────────────────────────────

    /// Returns all [`Change`] events recorded on this device after `since`.
    ///
    /// The `since` parameter is typically the value returned by the previous call to
    /// [`get_last_sync_time`](Self::get_last_sync_time). The returned list is ordered
    /// by the time each change was recorded.
    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError>;

    /// Applies a single incoming change from another device to the local store.
    ///
    /// Must be idempotent: applying the same change twice produces the same result as
    /// applying it once. This allows the sync engine to safely retry after a partial
    /// failure without risking data corruption.
    async fn apply_change(&self, change: Change) -> Result<(), StorageError>;

    /// Returns the UTC timestamp of the most recent successful sync cycle.
    ///
    /// Returns the Unix epoch (1970-01-01T00:00:00Z) if no sync has ever completed
    /// on this device.
    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError>;

    /// Overwrites the stored last-sync timestamp with `ts`.
    ///
    /// Called at the end of a successful sync cycle by [`crate::sync::SyncEngine`].
    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError>;

    /// Transmits the given list of local changes to the remote peer.
    ///
    /// In `DbBackend`, this sends the changes over the WebSocket connection with
    /// exponential-backoff retry. In `FsBackend`, this is a no-op because Syncthing
    /// replicates the log files independently.
    async fn send_changes(&self, changes: Vec<Change>) -> Result<(), StorageError>;

    /// Retrieves all changes that the remote peer has for this device since the last pull.
    ///
    /// In `DbBackend`, this drains available messages from the WebSocket connection.
    /// In `FsBackend`, this is a no-op that returns an empty list (changes are discovered
    /// by scanning other devices' log files in `get_changes_since`).
    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError>;

    /// Returns the stable string identifier for this device installation.
    ///
    /// The device ID is generated once and persisted to disk. It is used as the Argon2id
    /// salt when deriving the AES encryption key, and as the file name for this device's
    /// change log (`logs/{device_id}.log`).
    async fn get_device_id(&self) -> Result<String, StorageError>;

    /// Permanently removes change-journal entries older than `older_than`.
    ///
    /// Returns the number of rows removed. Call this periodically (for example once per
    /// day after a successful sync cycle) to prevent the `entity_changes` table in
    /// `DbBackend` from growing indefinitely.
    ///
    /// `FsBackend` always returns `Ok(0)` without modifying any files, because pruning
    /// per-device NDJSON log files could cause remote devices that have not yet processed
    /// the removed entries to miss changes permanently.
    async fn prune_change_journal(&self, older_than: DateTime<Utc>) -> Result<u64, StorageError>;
}
