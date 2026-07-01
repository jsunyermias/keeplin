//! Domain data types shared across all storage backends and the sync engine.
//!
//! Every type in this module derives `serde::{Serialize, Deserialize}` so it can be
//! persisted as JSON (log files, SQLite TEXT columns) and transmitted over the network
//! without a separate conversion layer.
//!
//! The [`Change`] enum is the fundamental unit of synchronisation: every mutation
//! produces one `Change` that can be replayed on another device to reach the same state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::links::{Bookmark, NoteLink};
use crate::storage::note_log::VersionVector;

/// Generates a new random UUID version 4.
///
/// All entity constructors (`Note::new`, `Notebook::new`, etc.) call this function to
/// assign a unique identifier. Callers must never generate IDs themselves so that ID
/// generation remains consistent and testable.
pub fn new_id() -> Uuid {
    Uuid::new_v4()
}

/// Returns the current UTC timestamp.
///
/// Used by entity constructors to set `created_at` / `updated_at` and by the sync
/// engine to record the completion time of each sync cycle.
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// A user-created note.
///
/// Notes are the primary content unit in Keeplin. They may optionally belong to a
/// [`Notebook`] (via `notebook_id`) and may be flagged as to-do items with an optional
/// due date and completion timestamp.
///
/// Soft deletion is used: instead of removing the row, `deleted_at` is set to the
/// current UTC time. The note is then excluded from `list_notes` results but remains
/// visible when read directly by ID (useful for sync conflict resolution).
///
/// `title` and `body` are encrypted at rest when an [`crate::encryption::EncryptedBackend`]
/// is in use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Note {
    /// Stable, globally unique identifier (UUID v4). Generated once at creation and
    /// never changed, even across renames or moves.
    pub id: Uuid,
    /// User-visible title. May be an empty string. Encrypted at rest.
    pub title: String,
    /// Full text content of the note. May be multi-line. Encrypted at rest.
    pub body: String,
    /// UUID of the parent notebook, or `None` if the note has not been placed in any
    /// notebook. Stored in plaintext because it is needed for database queries.
    pub notebook_id: Option<Uuid>,
    /// Whether this note is being used as a to-do item.
    pub is_todo: bool,
    /// Optional deadline for the to-do. `None` when the note has no due date.
    pub todo_due: Option<DateTime<Utc>>,
    /// Timestamp when the to-do was marked as completed. `None` while still open.
    pub todo_completed: Option<DateTime<Utc>>,
    /// UTC timestamp set once when the note is first created. Never modified.
    pub created_at: DateTime<Utc>,
    /// UTC timestamp updated on every mutation of the note's fields.
    pub updated_at: DateTime<Utc>,
    /// UTC timestamp set when the note is soft-deleted. `None` means the note is active.
    pub deleted_at: Option<DateTime<Utc>>,
    /// Optional human-readable alias, unique among live notes. Lets links target the note as
    /// `#<alias>` instead of `#<uuid>`. Encrypted at rest. Defaults to `None`.
    #[serde(default)]
    pub alias: Option<String>,
    /// Bookmarks (in-note anchors) derived from `[text](### "alias")` markdown links in the
    /// body. Maintained by [`crate::linking::LinkingBackend`]. Defaults to empty.
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    /// Links to other notes: content-derived (markdown `#` links) and manually added.
    /// Maintained by [`crate::linking::LinkingBackend`]. Defaults to empty.
    #[serde(default)]
    pub links: Vec<NoteLink>,
    /// Version vector for conflict resolution — per-device edit counters. A local write
    /// increments this device's component; on sync, [`crate::storage::note_log::resolve`]
    /// compares vectors so concurrent edits converge deterministically (rather than a bare
    /// `updated_at` last-write-wins). Empty on records written before VV tracking. Plaintext
    /// (sync metadata, not user content), so it is not encrypted at rest. Defaults to empty.
    #[serde(default)]
    pub vv: VersionVector,
    /// Device id that authored the current value, used as the concurrent tiebreak alongside
    /// `updated_at`. Empty on pre-VV records. Plaintext. Defaults to empty.
    #[serde(default)]
    pub last_writer: String,
}

impl Note {
    /// Creates a new note with a fresh UUID, the given title and body, and the current
    /// UTC time for both `created_at` and `updated_at`. All optional fields are
    /// initialised to `None` / `false` / empty.
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        let ts = now();
        Self {
            id: new_id(),
            title: title.into(),
            body: body.into(),
            notebook_id: None,
            is_todo: false,
            todo_due: None,
            todo_completed: None,
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
            alias: None,
            bookmarks: Vec::new(),
            links: Vec::new(),
            vv: VersionVector::new(),
            last_writer: String::new(),
        }
    }
}

/// A named collection that groups notes together.
///
/// Like [`Note`], notebooks use soft deletion: `deleted_at` is set instead of removing
/// the row. The `title` field is encrypted at rest when an
/// [`crate::encryption::EncryptedBackend`] is in use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Notebook {
    /// Stable, globally unique identifier (UUID v4).
    pub id: Uuid,
    /// User-visible name. Encrypted at rest.
    pub title: String,
    /// UTC timestamp set once at creation.
    pub created_at: DateTime<Utc>,
    /// UTC timestamp updated on every rename.
    pub updated_at: DateTime<Utc>,
    /// UTC timestamp set on soft-delete. `None` means the notebook is active.
    pub deleted_at: Option<DateTime<Utc>>,
    /// Optional human-readable alias, unique among live notebooks. Lets links scope a note as
    /// `#<notebook alias>#<note>`. Encrypted at rest. Defaults to `None`.
    #[serde(default)]
    pub alias: Option<String>,
    /// Version vector for conflict resolution (see [`Note::vv`]). Plaintext. Defaults to empty.
    #[serde(default)]
    pub vv: VersionVector,
    /// Device id that authored the current value; concurrent tiebreak (see [`Note::last_writer`]).
    #[serde(default)]
    pub last_writer: String,
}

impl Notebook {
    /// Creates a new notebook with a fresh UUID, the given title, and the current UTC
    /// time for both timestamps. `deleted_at` and `alias` are `None`.
    pub fn new(title: impl Into<String>) -> Self {
        let ts = now();
        Self {
            id: new_id(),
            title: title.into(),
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
            alias: None,
            vv: VersionVector::new(),
            last_writer: String::new(),
        }
    }
}

/// A short label that can be attached to any number of notes.
///
/// Tag–note associations are stored in the separate [`NoteTag`] type. Like notebooks,
/// tags use soft deletion and their `title` is encrypted at rest when an
/// [`crate::encryption::EncryptedBackend`] is in use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Tag {
    /// Stable, globally unique identifier (UUID v4).
    pub id: Uuid,
    /// User-visible label text. Encrypted at rest.
    pub title: String,
    /// UTC timestamp set once at creation.
    pub created_at: DateTime<Utc>,
    /// UTC timestamp updated on every rename.
    pub updated_at: DateTime<Utc>,
    /// UTC timestamp set on soft-delete. `None` means the tag is active.
    pub deleted_at: Option<DateTime<Utc>>,
    /// Version vector for conflict resolution (see [`Note::vv`]). Plaintext. Defaults to empty.
    #[serde(default)]
    pub vv: VersionVector,
    /// Device id that authored the current value; concurrent tiebreak (see [`Note::last_writer`]).
    #[serde(default)]
    pub last_writer: String,
}

impl Tag {
    /// Creates a new tag with a fresh UUID, the given title, and the current UTC time
    /// for both timestamps. `deleted_at` is `None`.
    pub fn new(title: impl Into<String>) -> Self {
        let ts = now();
        Self {
            id: new_id(),
            title: title.into(),
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
            vv: VersionVector::new(),
            last_writer: String::new(),
        }
    }
}

/// A many-to-many association between one note and one tag.
///
/// `NoteTag` records are created by [`crate::storage::StorageBackend::add_note_tag`] and
/// removed by `remove_note_tag`. There is no soft-delete: the association is simply
/// deleted when the tag is detached.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct NoteTag {
    /// The UUID of the note that the tag is attached to.
    pub note_id: Uuid,
    /// The UUID of the tag that is attached to the note.
    pub tag_id: Uuid,
}

/// Metadata for a binary file attachment.
///
/// The binary payload is stored separately from the metadata (in a `data` file on disk
/// for `FsBackend`, or in a BLOB column for `DbBackend`). `list_resources` returns
/// only the metadata; the binary is fetched explicitly via `read_resource`.
///
/// `title`, `mime_type`, and `file_name` are encrypted at rest when an
/// [`crate::encryption::EncryptedBackend`] is in use.
/// `size` is stored in plaintext because it is needed to validate uploads without
/// decrypting the payload first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Resource {
    /// Stable, globally unique identifier (UUID v4).
    pub id: Uuid,
    /// User-visible name for the attachment. Encrypted at rest.
    pub title: String,
    /// IANA media type (e.g. `"image/png"`, `"application/pdf"`). Encrypted at rest.
    pub mime_type: String,
    /// Original file name as provided by the client (e.g. `"photo.jpg"`). Encrypted at rest.
    pub file_name: String,
    /// Size of the binary payload in bytes. Stored in plaintext.
    pub size: u64,
    /// UTC timestamp set once at creation.
    pub created_at: DateTime<Utc>,
}

impl Resource {
    /// Creates a new resource metadata record with a fresh UUID and the current UTC time.
    ///
    /// The binary payload is **not** stored here; it is passed separately to
    /// [`crate::storage::StorageBackend::create_resource`].
    ///
    /// # Parameters
    /// - `title` — user-visible name for the attachment
    /// - `mime_type` — IANA media type string (e.g. `"image/png"`)
    /// - `file_name` — original file name (e.g. `"photo.jpg"`)
    /// - `size` — exact byte length of the binary payload
    pub fn new(
        title: impl Into<String>,
        mime_type: impl Into<String>,
        file_name: impl Into<String>,
        size: u64,
    ) -> Self {
        Self {
            id: new_id(),
            title: title.into(),
            mime_type: mime_type.into(),
            file_name: file_name.into(),
            size,
            created_at: now(),
        }
    }
}

/// One unit of change that can be applied to a local store to bring it in sync with
/// another device.
///
/// Every mutating `StorageBackend` operation produces a `Change` that is appended to
/// the change journal (`entity_changes` table in `DbBackend`, NDJSON log file in
/// `FsBackend`). During a sync cycle, the local journal is read, changes are sent to
/// the remote peer, remote changes are received, and each is applied via
/// [`crate::storage::StorageBackend::apply_change`].
///
/// # Serialisation
///
/// `Change` is serialised as a JSON object with a `"op"` field that carries the
/// variant name in snake_case (e.g. `{"op":"note_create","note":{...}}`).
/// The `NoteCreate`, `NoteUpdate`, and `NoteDelete` variants accept the v1 short tags
/// (`"create"`, `"update"`, `"delete"`) via `#[serde(alias)]` to remain compatible
/// with log files written by earlier versions of the daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    /// A new note was created on the originating device.
    /// The `#[serde(alias = "create")]` allows v1 log files that used `"op":"create"`
    /// to be read correctly without any data migration.
    #[serde(alias = "create")]
    NoteCreate { note: Note },
    /// An existing note's fields were modified on the originating device.
    /// The `#[serde(alias = "update")]` preserves v1 log compatibility.
    #[serde(alias = "update")]
    NoteUpdate { note: Note },
    /// A note was soft-deleted on the originating device.
    ///
    /// `id` is the UUID of the deleted note and `deleted_at` is when the deletion
    /// happened. The tombstone also carries the deleting write's version vector (`vv`) and
    /// author (`last_writer`), so it competes in [`crate::storage::note_log::resolve`] exactly
    /// like an edit: a stale edit can never resurrect a newer delete, a stale delete can never
    /// override a newer edit, and a causal edit made *after* seeing the delete revives the
    /// note. `#[serde(default)]` on `vv`/`last_writer` and `#[serde(default = "now")]` on
    /// `deleted_at` keep older records readable, and `#[serde(alias = "delete")]` preserves the
    /// v1 op tag.
    #[serde(alias = "delete")]
    NoteDelete {
        id: Uuid,
        #[serde(default = "now")]
        deleted_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
    /// A new notebook was created on the originating device.
    NotebookCreate { notebook: Notebook },
    /// An existing notebook's title was changed on the originating device.
    NotebookUpdate { notebook: Notebook },
    /// A notebook was soft-deleted on the originating device. Carries the tombstone timestamp
    /// plus the deleting write's `vv`/`last_writer` for `resolve` (see [`Change::NoteDelete`]).
    NotebookDelete {
        id: Uuid,
        #[serde(default = "now")]
        deleted_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
    /// A new tag was created on the originating device.
    TagCreate { tag: Tag },
    /// An existing tag's title was changed on the originating device.
    TagUpdate { tag: Tag },
    /// A tag was soft-deleted on the originating device. Carries the tombstone timestamp plus
    /// the deleting write's `vv`/`last_writer` for `resolve` (see [`Change::NoteDelete`]).
    TagDelete {
        id: Uuid,
        #[serde(default = "now")]
        deleted_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
    /// A tag was attached to a note on the originating device.
    NoteTagAdd { note_id: Uuid, tag_id: Uuid },
    /// A tag was detached from a note on the originating device.
    NoteTagRemove { note_id: Uuid, tag_id: Uuid },
    /// A resource was created on the originating device.
    ///
    /// `data` carries the binary payload when syncing through `DbBackend` (where there
    /// is no shared filesystem and the receiving device has no other way to obtain the
    /// bytes). In `FsBackend`, `data` is always `None` because Syncthing replicates the
    /// `resources/{id}/data` file independently.
    ///
    /// The field is omitted from JSON when `None` (`skip_serializing_if`) and defaults
    /// to `None` when absent (`default`), ensuring backward compatibility with v1 change
    /// records that do not have a `data` key.
    ResourceCreate {
        resource: Resource,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        data: Option<Vec<u8>>,
    },
    /// A resource was permanently deleted on the originating device.
    /// Resources use hard delete (no soft-delete) because binary payloads can be very
    /// large and there is no benefit to retaining them after deletion.
    ResourceDelete { id: Uuid },
}
