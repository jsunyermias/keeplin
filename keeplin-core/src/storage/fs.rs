//! Filesystem-backed implementation of [`StorageBackend`].
//!
//! [`FsBackend`] stores every entity as a JSON file under a user-chosen root
//! directory and records every mutation as a newline-delimited JSON (NDJSON) entry
//! in a per-device log file under `{root}/logs/`. An external file-synchronisation
//! tool such as Syncthing can replicate the entire root directory to other devices;
//! `receive_changes` then reads the newly arrived foreign log files to discover what
//! changed on remote devices, advancing a byte-offset cursor so each entry is
//! processed exactly once.

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

use super::StorageBackend;

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
    match (entry.entity_type.as_str(), entry.operation.as_str()) {
        // Notes — "create"/"update"/"delete" accepted for v1 backward compat
        ("note", "create") | ("note", "note_create") => serde_json::from_value(entry.data)
            .ok()
            .map(|note| Change::NoteCreate { note }),
        ("note", "update") | ("note", "note_update") => serde_json::from_value(entry.data)
            .ok()
            .map(|note| Change::NoteUpdate { note }),
        ("note", "delete") | ("note", "note_delete") => Some(Change::NoteDelete { id }),
        // Notebooks
        ("notebook", "create") => serde_json::from_value(entry.data)
            .ok()
            .map(|notebook| Change::NotebookCreate { notebook }),
        ("notebook", "update") => serde_json::from_value(entry.data)
            .ok()
            .map(|notebook| Change::NotebookUpdate { notebook }),
        ("notebook", "delete") => Some(Change::NotebookDelete { id }),
        // Tags
        ("tag", "create") => serde_json::from_value(entry.data)
            .ok()
            .map(|tag| Change::TagCreate { tag }),
        ("tag", "update") => serde_json::from_value(entry.data)
            .ok()
            .map(|tag| Change::TagUpdate { tag }),
        ("tag", "delete") => Some(Change::TagDelete { id }),
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
    const FORMAT_VERSION: u32 = 2;

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

    /// Returns `{root}/notes/{id}/meta.json` — the JSON file that stores a note's
    /// metadata and body text.
    fn meta_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("meta.json")
    }

    /// Returns the path of the NDJSON log file owned by this device:
    /// `{root}/logs/{device_id}.log`.
    fn device_log_path(&self) -> PathBuf {
        self.root
            .join("logs")
            .join(format!("{}.log", self.device_id))
    }

    // ── Path helpers — Notebooks ──────────────────────────────────────────────

    /// Returns `{root}/notebooks/{id}.json` — the JSON file that stores a notebook.
    fn notebook_path(&self, id: Uuid) -> PathBuf {
        self.root.join("notebooks").join(format!("{id}.json"))
    }

    // ── Path helpers — Tags ───────────────────────────────────────────────────

    /// Returns `{root}/tags/{id}.json` — the JSON file that stores a tag.
    fn tag_path(&self, id: Uuid) -> PathBuf {
        self.root.join("tags").join(format!("{id}.json"))
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

    /// Returns `{root}/resources/{id}/meta.json` — the JSON file that stores a
    /// resource's metadata (title, MIME type, file name, size, creation timestamp).
    fn resource_meta_path(&self, id: Uuid) -> PathBuf {
        self.resource_dir(id).join("meta.json")
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

    // ── Note persistence ──────────────────────────────────────────────────────

    /// Persist a note to `{root}/notes/{id}/meta.json` using an atomic write.
    ///
    /// The JSON is first written to `meta.tmp` and then renamed over `meta.json`.
    /// On all major operating systems the rename is atomic with respect to readers:
    /// a concurrent reader will see either the old file or the new file, never a
    /// partially-written intermediate state.
    async fn write_note(&self, note: &Note) -> Result<(), StorageError> {
        let dir = self.note_dir(note.id);
        tokio::fs::create_dir_all(&dir).await?;
        let target = self.meta_path(note.id);
        let tmp = target.with_extension("tmp");
        tokio::fs::write(&tmp, serde_json::to_string_pretty(note)?).await?;
        tokio::fs::rename(&tmp, &target).await?;
        Ok(())
    }

    /// Load the note with the given `id` from `{root}/notes/{id}/meta.json`.
    ///
    /// Returns `StorageError::NotFound` if the file does not exist. Returns
    /// `StorageError::Json` if the file is present but contains invalid JSON.
    /// Deleted notes (those with a non-`None` `deleted_at` field) are returned
    /// as-is; callers decide whether to expose or hide them.
    async fn load_note(&self, id: Uuid) -> Result<Note, StorageError> {
        let meta_path = self.meta_path(id);
        if !meta_path.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let raw = tokio::fs::read_to_string(meta_path).await?;
        let note: Note = serde_json::from_str(&raw)?;
        Ok(note)
    }

    // ── Generic single-file JSON helpers ──────────────────────────────────────

    /// Write `value` as pretty-printed JSON to `path` using an atomic write.
    ///
    /// The JSON is written to a temporary file named `path` with the extension
    /// replaced by `.tmp`, then that file is renamed over `path`. The rename is
    /// atomic on all major operating systems, so a concurrent reader will never
    /// see a partially-written file.
    async fn write_json<T: serde::Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), StorageError> {
        let raw = serde_json::to_string_pretty(value)?;
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, raw).await?;
        tokio::fs::rename(&tmp, path).await?;
        Ok(())
    }

    /// Read the file at `path` and deserialise it as JSON into `T`.
    ///
    /// Returns `StorageError::NotFound(id.to_string())` when the file does not exist,
    /// which surfaces to callers as the standard "entity not found" error. Returns
    /// `StorageError::Json` if the file contains invalid JSON.
    async fn read_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &Path,
        id: Uuid,
    ) -> Result<T, StorageError> {
        if !path.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let raw = tokio::fs::read_to_string(path).await?;
        let value: T = serde_json::from_str(&raw)?;
        Ok(value)
    }
}

// ── StorageBackend impl ───────────────────────────────────────────────────────

#[async_trait]
impl StorageBackend for FsBackend {
    // ── Notes ─────────────────────────────────────────────────────────────────

    async fn create_note(&self, note: Note) -> Result<Note, StorageError> {
        self.write_note(&note).await?;
        self.append_log("note", note.id, "create", serde_json::to_value(&note)?)
            .await?;
        tracing::info!(id = %note.id, "Note created");
        Ok(note)
    }

    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        self.load_note(id).await
    }

    async fn update_note(&self, note: Note) -> Result<Note, StorageError> {
        if !self.meta_path(note.id).exists() {
            return Err(StorageError::NotFound(note.id.to_string()));
        }
        self.write_note(&note).await?;
        self.append_log("note", note.id, "update", serde_json::to_value(&note)?)
            .await?;
        tracing::info!(id = %note.id, "Note updated");
        Ok(note)
    }

    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        let mut note = self.load_note(id).await?;
        note.deleted_at = Some(now());
        self.write_note(&note).await?;
        self.append_log("note", id, "delete", serde_json::json!({ "id": id }))
            .await?;
        tracing::info!(%id, "Note deleted");
        Ok(())
    }

    async fn list_notes(&self) -> Result<Vec<Note>, StorageError> {
        let mut notes = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("notes")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let id_str = entry.file_name().to_string_lossy().to_string();
            if let Ok(id) = Uuid::parse_str(&id_str) {
                match self.load_note(id).await {
                    Ok(n) if n.deleted_at.is_none() => notes.push(n),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("Could not load note {id}: {e}"),
                }
            }
        }
        Ok(notes)
    }

    // ── Notebooks ─────────────────────────────────────────────────────────────

    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        self.write_json(&self.notebook_path(notebook.id), &notebook)
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
        self.read_json(&self.notebook_path(id), id).await
    }

    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        if !self.notebook_path(notebook.id).exists() {
            return Err(StorageError::NotFound(notebook.id.to_string()));
        }
        self.write_json(&self.notebook_path(notebook.id), &notebook)
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
        let mut nb: Notebook = self.read_json(&self.notebook_path(id), id).await?;
        nb.deleted_at = Some(now());
        self.write_json(&self.notebook_path(id), &nb).await?;
        self.append_log("notebook", id, "delete", serde_json::json!({ "id": id }))
            .await?;
        tracing::info!(%id, "Notebook deleted");
        Ok(())
    }

    async fn list_notebooks(&self) -> Result<Vec<Notebook>, StorageError> {
        let mut notebooks = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("notebooks")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(stem) = fname.strip_suffix(".json") {
                if let Ok(id) = Uuid::parse_str(stem) {
                    match self.read_json::<Notebook>(&entry.path(), id).await {
                        Ok(nb) if nb.deleted_at.is_none() => notebooks.push(nb),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("Could not load notebook {id}: {e}"),
                    }
                }
            }
        }
        Ok(notebooks)
    }

    // ── Tags ──────────────────────────────────────────────────────────────────

    async fn create_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        self.write_json(&self.tag_path(tag.id), &tag).await?;
        self.append_log("tag", tag.id, "create", serde_json::to_value(&tag)?)
            .await?;
        tracing::info!(id = %tag.id, "Tag created");
        Ok(tag)
    }

    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        self.read_json(&self.tag_path(id), id).await
    }

    async fn update_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        if !self.tag_path(tag.id).exists() {
            return Err(StorageError::NotFound(tag.id.to_string()));
        }
        self.write_json(&self.tag_path(tag.id), &tag).await?;
        self.append_log("tag", tag.id, "update", serde_json::to_value(&tag)?)
            .await?;
        tracing::info!(id = %tag.id, "Tag updated");
        Ok(tag)
    }

    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        let mut tag: Tag = self.read_json(&self.tag_path(id), id).await?;
        tag.deleted_at = Some(now());
        self.write_json(&self.tag_path(id), &tag).await?;
        self.append_log("tag", id, "delete", serde_json::json!({ "id": id }))
            .await?;
        tracing::info!(%id, "Tag deleted");
        Ok(())
    }

    async fn list_tags(&self) -> Result<Vec<Tag>, StorageError> {
        let mut tags = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("tags")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(stem) = fname.strip_suffix(".json") {
                if let Ok(id) = Uuid::parse_str(stem) {
                    match self.read_json::<Tag>(&entry.path(), id).await {
                        Ok(t) if t.deleted_at.is_none() => tags.push(t),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("Could not load tag {id}: {e}"),
                    }
                }
            }
        }
        Ok(tags)
    }

    // ── Note–Tag relations ────────────────────────────────────────────────────

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

    async fn list_note_tags(&self, note_id: Uuid) -> Result<Vec<Tag>, StorageError> {
        let dir_path = self.note_tag_dir(note_id);
        if !dir_path.exists() {
            return Ok(vec![]);
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
        Ok(tags)
    }

    // ── Resources ─────────────────────────────────────────────────────────────

    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let dir = self.resource_dir(resource.id);
        tokio::fs::create_dir_all(&dir).await?;
        self.write_json(&self.resource_meta_path(resource.id), &resource)
            .await?;
        tokio::fs::write(self.resource_data_path(resource.id), &data).await?;
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
        let resource: Resource = self.read_json(&meta_path, id).await?;
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

    async fn list_resources(&self) -> Result<Vec<Resource>, StorageError> {
        let mut resources = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("resources")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let id_str = entry.file_name().to_string_lossy().to_string();
            if let Ok(id) = Uuid::parse_str(&id_str) {
                let meta_path = self.resource_meta_path(id);
                match self.read_json::<Resource>(&meta_path, id).await {
                    Ok(r) => resources.push(r),
                    Err(e) => tracing::warn!("Could not load resource {id}: {e}"),
                }
            }
        }
        Ok(resources)
    }

    // ── Synchronisation ───────────────────────────────────────────────────────

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
            // Notes
            Change::NoteCreate { note } | Change::NoteUpdate { note } => {
                self.write_note(&note).await?;
                tracing::debug!(id = %note.id, "Applied remote note change");
            }
            Change::NoteDelete { id } => {
                if self.meta_path(id).exists() {
                    let mut note = self.load_note(id).await?;
                    note.deleted_at = Some(now());
                    self.write_note(&note).await?;
                }
                tracing::debug!(%id, "Applied remote note delete");
            }
            // Notebooks
            Change::NotebookCreate { notebook } | Change::NotebookUpdate { notebook } => {
                self.write_json(&self.notebook_path(notebook.id), &notebook)
                    .await?;
                tracing::debug!(id = %notebook.id, "Applied remote notebook change");
            }
            Change::NotebookDelete { id } => {
                let path = self.notebook_path(id);
                if path.exists() {
                    let mut nb: Notebook = self.read_json(&path, id).await?;
                    nb.deleted_at = Some(now());
                    self.write_json(&path, &nb).await?;
                }
                tracing::debug!(%id, "Applied remote notebook delete");
            }
            // Tags
            Change::TagCreate { tag } | Change::TagUpdate { tag } => {
                self.write_json(&self.tag_path(tag.id), &tag).await?;
                tracing::debug!(id = %tag.id, "Applied remote tag change");
            }
            Change::TagDelete { id } => {
                let path = self.tag_path(id);
                if path.exists() {
                    let mut t: Tag = self.read_json(&path, id).await?;
                    t.deleted_at = Some(now());
                    self.write_json(&path, &t).await?;
                }
                tracing::debug!(%id, "Applied remote tag delete");
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
                self.write_json(&self.resource_meta_path(resource.id), &resource)
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
        let path = self.root.join(".keeplin").join("sync_state.json");
        if !path.exists() {
            return Ok(DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default());
        }
        let raw = tokio::fs::read_to_string(path).await?;
        let state: SyncState = serde_json::from_str(&raw)?;
        Ok(state.last_sync)
    }

    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        let state = SyncState { last_sync: ts };
        let path = self.root.join(".keeplin").join("sync_state.json");
        self.write_json(&path, &state).await
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
        let entries = self.read_new_entries().await?;
        let changes = entries
            .into_iter()
            .filter_map(|e| {
                let result = log_entry_to_change(e);
                if result.is_none() {
                    tracing::warn!("Skipped unrecognised log entry in receive_changes");
                }
                result
            })
            .collect();
        Ok(changes)
    }

    async fn get_device_id(&self) -> Result<String, StorageError> {
        Ok(self.device_id.clone())
    }
}
