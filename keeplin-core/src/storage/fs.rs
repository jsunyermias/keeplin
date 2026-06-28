//! Filesystem-backed implementation of [`StorageBackend`].
//!
//! [`FsBackend`] stores data as files under a user-chosen root directory that an external
//! file-synchronisation tool such as Syncthing replicates between devices. There are two
//! storage models:
//!
//! ## Notes — per-device logs with version-vector merge
//!
//! Each note is a directory `notes/{id}/` holding three kinds of file:
//! - `note.md` — the materialized markdown body (ciphertext when encryption is on);
//! - `meta.msgpack` — the materialized metadata projection plus the merged version vector;
//! - `log.{device_id}.msgpack` — an append-only operation log written **only** by that
//!   device.
//!
//! Because each log has a single writer it never conflicts under Syncthing. A note's true
//! state is the merge of all its logs, computed by comparing **version vectors** (see
//! [`crate::storage::note_log`]): a causal edit applies cleanly, while a genuine
//! concurrent edit is resolved deterministically by last-write-wins so every device
//! converges. `note.md` / `meta.msgpack` are local projections regenerated from the logs
//! on every write and sync; reads materialize live from the logs.
//!
//! ## Notebooks, tags, resources — sidecar files + global change log
//!
//! These remain a single MessagePack sidecar per entity, with every mutation appended as
//! a newline-delimited JSON (NDJSON) entry to a per-device log under `{root}/logs/`;
//! `receive_changes` reads new foreign entries via a byte-offset cursor.
//!
//! ## Operational note: log growth
//!
//! Both the per-note logs and the global `logs/` NDJSON files are append-only and are
//! **never pruned** by the backend (`prune_change_journal` is a deliberate no-op here,
//! because removing entries a peer has not yet consumed would corrupt that peer's sync
//! state). They therefore grow over the lifetime of the store — acceptable for typical
//! note volumes, but operators running very large or long-lived stores should compact
//! out-of-band once every device is known to have synced past a given point. There is
//! intentionally no automatic mechanism, since safe compaction requires knowing every
//! peer's consumed position, which lives outside this backend.

use std::io::SeekFrom;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{new_id, now, Change, Note, NoteTag, Notebook, Resource, Tag},
};

use super::note_log::{self, NoteLogEntry, NoteOp, VersionVector};
use super::{NoteRepository, NotebookRepository, ResourceRepository, SyncBackend, TagRepository};

/// The materialized projection written to `notes/{id}/meta.msgpack`.
///
/// It mirrors the merged note (its body lives in `note.md`, so the copy here is blanked
/// to avoid duplicating content) plus the merged version vector. It is a local cache
/// regenerated from the per-device logs on every write and sync; it is never the source
/// of truth for conflict resolution.
#[derive(Debug, Serialize, Deserialize)]
struct NoteMeta {
    note: Note,
    vv: VersionVector,
}

// ── Log entry ─────────────────────────────────────────────────────────────────

/// One line in a per-device NDJSON change log.
///
/// Each time a mutation is performed (create, update, delete) on any entity, one
/// `LogEntry` is appended as a single JSON object followed by a newline character.
/// Log files are plain text files that external tools (such as Syncthing) can
/// replicate between devices.
///
/// Backward-compatibility notes:
/// - `entity_type` defaults to `"note"` so log files written by version 1 of the
///   storage format (which had no `entity_type` field) are still parsed correctly.
/// - `entity_id` also accepts the old field name `"note_id"` via a serde alias, for
///   the same v1 compatibility reason.
#[derive(Debug, Serialize, Deserialize)]
struct LogEntry {
    timestamp: DateTime<Utc>,
    #[serde(default = "default_entity_type")]
    entity_type: String,
    #[serde(alias = "note_id")]
    entity_id: Uuid,
    operation: String,
    data: serde_json::Value,
}

fn default_entity_type() -> String {
    "note".to_string()
}

/// Convert a single [`LogEntry`] read from a log file into a typed [`Change`] variant.
///
/// Returns `None` for any `(entity_type, operation)` combination that is not
/// recognised. This can happen for two reasons:
/// 1. The log line is malformed (corrupted or partially written).
/// 2. The log line was written by a newer version of the software that added new
///    entity types or operations not known to this version.
///
/// Callers are expected to skip `None` entries and continue processing the rest of
/// the log. Skipped entries are logged as warnings by the callers that use this
/// function.
///
/// Version 1 backward compatibility: the old `"note"` entity type accepted the
/// operations `"create"`, `"update"`, and `"delete"` without any prefix. Both
/// old-style (`"create"`) and new-style (`"note_create"`) operation strings are
/// accepted so that logs from devices still running v1 can be integrated correctly.
fn log_entry_to_change(entry: LogEntry) -> Option<Change> {
    let id = entry.entity_id;
    // The log entry's own timestamp becomes the tombstone time for delete operations,
    // so a replayed delete competes in last-write-wins on the receiving device.
    let ts = entry.timestamp;
    match (entry.entity_type.as_str(), entry.operation.as_str()) {
        // Notes — "create"/"update"/"delete" accepted for v1 backward compat
        ("note", "create") | ("note", "note_create") => serde_json::from_value(entry.data)
            .ok()
            .map(|note| Change::NoteCreate { note }),
        ("note", "update") | ("note", "note_update") => serde_json::from_value(entry.data)
            .ok()
            .map(|note| Change::NoteUpdate { note }),
        ("note", "delete") | ("note", "note_delete") => {
            Some(Change::NoteDelete { id, deleted_at: ts })
        }
        // Notebooks
        ("notebook", "create") => serde_json::from_value(entry.data)
            .ok()
            .map(|notebook| Change::NotebookCreate { notebook }),
        ("notebook", "update") => serde_json::from_value(entry.data)
            .ok()
            .map(|notebook| Change::NotebookUpdate { notebook }),
        ("notebook", "delete") => Some(Change::NotebookDelete { id, deleted_at: ts }),
        // Tags
        ("tag", "create") => serde_json::from_value(entry.data)
            .ok()
            .map(|tag| Change::TagCreate { tag }),
        ("tag", "update") => serde_json::from_value(entry.data)
            .ok()
            .map(|tag| Change::TagUpdate { tag }),
        ("tag", "delete") => Some(Change::TagDelete { id, deleted_at: ts }),
        // NoteTag associations store only the secondary key in the `data` field
        // because the primary key (note_id) is already captured by `entity_id`.
        ("note_tag", "add") => {
            let tag_id: Uuid = entry.data["tag_id"].as_str()?.parse().ok()?;
            Some(Change::NoteTagAdd {
                note_id: id,
                tag_id,
            })
        }
        ("note_tag", "remove") => {
            let tag_id: Uuid = entry.data["tag_id"].as_str()?.parse().ok()?;
            Some(Change::NoteTagRemove {
                note_id: id,
                tag_id,
            })
        }
        // Resource log entries carry metadata (title, MIME type, file name) but not
        // the binary payload. Syncthing replicates the data file at
        // `{root}/resources/{id}/data` independently, so `data: None` is correct here.
        ("resource", "create") => {
            serde_json::from_value(entry.data)
                .ok()
                .map(|resource| Change::ResourceCreate {
                    resource,
                    data: None,
                })
        }
        ("resource", "delete") => Some(Change::ResourceDelete { id }),
        _ => None,
    }
}

// ── Sync state ────────────────────────────────────────────────────────────────

/// The contents of `.keeplin/sync_state.json`.
///
/// This struct records when the last complete synchronisation cycle finished.
/// It is written atomically (via a temporary file and an OS-level rename) so a
/// crash during the write cannot leave a partially-written file behind.
#[derive(Debug, Serialize, Deserialize)]
struct SyncState {
    /// The UTC timestamp of the most recent successful sync cycle.
    ///
    /// On the next sync cycle, `get_changes_since` uses this value to collect only
    /// those log entries that arrived after the previous cycle completed.
    last_sync: DateTime<Utc>,
}

// ── FsBackend ─────────────────────────────────────────────────────────────────

/// Filesystem-backed implementation of [`StorageBackend`].
///
/// Data is stored as JSON files under the following directory tree:
/// ```text
/// {root}/
///   notes/{uuid}/meta.json      — note metadata and body
///   notebooks/{uuid}.json       — notebook metadata
///   tags/{uuid}.json            — tag metadata
///   note_tags/{note_uuid}/{tag_uuid}  — empty sentinel file for each association
///   resources/{uuid}/meta.json  — resource metadata (title, MIME type, file name, size)
///   resources/{uuid}/data       — raw binary payload
///   logs/{device_id}.log        — this device's NDJSON change log
///   .keeplin/device_id          — persisted UUID that identifies this installation
///   .keeplin/format_version     — integer version stamp written on every startup
///   .keeplin/sync_state.json    — last-sync timestamp
///   .keeplin/offsets/{device_id} — byte-offset cursor for each foreign log file
/// ```
///
/// Syncthing (or any equivalent tool) replicates the entire `{root}` tree to other
/// devices. When a foreign device's log file appears under `{root}/logs/`, the
/// `receive_changes` method reads new entries starting from the stored byte-offset
/// cursor and advances the cursor so each entry is processed exactly once.
pub struct FsBackend {
    /// The root directory of the storage tree.
    root: PathBuf,
    /// The UUID string that uniquely identifies this device's log file. It is read from
    /// `.keeplin/device_id` on startup, or generated and persisted if the file does not
    /// yet exist.
    device_id: String,
}

impl FsBackend {
    /// Create a new `FsBackend` rooted at `root`.
    ///
    /// On the first call for a given directory, this method creates all required
    /// sub-directories, generates and persists a UUID device identifier, and stamps
    /// the current format version. On subsequent calls the directory structure is
    /// verified to exist and the format version file is updated if needed (the actual
    /// data migration for v1 → v2 is a no-op because the serde aliases in
    /// [`LogEntry`] handle the old field names transparently).
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Io` if any directory cannot be created or if the
    /// device-ID file cannot be read or written.
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let root: PathBuf = root.into();

        for dir in &[
            "notes",
            "resources",
            ".keeplin",
            ".keeplin/offsets",
            "logs",
            "notebooks",
            "tags",
            "note_tags",
        ] {
            tokio::fs::create_dir_all(root.join(dir)).await?;
        }

        let device_id = Self::read_or_create_device_id(&root).await?;
        let backend = Self { root, device_id };
        backend.ensure_format_version().await?;
        Ok(backend)
    }

    // ── Format version ────────────────────────────────────────────────────────

    /// The current on-disk storage format version.
    ///
    /// Increment this constant when a structural change to the directory layout or
    /// JSON schemas is made that requires an explicit data-migration step. Minor
    /// additions that are handled transparently by serde (such as new optional fields
    /// or serde aliases) do not require a version bump.
    const FORMAT_VERSION: u32 = 4;

    /// Returns the path of the format-version stamp file: `.keeplin/format_version`.
    fn format_version_path(&self) -> PathBuf {
        self.root.join(".keeplin").join("format_version")
    }

    /// Read the existing format version, perform any necessary migration steps, and
    /// overwrite the stamp file with the current [`FORMAT_VERSION`].
    ///
    /// If the stamp file does not exist, the directory is assumed to be a version-1
    /// layout. The v1 → v2 migration is a no-op because the `serde(alias)` attributes
    /// on [`LogEntry`] already make old log files parseable without renaming any fields.
    async fn ensure_format_version(&self) -> Result<(), StorageError> {
        let path = self.format_version_path();
        let current = if path.exists() {
            tokio::fs::read_to_string(&path)
                .await?
                .trim()
                .parse::<u32>()
                .unwrap_or(1)
        } else {
            1
        };

        if current < Self::FORMAT_VERSION {
            // The v1 → v2 migration requires no data transformation because the
            // `serde(alias = "note_id")` attribute on `LogEntry.entity_id` and the
            // `serde(default = "default_entity_type")` attribute on `LogEntry.entity_type`
            // together handle all v1 log files transparently at parse time. Only the
            // version stamp itself needs to be updated.
            tracing::info!(
                from = current,
                to = Self::FORMAT_VERSION,
                "Migrating FsBackend format"
            );
        }

        // Always (re)write the version stamp so that directories originally created
        // without a stamp file are stamped on the very first startup that uses this
        // version of the code.
        tokio::fs::write(&path, Self::FORMAT_VERSION.to_string()).await?;
        Ok(())
    }

    // ── Path helpers — Notes ──────────────────────────────────────────────────

    /// Returns `{root}/notes/{id}` — the directory that holds a single note's files.
    fn note_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("notes").join(id.to_string())
    }

    /// Returns `{root}/notes/{id}/note.md` — the materialized markdown body. Human- and
    /// tool-readable when encryption is off; ciphertext when an `EncryptedBackend` wraps
    /// this backend.
    fn note_md_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("note.md")
    }

    /// Returns `{root}/notes/{id}/meta.msgpack` — the materialized metadata projection
    /// (note fields plus merged version vector). A local cache, not the source of truth.
    fn note_meta_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("meta.msgpack")
    }

    /// Returns `{root}/notes/{id}/log.{device_id}.msgpack` — the append-only operation
    /// log written **only** by `device_id`. Single-writer, so it never conflicts under
    /// Syncthing; the union of all of a note's logs is its authoritative history.
    fn note_log_path(&self, id: Uuid, device_id: &str) -> PathBuf {
        self.note_dir(id).join(format!("log.{device_id}.msgpack"))
    }

    /// Returns the path of the NDJSON log file owned by this device:
    /// `{root}/logs/{device_id}.log`. Still used for notebooks, tags, and resources;
    /// notes use per-note logs instead.
    fn device_log_path(&self) -> PathBuf {
        self.root
            .join("logs")
            .join(format!("{}.log", self.device_id))
    }

    // ── Path helpers — Notebooks ──────────────────────────────────────────────

    /// Returns `{root}/notebooks/{id}.msgpack` — the MessagePack file that stores a notebook.
    fn notebook_path(&self, id: Uuid) -> PathBuf {
        self.root.join("notebooks").join(format!("{id}.msgpack"))
    }

    // ── Path helpers — Tags ───────────────────────────────────────────────────

    /// Returns `{root}/tags/{id}.msgpack` — the MessagePack file that stores a tag.
    fn tag_path(&self, id: Uuid) -> PathBuf {
        self.root.join("tags").join(format!("{id}.msgpack"))
    }

    // ── Path helpers — NoteTag ────────────────────────────────────────────────

    /// Returns `{root}/note_tags/{note_id}` — the directory that holds one empty
    /// sentinel file per tag attached to the note.
    fn note_tag_dir(&self, note_id: Uuid) -> PathBuf {
        self.root.join("note_tags").join(note_id.to_string())
    }

    /// Returns `{root}/note_tags/{note_id}/{tag_id}` — the empty sentinel file that
    /// records the association between a note and a tag. The file has no content;
    /// its mere existence encodes the relationship.
    fn note_tag_path(&self, note_id: Uuid, tag_id: Uuid) -> PathBuf {
        self.note_tag_dir(note_id).join(tag_id.to_string())
    }

    // ── Path helpers — Resources ──────────────────────────────────────────────

    /// Returns `{root}/resources/{id}` — the directory that holds a resource's
    /// metadata and binary payload.
    fn resource_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("resources").join(id.to_string())
    }

    /// Returns `{root}/resources/{id}/meta.msgpack` — the MessagePack file that stores a
    /// resource's metadata (title, MIME type, file name, size, creation timestamp).
    fn resource_meta_path(&self, id: Uuid) -> PathBuf {
        self.resource_dir(id).join("meta.msgpack")
    }

    /// Returns `{root}/resources/{id}/data` — the file that stores the raw binary
    /// payload of a resource. When `EncryptedBackend` is active, the payload is
    /// stored as `nonce || ciphertext` (raw bytes, no Base64 wrapper).
    fn resource_data_path(&self, id: Uuid) -> PathBuf {
        self.resource_dir(id).join("data")
    }

    // ── Device ID ─────────────────────────────────────────────────────────────

    /// Read the device identifier from `.keeplin/device_id`, or generate and persist
    /// a new UUID v4 string if the file does not yet exist.
    ///
    /// The device identifier is used as the name of this device's log file
    /// (`{root}/logs/{device_id}.log`) and as the Argon2id salt for
    /// `EncryptedBackend`. It must remain stable across restarts.
    async fn read_or_create_device_id(root: &Path) -> Result<String, StorageError> {
        let path = root.join(".keeplin").join("device_id");
        if path.exists() {
            let id = tokio::fs::read_to_string(&path).await?;
            Ok(id.trim().to_string())
        } else {
            let id = new_id().to_string();
            tokio::fs::write(&path, &id).await?;
            Ok(id)
        }
    }

    // ── Log helpers ───────────────────────────────────────────────────────────

    /// Append a single [`LogEntry`] to this device's NDJSON log file.
    ///
    /// The entry is serialised to a single JSON line and written with the file opened
    /// in append mode, which means multiple concurrent writers on the same operating
    /// system will not corrupt each other's entries as long as each `write_all` call
    /// is atomic at the kernel level (guaranteed for writes smaller than `PIPE_BUF`
    /// on POSIX systems, typically 4 KiB).
    ///
    /// # Parameters
    ///
    /// - `entity_type` — one of `"note"`, `"notebook"`, `"tag"`, `"note_tag"`, or
    ///   `"resource"`.
    /// - `entity_id` — the UUID of the affected entity.
    /// - `operation` — one of `"create"`, `"update"`, `"delete"`, `"add"`, or
    ///   `"remove"`.
    /// - `data` — the full serialised entity (for create/update) or a minimal object
    ///   such as `{"id": "<uuid>"}` (for delete).
    async fn append_log(
        &self,
        entity_type: &str,
        entity_id: Uuid,
        operation: &str,
        data: serde_json::Value,
    ) -> Result<(), StorageError> {
        let entry = LogEntry {
            timestamp: now(),
            entity_type: entity_type.to_string(),
            entity_id,
            operation: operation.to_string(),
            data,
        };
        let line = serde_json::to_string(&entry)? + "\n";
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.device_log_path())
            .await?;
        file.write_all(line.as_bytes()).await?;
        Ok(())
    }

    /// Returns the path of the byte-offset cursor file for a foreign device:
    /// `.keeplin/offsets/{device_id}`.
    ///
    /// The file stores a decimal integer representing the number of bytes already
    /// consumed from the foreign device's log file. On the next call to
    /// `receive_changes`, reading starts from this offset so each log entry is
    /// delivered exactly once.
    fn log_offset_path(&self, device_id: &str) -> PathBuf {
        self.root.join(".keeplin").join("offsets").join(device_id)
    }

    /// Read the stored byte offset for a foreign device log, or return `0` if no
    /// offset has been recorded yet (i.e., the log has never been processed before).
    async fn read_log_offset(&self, device_id: &str) -> u64 {
        tokio::fs::read_to_string(self.log_offset_path(device_id))
            .await
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    /// Persist the byte offset for a foreign device log using an atomic
    /// write-then-rename so that a crash during the write cannot leave the cursor
    /// file in a partially-written state. A torn cursor file would be interpreted
    /// as offset `0` by `read_log_offset`, causing duplicate delivery of already-
    /// processed log entries, which is safe but wasteful.
    async fn write_log_offset(&self, device_id: &str, offset: u64) -> Result<(), StorageError> {
        let path = self.log_offset_path(device_id);
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, offset.to_string()).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    /// Scan all foreign device log files under `{root}/logs/` and return every
    /// [`LogEntry`] whose timestamp is strictly later than `since`.
    ///
    /// This method reads **every** line of each foreign log file from the beginning
    /// on each call and does not advance the stored byte-offset cursor. It is used
    /// by `get_changes_since`, which only needs a filtered view of remote entries
    /// since a specific point in time (typically the last-sync timestamp).
    ///
    /// This device's own log file is skipped because the local device's changes
    /// are already reflected in the local state and do not need to be reapplied.
    /// Files that do not end with `.log` are also skipped to avoid confusion with
    /// offset-cursor files or other incidental files in the logs directory.
    ///
    /// Malformed JSON lines produce a `tracing::warn` and are silently skipped so
    /// that a single corrupt entry does not halt the entire sync.
    async fn read_other_logs_since(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<LogEntry>, StorageError> {
        let mut entries = Vec::new();
        let logs_dir = self.root.join("logs");
        let mut dir = match tokio::fs::read_dir(&logs_dir).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
            Err(e) => return Err(e.into()),
        };
        while let Some(dir_entry) = dir.next_entry().await? {
            let fname = dir_entry.file_name().to_string_lossy().into_owned();
            if fname == format!("{}.log", self.device_id) {
                continue;
            }
            if !fname.ends_with(".log") {
                continue;
            }
            let file = tokio::fs::File::open(dir_entry.path()).await?;
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    break;
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<LogEntry>(trimmed) {
                    Ok(e) if e.timestamp > since => entries.push(e),
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!("Skipping malformed log line: {err}");
                    }
                }
            }
        }
        Ok(entries)
    }

    /// Read all new entries from each foreign device log since the last call and
    /// advance the byte-offset cursor so that each entry is delivered exactly once.
    ///
    /// For each `.log` file in `{root}/logs/` that does not belong to this device,
    /// this method seeks to the previously recorded byte offset, reads every
    /// subsequent line, and updates the offset file to point past the last byte read.
    /// This means that on the next call only lines written after the current call
    /// will be returned.
    ///
    /// If writing the offset file fails (for example due to a disk-full condition),
    /// the error is logged as a warning but does not propagate. The consequence is
    /// that the same entries will be returned again on the next call — but since
    /// `apply_change` is idempotent (it uses `INSERT OR REPLACE` or checks existence
    /// before writing), duplicate delivery is safe.
    async fn read_new_entries(&self) -> Result<Vec<LogEntry>, StorageError> {
        let mut entries = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("logs")).await?;
        while let Some(dir_entry) = dir.next_entry().await? {
            let fname = dir_entry.file_name().to_string_lossy().into_owned();
            if fname == format!("{}.log", self.device_id) {
                continue;
            }
            if !fname.ends_with(".log") {
                continue;
            }
            let device_id = fname.trim_end_matches(".log").to_owned();
            let offset = self.read_log_offset(&device_id).await;

            let mut file = tokio::fs::File::open(dir_entry.path()).await?;
            file.seek(SeekFrom::Start(offset)).await?;

            let mut reader = BufReader::new(file);
            let mut line = String::new();
            let mut new_offset = offset;
            loop {
                line.clear();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    break;
                }
                new_offset += n as u64;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<LogEntry>(trimmed) {
                    Ok(e) => entries.push(e),
                    Err(err) => {
                        tracing::warn!("Skipping malformed log line: {err}");
                    }
                }
            }

            if new_offset > offset {
                if let Err(e) = self.write_log_offset(&device_id, new_offset).await {
                    tracing::warn!("Could not save log offset for {device_id}: {e}");
                }
            }
        }
        Ok(entries)
    }

    // ── Generic single-file MessagePack sidecar helpers ───────────────────────

    /// Serialise `value` to MessagePack and write it to `path` using an atomic
    /// temp-file-then-rename, so a concurrent reader never sees a half-written file.
    async fn write_sidecar<T: serde::Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), StorageError> {
        let bytes = rmp_serde::to_vec_named(value)
            .map_err(|e| StorageError::InvalidState(format!("msgpack encode: {e}")))?;
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, path).await?;
        Ok(())
    }

    /// Read `path` and deserialise its MessagePack contents into `T`.
    ///
    /// Returns `StorageError::NotFound(id)` when the file does not exist and
    /// `StorageError::CorruptedData` when the bytes are not valid MessagePack for `T`.
    async fn read_sidecar<T: serde::de::DeserializeOwned>(
        &self,
        path: &Path,
        id: Uuid,
    ) -> Result<T, StorageError> {
        if !path.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let bytes = tokio::fs::read(path).await?;
        rmp_serde::from_slice(&bytes)
            .map_err(|e| StorageError::CorruptedData(format!("msgpack decode: {e}")))
    }

    // ── Versioned note storage (per-device logs + version-vector merge) ────────

    /// Return a note's current merged version vector from its meta projection, or an
    /// empty vector when the note has no meta yet.
    async fn note_vv(&self, id: Uuid) -> Result<VersionVector, StorageError> {
        match self
            .read_sidecar::<NoteMeta>(&self.note_meta_path(id), id)
            .await
        {
            Ok(meta) => Ok(meta.vv),
            Err(StorageError::NotFound(_)) => Ok(VersionVector::new()),
            Err(e) => Err(e),
        }
    }

    /// Read every per-device log (`log.*.msgpack`) for a note. Missing note directory or
    /// unreadable individual logs yield an empty / skipped result rather than an error,
    /// so one corrupt log never blocks the merge of the others.
    async fn read_note_logs(&self, id: Uuid) -> Result<Vec<Vec<NoteLogEntry>>, StorageError> {
        let dir = self.note_dir(id);
        let mut logs = Vec::new();
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(logs),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("log.") && name.ends_with(".msgpack") {
                let bytes = tokio::fs::read(entry.path()).await?;
                match rmp_serde::from_slice::<Vec<NoteLogEntry>>(&bytes) {
                    Ok(v) => logs.push(v),
                    Err(e) => tracing::warn!("Skipping unreadable note log {name}: {e}"),
                }
            }
        }
        Ok(logs)
    }

    /// Merge all of a note's per-device logs into its current state and refresh the
    /// `note.md` + `meta.msgpack` projection. Returns the merged note, or `None` when the
    /// note has no log entries at all.
    async fn materialize(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
        let logs = self.read_note_logs(id).await?;
        let merged = note_log::merge(&logs);
        match merged.note {
            None => Ok(None),
            Some(note) => {
                if merged.conflict {
                    tracing::warn!(%id, "Concurrent note edit resolved by last-write-wins");
                }
                self.persist_note_projection(&note, &merged.vv).await?;
                Ok(Some(note))
            }
        }
    }

    /// Write the projection: the body to `note.md` and the metadata (body blanked, since
    /// it lives in `note.md`) plus the version vector to `meta.msgpack`. Both writes are
    /// atomic temp-then-rename.
    async fn persist_note_projection(
        &self,
        note: &Note,
        vv: &VersionVector,
    ) -> Result<(), StorageError> {
        tokio::fs::create_dir_all(self.note_dir(note.id)).await?;
        let md = self.note_md_path(note.id);
        let md_tmp = md.with_extension("tmp");
        tokio::fs::write(&md_tmp, note.body.as_bytes()).await?;
        tokio::fs::rename(&md_tmp, &md).await?;
        let mut meta_note = note.clone();
        meta_note.body = String::new();
        self.write_sidecar(
            &self.note_meta_path(note.id),
            &NoteMeta {
                note: meta_note,
                vv: vv.clone(),
            },
        )
        .await
    }

    /// Append an operation to this device's note log, then re-materialize the note and
    /// return the merged result. This is the single entry point for every local note
    /// mutation (create, update, delete).
    async fn append_note_op(&self, id: Uuid, op: NoteOp) -> Result<Note, StorageError> {
        tokio::fs::create_dir_all(self.note_dir(id)).await?;
        let mut vv = self.note_vv(id).await?;
        note_log::increment(&mut vv, &self.device_id);
        let log_path = self.note_log_path(id, &self.device_id);
        let mut log: Vec<NoteLogEntry> = match self.read_sidecar(&log_path, id).await {
            Ok(v) => v,
            Err(StorageError::NotFound(_)) => Vec::new(),
            Err(e) => return Err(e),
        };
        log.push(NoteLogEntry {
            vv,
            timestamp: now(),
            device_id: self.device_id.clone(),
            op,
        });
        self.write_sidecar(&log_path, &log).await?;
        self.materialize(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))
    }

    /// Scan every note directory and re-materialize those whose per-device logs have
    /// advanced beyond the locally stored projection (for example because Syncthing just
    /// replicated a peer's log). Returns one [`Change`] per advanced note — a
    /// `NoteUpdate` for a live note or a `NoteDelete` for a tombstoned one — so the sync
    /// engine can report them. Comparison is by version vector, not file mtime, so it is
    /// immune to clock skew between devices.
    async fn collect_advanced_notes(&self) -> Result<Vec<Change>, StorageError> {
        let mut changes = Vec::new();
        let notes_dir = self.root.join("notes");
        let mut rd = match tokio::fs::read_dir(&notes_dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(changes),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            let id = match Uuid::parse_str(&entry.file_name().to_string_lossy()) {
                Ok(id) => id,
                Err(_) => continue,
            };
            let old_vv = self.note_vv(id).await?;
            let logs = self.read_note_logs(id).await?;
            let merged = note_log::merge(&logs);
            // The merged frontier differs from what we last materialized → new content.
            if merged.vv != old_vv {
                if let Some(note) = merged.note {
                    self.persist_note_projection(&note, &merged.vv).await?;
                    match note.deleted_at {
                        Some(deleted_at) => changes.push(Change::NoteDelete { id, deleted_at }),
                        None => changes.push(Change::NoteUpdate { note }),
                    }
                }
            }
        }
        Ok(changes)
    }
}

// ── Pagination helper ─────────────────────────────────────────────────────────

/// Apply cursor-based pagination to an already-sorted `items` slice.
///
/// The cursor format is `"<created_at_rfc3339>|<uuid>"`. An absent or empty
/// cursor means "start from the first item". Items are compared by the
/// `(created_at, id)` pair returned by `key_fn`; the cursor points to the last
/// item of the previous page, so the next page starts immediately after it.
///
/// Returns `(page, next_token)` where `next_token` is `None` when the page
/// exhausts all remaining items.
fn paginate<T, F>(
    items: Vec<T>,
    limit: usize,
    token: Option<&str>,
    key_fn: F,
) -> (Vec<T>, Option<String>)
where
    F: Fn(&T) -> (String, Uuid),
{
    let start = match token.filter(|t| !t.is_empty()) {
        Some(cursor) => {
            if let Some((ts, id_str)) = cursor.split_once('|') {
                if let Ok(cursor_id) = Uuid::parse_str(id_str) {
                    items.partition_point(|item| {
                        let (item_ts, item_id) = key_fn(item);
                        item_ts.as_str() < ts || (item_ts.as_str() == ts && item_id <= cursor_id)
                    })
                } else {
                    0
                }
            } else {
                0
            }
        }
        None => 0,
    };

    let remaining: Vec<T> = items.into_iter().skip(start).collect();
    let has_more = remaining.len() > limit;
    let page: Vec<T> = remaining.into_iter().take(limit).collect();

    let next_token = if has_more {
        page.last().map(|last| {
            let (ts, id) = key_fn(last);
            format!("{ts}|{id}")
        })
    } else {
        None
    };

    (page, next_token)
}

// ── NoteRepository impl ───────────────────────────────────────────────────────

#[async_trait]
impl NoteRepository for FsBackend {
    async fn create_note(&self, note: Note) -> Result<Note, StorageError> {
        let merged = self.append_note_op(note.id, NoteOp::Upsert(note)).await?;
        tracing::info!(id = %merged.id, "Note created");
        Ok(merged)
    }

    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        // Reads materialize live from the per-device logs, so they always reflect the
        // latest merge — even immediately after Syncthing brings in a peer's log.
        self.materialize(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))
    }

    async fn update_note(&self, note: Note) -> Result<Note, StorageError> {
        if self.read_note_logs(note.id).await?.is_empty() {
            return Err(StorageError::NotFound(note.id.to_string()));
        }
        let merged = self.append_note_op(note.id, NoteOp::Upsert(note)).await?;
        tracing::info!(id = %merged.id, "Note updated");
        Ok(merged)
    }

    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        if self.read_note_logs(id).await?.is_empty() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        self.append_note_op(id, NoteOp::Tombstone { deleted_at: now() })
            .await?;
        tracing::info!(%id, "Note deleted");
        Ok(())
    }

    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let limit = if page_size == 0 {
            100
        } else {
            page_size as usize
        };
        let mut notes = Vec::new();
        let mut dir = match tokio::fs::read_dir(self.root.join("notes")).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((vec![], None));
            }
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = dir.next_entry().await? {
            let id_str = entry.file_name().to_string_lossy().to_string();
            if let Ok(id) = Uuid::parse_str(&id_str) {
                match self.materialize(id).await {
                    Ok(Some(n)) if n.deleted_at.is_none() => notes.push(n),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("Could not materialize note {id}: {e}"),
                }
            }
        }
        notes.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(paginate(notes, limit, page_token.as_deref(), |n| {
            (n.created_at.to_rfc3339(), n.id)
        }))
    }
}

// ── NotebookRepository impl ───────────────────────────────────────────────────

#[async_trait]
impl NotebookRepository for FsBackend {
    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        self.write_sidecar(&self.notebook_path(notebook.id), &notebook)
            .await?;
        self.append_log(
            "notebook",
            notebook.id,
            "create",
            serde_json::to_value(&notebook)?,
        )
        .await?;
        tracing::info!(id = %notebook.id, "Notebook created");
        Ok(notebook)
    }

    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        self.read_sidecar(&self.notebook_path(id), id).await
    }

    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        if !self.notebook_path(notebook.id).exists() {
            return Err(StorageError::NotFound(notebook.id.to_string()));
        }
        self.write_sidecar(&self.notebook_path(notebook.id), &notebook)
            .await?;
        self.append_log(
            "notebook",
            notebook.id,
            "update",
            serde_json::to_value(&notebook)?,
        )
        .await?;
        tracing::info!(id = %notebook.id, "Notebook updated");
        Ok(notebook)
    }

    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        let mut nb: Notebook = self.read_sidecar(&self.notebook_path(id), id).await?;
        let ts = now();
        nb.deleted_at = Some(ts);
        nb.updated_at = ts;
        self.write_sidecar(&self.notebook_path(id), &nb).await?;
        self.append_log("notebook", id, "delete", serde_json::json!({ "id": id }))
            .await?;
        tracing::info!(%id, "Notebook deleted");
        Ok(())
    }

    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        let limit = if page_size == 0 {
            100
        } else {
            page_size as usize
        };
        let mut notebooks = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("notebooks")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(stem) = fname.strip_suffix(".json") {
                if let Ok(id) = Uuid::parse_str(stem) {
                    match self.read_sidecar::<Notebook>(&entry.path(), id).await {
                        Ok(nb) if nb.deleted_at.is_none() => notebooks.push(nb),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("Could not load notebook {id}: {e}"),
                    }
                }
            }
        }
        notebooks.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(paginate(notebooks, limit, page_token.as_deref(), |nb| {
            (nb.created_at.to_rfc3339(), nb.id)
        }))
    }
}

// ── TagRepository impl ────────────────────────────────────────────────────────

#[async_trait]
impl TagRepository for FsBackend {
    async fn create_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        self.write_sidecar(&self.tag_path(tag.id), &tag).await?;
        self.append_log("tag", tag.id, "create", serde_json::to_value(&tag)?)
            .await?;
        tracing::info!(id = %tag.id, "Tag created");
        Ok(tag)
    }

    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        self.read_sidecar(&self.tag_path(id), id).await
    }

    async fn update_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        if !self.tag_path(tag.id).exists() {
            return Err(StorageError::NotFound(tag.id.to_string()));
        }
        self.write_sidecar(&self.tag_path(tag.id), &tag).await?;
        self.append_log("tag", tag.id, "update", serde_json::to_value(&tag)?)
            .await?;
        tracing::info!(id = %tag.id, "Tag updated");
        Ok(tag)
    }

    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        let mut tag: Tag = self.read_sidecar(&self.tag_path(id), id).await?;
        let ts = now();
        tag.deleted_at = Some(ts);
        tag.updated_at = ts;
        self.write_sidecar(&self.tag_path(id), &tag).await?;
        self.append_log("tag", id, "delete", serde_json::json!({ "id": id }))
            .await?;
        tracing::info!(%id, "Tag deleted");
        Ok(())
    }

    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let limit = if page_size == 0 {
            100
        } else {
            page_size as usize
        };
        let mut tags = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("tags")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(stem) = fname.strip_suffix(".json") {
                if let Ok(id) = Uuid::parse_str(stem) {
                    match self.read_sidecar::<Tag>(&entry.path(), id).await {
                        Ok(t) if t.deleted_at.is_none() => tags.push(t),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("Could not load tag {id}: {e}"),
                    }
                }
            }
        }
        tags.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(paginate(tags, limit, page_token.as_deref(), |t| {
            (t.created_at.to_rfc3339(), t.id)
        }))
    }

    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        tokio::fs::create_dir_all(self.note_tag_dir(note_tag.note_id)).await?;
        tokio::fs::write(self.note_tag_path(note_tag.note_id, note_tag.tag_id), b"").await?;
        self.append_log(
            "note_tag",
            note_tag.note_id,
            "add",
            serde_json::json!({ "tag_id": note_tag.tag_id }),
        )
        .await?;
        Ok(())
    }

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        let path = self.note_tag_path(note_id, tag_id);
        if path.exists() {
            tokio::fs::remove_file(path).await?;
        }
        self.append_log(
            "note_tag",
            note_id,
            "remove",
            serde_json::json!({ "tag_id": tag_id }),
        )
        .await?;
        Ok(())
    }

    async fn list_note_tags(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let limit = if page_size == 0 {
            100
        } else {
            page_size as usize
        };
        let dir_path = self.note_tag_dir(note_id);
        if !dir_path.exists() {
            return Ok((vec![], None));
        }
        let mut tags = Vec::new();
        let mut dir = tokio::fs::read_dir(&dir_path).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Ok(tag_id) = Uuid::parse_str(&fname) {
                match self.read_tag(tag_id).await {
                    Ok(t) if t.deleted_at.is_none() => tags.push(t),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("Could not load tag {tag_id} for note {note_id}: {e}"),
                }
            }
        }
        tags.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(paginate(tags, limit, page_token.as_deref(), |t| {
            (t.created_at.to_rfc3339(), t.id)
        }))
    }
}

// ── ResourceRepository impl ───────────────────────────────────────────────────

#[async_trait]
impl ResourceRepository for FsBackend {
    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let dir = self.resource_dir(resource.id);
        tokio::fs::create_dir_all(&dir).await?;
        // Write the binary payload first, then the metadata file. `read_resource` treats
        // the presence of `meta.json` as proof the resource exists, so writing it last
        // makes it the commit marker: a crash between the two writes leaves an orphan
        // data file (harmless, overwritten on retry) rather than a metadata record that
        // points at a missing payload.
        tokio::fs::write(self.resource_data_path(resource.id), &data).await?;
        self.write_sidecar(&self.resource_meta_path(resource.id), &resource)
            .await?;
        self.append_log(
            "resource",
            resource.id,
            "create",
            serde_json::to_value(&resource)?,
        )
        .await?;
        tracing::info!(id = %resource.id, "Resource created");
        Ok(resource)
    }

    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let meta_path = self.resource_meta_path(id);
        if !meta_path.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let resource: Resource = self.read_sidecar(&meta_path, id).await?;
        let data = tokio::fs::read(self.resource_data_path(id)).await?;
        Ok((resource, data))
    }

    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        let dir = self.resource_dir(id);
        if !dir.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        tokio::fs::remove_dir_all(&dir).await?;
        self.append_log("resource", id, "delete", serde_json::json!({ "id": id }))
            .await?;
        tracing::info!(%id, "Resource deleted");
        Ok(())
    }

    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let limit = if page_size == 0 {
            100
        } else {
            page_size as usize
        };
        let mut resources = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("resources")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let id_str = entry.file_name().to_string_lossy().to_string();
            if let Ok(id) = Uuid::parse_str(&id_str) {
                let meta_path = self.resource_meta_path(id);
                match self.read_sidecar::<Resource>(&meta_path, id).await {
                    Ok(r) => resources.push(r),
                    Err(e) => tracing::warn!("Could not load resource {id}: {e}"),
                }
            }
        }
        resources.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(paginate(resources, limit, page_token.as_deref(), |r| {
            (r.created_at.to_rfc3339(), r.id)
        }))
    }
}

// ── SyncBackend impl ──────────────────────────────────────────────────────────

#[async_trait]
impl SyncBackend for FsBackend {
    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError> {
        let entries = self.read_other_logs_since(since).await?;
        let changes = entries
            .into_iter()
            .filter_map(|e| {
                let result = log_entry_to_change(e);
                if result.is_none() {
                    tracing::warn!("Skipped unrecognised log entry");
                }
                result
            })
            .collect();
        Ok(changes)
    }

    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        match change {
            // Notes — conflict resolution lives entirely in the per-device logs, which
            // Syncthing has already replicated to disk. Applying a remote note change is
            // therefore just a re-materialization: read every log, merge by version
            // vector, and refresh the projection. It never appends to this device's log,
            // so it cannot create a spurious local edit, and it is idempotent.
            Change::NoteCreate { note } | Change::NoteUpdate { note } => {
                self.materialize(note.id).await?;
                tracing::debug!(id = %note.id, "Materialized remote note change");
            }
            Change::NoteDelete { id, .. } => {
                self.materialize(id).await?;
                tracing::debug!(%id, "Materialized remote note delete");
            }
            // Notebooks — last-write-wins by `updated_at` (see the note arm above).
            Change::NotebookCreate { notebook } | Change::NotebookUpdate { notebook } => {
                let path = self.notebook_path(notebook.id);
                let apply = match self.read_sidecar::<Notebook>(&path, notebook.id).await {
                    Ok(existing) => notebook.updated_at > existing.updated_at,
                    Err(StorageError::NotFound(_)) => true,
                    Err(e) => return Err(e),
                };
                if apply {
                    self.write_sidecar(&path, &notebook).await?;
                    tracing::debug!(id = %notebook.id, "Applied remote notebook change");
                } else {
                    tracing::debug!(id = %notebook.id, "Skipped stale remote notebook change");
                }
            }
            Change::NotebookDelete { id, deleted_at } => {
                let path = self.notebook_path(id);
                if path.exists() {
                    let mut nb: Notebook = self.read_sidecar(&path, id).await?;
                    if deleted_at > nb.updated_at {
                        nb.deleted_at = Some(deleted_at);
                        nb.updated_at = deleted_at;
                        self.write_sidecar(&path, &nb).await?;
                        tracing::debug!(%id, "Applied remote notebook delete");
                    } else {
                        tracing::debug!(%id, "Skipped stale remote notebook delete");
                    }
                }
            }
            // Tags — last-write-wins by `updated_at` (see the note arm above).
            Change::TagCreate { tag } | Change::TagUpdate { tag } => {
                let path = self.tag_path(tag.id);
                let apply = match self.read_sidecar::<Tag>(&path, tag.id).await {
                    Ok(existing) => tag.updated_at > existing.updated_at,
                    Err(StorageError::NotFound(_)) => true,
                    Err(e) => return Err(e),
                };
                if apply {
                    self.write_sidecar(&path, &tag).await?;
                    tracing::debug!(id = %tag.id, "Applied remote tag change");
                } else {
                    tracing::debug!(id = %tag.id, "Skipped stale remote tag change");
                }
            }
            Change::TagDelete { id, deleted_at } => {
                let path = self.tag_path(id);
                if path.exists() {
                    let mut t: Tag = self.read_sidecar(&path, id).await?;
                    if deleted_at > t.updated_at {
                        t.deleted_at = Some(deleted_at);
                        t.updated_at = deleted_at;
                        self.write_sidecar(&path, &t).await?;
                        tracing::debug!(%id, "Applied remote tag delete");
                    } else {
                        tracing::debug!(%id, "Skipped stale remote tag delete");
                    }
                }
            }
            // NoteTag associations
            Change::NoteTagAdd { note_id, tag_id } => {
                tokio::fs::create_dir_all(self.note_tag_dir(note_id)).await?;
                tokio::fs::write(self.note_tag_path(note_id, tag_id), b"").await?;
                tracing::debug!(%note_id, %tag_id, "Applied remote note_tag add");
            }
            Change::NoteTagRemove { note_id, tag_id } => {
                let path = self.note_tag_path(note_id, tag_id);
                if path.exists() {
                    tokio::fs::remove_file(path).await?;
                }
                tracing::debug!(%note_id, %tag_id, "Applied remote note_tag remove");
            }
            // Resource changes from a DbBackend peer include the binary payload
            // (data=Some) because the database stores bytes directly and can embed
            // them in the change record. Resource changes originating from another
            // FsBackend peer set data=None because Syncthing has already replicated
            // the `resources/{id}/data` file through the filesystem. In the latter
            // case, writing the metadata file is sufficient; the data file is already
            // in place or will arrive shortly via replication.
            Change::ResourceCreate { resource, data } => {
                let dir = self.resource_dir(resource.id);
                tokio::fs::create_dir_all(&dir).await?;
                self.write_sidecar(&self.resource_meta_path(resource.id), &resource)
                    .await?;
                if let Some(bytes) = data {
                    tokio::fs::write(self.resource_data_path(resource.id), &bytes).await?;
                }
                tracing::debug!(id = %resource.id, "Applied remote resource create");
            }
            Change::ResourceDelete { id } => {
                let dir = self.resource_dir(id);
                if dir.exists() {
                    tokio::fs::remove_dir_all(dir).await?;
                }
                tracing::debug!(%id, "Applied remote resource delete");
            }
        }
        Ok(())
    }

    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
        let path = self.root.join(".keeplin").join("sync_state.msgpack");
        match self.read_sidecar::<SyncState>(&path, Uuid::nil()).await {
            Ok(state) => Ok(state.last_sync),
            Err(StorageError::NotFound(_)) => {
                Ok(DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default())
            }
            Err(e) => Err(e),
        }
    }

    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        let state = SyncState { last_sync: ts };
        let path = self.root.join(".keeplin").join("sync_state.msgpack");
        self.write_sidecar(&path, &state).await
    }

    async fn send_changes(&self, _changes: Vec<Change>) -> Result<(), StorageError> {
        // In filesystem mode, changes are not pushed to a remote server. Instead,
        // Syncthing (or a similar tool) replicates the `logs/` directory from this
        // device to all other devices. This method is therefore a no-op; the
        // `SyncEngine` still calls it as part of the standard six-step cycle, so
        // the method must exist and succeed.
        tracing::debug!("Offline mode: changes are replicated passively via the filesystem");
        Ok(())
    }

    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        // Notebooks, tags, and resources still flow through the global per-device NDJSON
        // logs and are discovered by advancing each foreign log's byte-offset cursor.
        let mut changes: Vec<Change> = self
            .read_new_entries()
            .await?
            .into_iter()
            .filter_map(|e| {
                let result = log_entry_to_change(e);
                if result.is_none() {
                    tracing::warn!("Skipped unrecognised log entry in receive_changes");
                }
                result
            })
            .collect();
        // Notes flow through per-note version-vector logs: detect and materialize any
        // whose logs advanced (e.g. a peer's log just arrived via Syncthing).
        changes.extend(self.collect_advanced_notes().await?);
        Ok(changes)
    }

    async fn get_device_id(&self) -> Result<String, StorageError> {
        Ok(self.device_id.clone())
    }

    async fn prune_change_journal(&self, _older_than: DateTime<Utc>) -> Result<u64, StorageError> {
        // The filesystem backend stores changes as append-only NDJSON log files that are
        // replicated to other devices by Syncthing. Removing entries from these files
        // would cause any device that has not yet processed the removed entries to miss
        // those changes permanently, potentially corrupting the sync state. This method
        // therefore intentionally does nothing and always returns zero.
        Ok(0)
    }
}
