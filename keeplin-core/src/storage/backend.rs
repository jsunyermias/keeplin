//! The `StorageBackend` supertrait and its five focused sub-traits.
//!
//! Rather than exposing a single 30-method trait, the storage layer is split into
//! five cohesive interfaces — [`NoteRepository`], [`NotebookRepository`],
//! [`TagRepository`], [`ResourceRepository`], and [`SyncBackend`] — each covering
//! one domain of responsibility. [`StorageBackend`] is then a supertrait that requires
//! all five, giving call-sites a single bound while keeping each domain independently
//! testable and mockable.
//!
//! A blanket impl automatically satisfies [`StorageBackend`] for any type that
//! implements all five sub-traits, so adding a new backend only requires writing the
//! five focused `impl` blocks — no additional glue code is needed.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
};

// ── NoteRepository ────────────────────────────────────────────────────────────

/// CRUD operations for [`Note`] entities.
///
/// Implementations must treat `delete_note` as a **soft delete**: the note's
/// `deleted_at` field is set to the current time and the record is retained in
/// storage. `list_notes` must exclude soft-deleted notes from its results.
#[async_trait]
pub trait NoteRepository: Send + Sync + 'static {
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

    /// Returns a page of notes that have not been soft-deleted, ordered by
    /// `(created_at ASC, id ASC)`.
    ///
    /// `page_size = 0` uses the backend default of 100. `page_token = None` starts
    /// from the beginning. The returned `Option<String>` is the opaque cursor for the
    /// next page; `None` means there are no further pages.
    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError>;
}

// ── NotebookRepository ────────────────────────────────────────────────────────

/// CRUD operations for [`Notebook`] entities.
///
/// The same soft-delete semantics as [`NoteRepository`] apply: `delete_notebook`
/// sets `deleted_at` rather than removing the record, and `list_notebooks` omits
/// soft-deleted notebooks.
#[async_trait]
pub trait NotebookRepository: Send + Sync + 'static {
    /// Persists a new notebook and returns the stored copy.
    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError>;

    /// Fetches a notebook by its UUID. Returns [`StorageError::NotFound`] if absent.
    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError>;

    /// Overwrites a notebook's fields. Returns [`StorageError::NotFound`] if absent.
    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError>;

    /// Soft-deletes a notebook. Returns [`StorageError::NotFound`] if absent.
    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError>;

    /// Returns a page of notebooks that have not been soft-deleted, ordered by
    /// `(created_at ASC, id ASC)`. Pagination semantics match [`NoteRepository::list_notes`].
    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError>;
}

// ── TagRepository ─────────────────────────────────────────────────────────────

/// CRUD operations for [`Tag`] entities and the note–tag association table.
///
/// Note–tag links (`add_note_tag`, `remove_note_tag`) are included here rather
/// than in a separate trait because they are always used together with tag reads
/// and the association has no independent lifecycle beyond the tags themselves.
///
/// Both `add_note_tag` and `remove_note_tag` must be **idempotent**: adding a tag
/// that is already attached, or removing one that is not attached, must succeed
/// without returning an error.
#[async_trait]
pub trait TagRepository: Send + Sync + 'static {
    /// Persists a new tag and returns the stored copy.
    async fn create_tag(&self, tag: Tag) -> Result<Tag, StorageError>;

    /// Fetches a tag by its UUID. Returns [`StorageError::NotFound`] if absent.
    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError>;

    /// Overwrites a tag's fields. Returns [`StorageError::NotFound`] if absent.
    async fn update_tag(&self, tag: Tag) -> Result<Tag, StorageError>;

    /// Soft-deletes a tag. Returns [`StorageError::NotFound`] if absent.
    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError>;

    /// Returns a page of tags that have not been soft-deleted, ordered by
    /// `(created_at ASC, id ASC)`. Pagination semantics match [`NoteRepository::list_notes`].
    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError>;

    /// Attaches `note_tag.tag_id` to `note_tag.note_id`.
    ///
    /// Must be idempotent: attaching a tag that is already attached must not
    /// return an error.
    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError>;

    /// Detaches the tag identified by `tag_id` from the note identified by `note_id`.
    ///
    /// Returns successfully even if the association did not exist (idempotent).
    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError>;

    /// Returns a page of tags currently attached to the note identified by `note_id`,
    /// ordered by `(created_at ASC, id ASC)`. Pagination semantics match
    /// [`NoteRepository::list_notes`].
    async fn list_note_tags(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError>;
}

// ── ResourceRepository ────────────────────────────────────────────────────────

/// CRUD operations for binary [`Resource`] attachments.
///
/// Resources use **hard delete** (data removed immediately) rather than soft
/// delete. Binary payloads can be large and there is no business requirement to
/// retain deleted attachment data.
#[async_trait]
pub trait ResourceRepository: Send + Sync + 'static {
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

    /// Returns a page of resource metadata records, without their binary payloads,
    /// ordered by `(created_at ASC, id ASC)`. Pagination semantics match
    /// [`NoteRepository::list_notes`]. To read the binary payload for a specific
    /// resource, call `read_resource` with that resource's UUID.
    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError>;
}

// ── SyncBackend ───────────────────────────────────────────────────────────────

/// Device identification and change-journal synchronisation operations.
///
/// All methods in this trait are used by [`crate::sync::SyncEngine`] to coordinate
/// state between devices. The six-step sync cycle is:
/// 1. [`get_changes_since`](Self::get_changes_since) — collect local changes since last sync.
/// 2. [`send_changes`](Self::send_changes) — push them to the remote peer.
/// 3. [`receive_changes`](Self::receive_changes) — pull changes from the remote peer.
/// 4. [`apply_change`](Self::apply_change) (repeated) — apply each incoming change locally.
/// 5. [`update_sync_time`](Self::update_sync_time) — record the completion timestamp.
/// 6. [`prune_change_journal`](Self::prune_change_journal) (optional) — trim old journal rows.
///
/// ## Idempotency requirement
///
/// `apply_change` **must** be idempotent: applying the same `Change` twice must produce
/// the same result as applying it once. All built-in implementations satisfy this by
/// using `INSERT OR IGNORE` / `INSERT OR REPLACE` for creates and no-op deletes.
#[async_trait]
pub trait SyncBackend: Send + Sync + 'static {
    /// Returns the stable string identifier for this device installation.
    ///
    /// The device ID is generated once and persisted to disk. It is used as the Argon2id
    /// salt when deriving the AES encryption key, and as the file name for this device's
    /// change log (`logs/{device_id}.log`).
    async fn get_device_id(&self) -> Result<String, StorageError>;

    /// Returns the UTC timestamp of the most recent successful sync cycle.
    ///
    /// Returns the Unix epoch (1970-01-01T00:00:00Z) if no sync has ever completed
    /// on this device.
    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError>;

    /// Overwrites the stored last-sync timestamp with `ts`.
    ///
    /// Called at the end of a successful sync cycle by [`crate::sync::SyncEngine`].
    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError>;

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

// ── StorageBackend supertrait ─────────────────────────────────────────────────

/// Unified async storage interface.
///
/// This supertrait requires all five domain-specific sub-traits:
/// [`NoteRepository`], [`NotebookRepository`], [`TagRepository`],
/// [`ResourceRepository`], and [`SyncBackend`]. Any type that implements all five
/// automatically satisfies `StorageBackend` via the blanket impl below — no
/// additional code is required.
///
/// Code that works with any backend uses `T: StorageBackend` as a single bound.
/// Methods from all five sub-traits are available on `T` because supertrait
/// bounds are transitive.
///
/// ## Implemented by
///
/// - [`crate::storage::fs::FsBackend`] — JSON files + Syncthing replication.
/// - [`crate::storage::db::DbBackend`] — LibSQL database + WebSocket sync.
/// - [`crate::encryption::EncryptedBackend<B>`] — transparent AES-256-GCM decorator.
pub trait StorageBackend:
    NoteRepository + NotebookRepository + TagRepository + ResourceRepository + SyncBackend
{
}

/// Blanket implementation: any type satisfying all five sub-traits automatically
/// satisfies `StorageBackend`. This means adding a new backend only requires
/// writing the five focused `impl` blocks — no additional glue code is needed.
impl<T> StorageBackend for T where
    T: NoteRepository + NotebookRepository + TagRepository + ResourceRepository + SyncBackend
{
}
