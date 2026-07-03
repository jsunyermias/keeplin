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
//! **Per-note logs are compacted automatically.** Because each `log.{device}.msgpack` has a
//! single writer, its last entry dominates all earlier ones, so once the log passes
//! [`FsBackend::NOTE_LOG_COMPACT_THRESHOLD`] entries `append_note_op` collapses it to its
//! frontier (the head, plus the newest `Upsert` needed to recover a tombstone winner's
//! content) via [`note_log::compact_own_log`]. This is lossless — `merge` yields the same
//! result — and bounds each per-note per-device log regardless of edit churn.
//!
//! **The global `logs/` journal is compacted too.** Peers track their read position in a
//! foreign log by byte offset, so entries cannot simply be deleted. Instead, once this device's
//! own log passes [`FsBackend::GLOBAL_LOG_COMPACT_THRESHOLD`] entries, `append_log` rewrites it
//! as a **current-state snapshot** (one entry per notebook/tag/resource/association) behind a
//! bumped generation-epoch header (its first line). A peer notices the epoch changed and re-reads
//! the snapshot from the start; because every entry is version-vector resolved and idempotent,
//! replaying the snapshot converges rather than duplicating or resurrecting state. This bounds
//! the log by entity count rather than by mutation count. `prune_change_journal` stays a no-op —
//! compaction, not time-based deletion, does the bounding.

use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{new_id, now, Change, Note, NoteTag, Notebook, Resource, Tag},
};

use super::note_log::{self, resolve, NoteLogEntry, NoteOp, VersionVector, Winner};
use super::{
    NoteRepository, NotebookRepository, NotebookSortProfile, ResourceRepository, SortableRfc3339,
    SyncBackend, TagRepository,
};

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

/// The listing/ordering metadata of one **live** note, held by the in-memory
/// [`NoteMetaIndex`]. Deliberately tiny — no title or body — so the whole index is bounded
/// by the note count, not the corpus size.
#[derive(Debug, Clone)]
struct NoteMetaEntry {
    notebook_id: Uuid,
    created_at: DateTime<Utc>,
    /// The note's [`Note::effective_sort_key`] (the legacy `0` sentinel already mapped).
    effective_sort_key: u32,
    is_starred: bool,
}

impl NoteMetaEntry {
    fn from_note(note: &Note) -> Self {
        Self {
            notebook_id: note.notebook_id,
            created_at: note.created_at,
            effective_sort_key: note.effective_sort_key(),
            is_starred: note.is_starred,
        }
    }
}

/// In-memory index of every **live** note's listing metadata, so `list_notes`,
/// `list_notes_in_notebook`, `list_starred_notes`, and `notebook_sort_profile` can select,
/// order, and paginate without re-merging every note's per-device logs on each call.
///
/// It maps `note_id -> `[`NoteMetaEntry`]. It is built lazily on the first listing (from the
/// cheap `meta.msgpack` projections, falling back to a full merge only for a note that has
/// no projection yet), then maintained incrementally: every write path funnels through
/// [`FsBackend::persist_note_projection`], which updates the index right after it rewrites
/// the projection, so a local edit is reflected immediately and a sync-applied change is
/// reflected as soon as its cycle materializes it. A tombstoned note is dropped from the
/// index (listings exclude soft-deleted notes).
///
/// **Freshness.** Listings therefore reflect the last *materialized* state — exactly what
/// the on-disk projections hold — which is updated on every local write and every sync
/// cycle. A peer edit that Syncthing has replicated but that no sync cycle has processed
/// yet appears in listings only after the next cycle, matching `DbBackend` (whose rows also
/// only change on `apply_change`). Single-note `read_note` stays a live log merge, so
/// reading a specific note is always current.
#[derive(Debug, Default)]
struct NoteMetaIndex {
    entries: std::collections::HashMap<Uuid, NoteMetaEntry>,
}

impl NoteMetaIndex {
    /// Reflect a note's current state: a live note is (re-)inserted, a tombstoned one is
    /// dropped (listings never include soft-deleted notes).
    fn apply(&mut self, note: &Note) {
        if note.deleted_at.is_some() {
            self.entries.remove(&note.id);
        } else {
            self.entries.insert(note.id, NoteMetaEntry::from_note(note));
        }
    }
}

/// The versioned state of one note↔tag association, stored as the MessagePack contents of
/// `note_tags/{note}/{tag}` (previously an empty marker). `deleted_at: None` means the tag is
/// attached (present); `Some` is a tombstone (detached) kept so a remove can beat a concurrent
/// add through `note_log::resolve`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct NoteTagState {
    updated_at: DateTime<Utc>,
    #[serde(default)]
    deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    vv: VersionVector,
    #[serde(default)]
    last_writer: String,
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

/// The first line of a compacted global log: a generation marker written by
/// [`FsBackend::compact_global_log_locked`]. Its epoch increments on every compaction, so a
/// peer that reads by byte offset can notice the log was rewritten (epoch changed) and re-read
/// the fresh snapshot from the start instead of seeking into stale byte positions.
#[derive(Debug, Serialize, Deserialize)]
struct EpochHeader {
    #[serde(rename = "__keeplin_epoch__")]
    epoch: u64,
}

/// Parse a global-log line as an [`EpochHeader`], returning its epoch. Returns `None` for a
/// normal [`LogEntry`] line (which lacks the `__keeplin_epoch__` field), so callers can tell a
/// generation header apart from a change entry.
fn parse_epoch_header(line: &str) -> Option<u64> {
    serde_json::from_str::<EpochHeader>(line)
        .ok()
        .map(|h| h.epoch)
}

/// Build the global-log `data` payload for a notebook/tag delete: the tombstone timestamp plus
/// the deleting write's version vector and author, so `log_entry_to_change` can reconstruct a
/// delete `Change` carrying everything `note_log::resolve` needs on the receiving device.
fn fs_tombstone_value(
    deleted_at: DateTime<Utc>,
    vv: &VersionVector,
    last_writer: &str,
) -> serde_json::Value {
    serde_json::json!({
        "deleted_at": deleted_at,
        "vv": vv,
        "last_writer": last_writer,
    })
}

/// Build the global-log `data` payload for a note↔tag add/remove: the tag id plus the
/// association's version metadata, so `log_entry_to_change` reconstructs a versioned change.
fn fs_assoc_value(
    tag_id: Uuid,
    updated_at: DateTime<Utc>,
    vv: &VersionVector,
    last_writer: &str,
) -> serde_json::Value {
    serde_json::json!({
        "tag_id": tag_id,
        "updated_at": updated_at,
        "vv": vv,
        "last_writer": last_writer,
    })
}

/// Build a snapshot [`LogEntry`] for a notebook/tag/resource by deserialising its MessagePack
/// sidecar into the concrete type `T` and re-serialising through `serde_json` — the same encoding
/// `append_log` uses for a live entity, so the entry round-trips back through
/// [`log_entry_to_change`] identically. Returns `None` if the sidecar cannot be decoded.
fn snapshot_entry_from_sidecar<T: serde::Serialize + serde::de::DeserializeOwned>(
    bytes: &[u8],
    kind: &str,
    id: Uuid,
    ts: DateTime<Utc>,
) -> Option<LogEntry> {
    let concrete: T = rmp_serde::from_slice(bytes).ok()?;
    let value = serde_json::to_value(&concrete).ok()?;
    Some(snapshot_entry_from_value(kind, id, ts, value))
}

/// Build a snapshot [`LogEntry`] from an entity's JSON value (a `serde_json::to_value` of the
/// concrete record). A live entity becomes a `create` carrying the full record; a soft-deleted
/// one becomes a `delete` tombstone carrying `(deleted_at, vv, last_writer)` — exactly the shapes
/// [`log_entry_to_change`] already reconstructs.
fn snapshot_entry_from_value(
    kind: &str,
    id: Uuid,
    ts: DateTime<Utc>,
    value: serde_json::Value,
) -> LogEntry {
    let deleted_at = value
        .get("deleted_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok());
    match deleted_at {
        Some(del) => {
            let vv = value
                .get("vv")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let last_writer = value
                .get("last_writer")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            LogEntry {
                timestamp: ts,
                entity_type: kind.to_string(),
                entity_id: id,
                operation: "delete".to_string(),
                data: fs_tombstone_value(del, &vv, &last_writer),
            }
        }
        None => LogEntry {
            timestamp: ts,
            entity_type: kind.to_string(),
            entity_id: id,
            operation: "create".to_string(),
            data: value,
        },
    }
}

/// Reconstruct an association change's `(updated_at, vv, last_writer)` from a global-log `data`
/// value. Falls back to the entry timestamp and an empty vector for pre-version records.
fn fs_assoc_from_data(
    data: &serde_json::Value,
    fallback_ts: DateTime<Utc>,
) -> (DateTime<Utc>, VersionVector, String) {
    let updated_at = data
        .get("updated_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(fallback_ts);
    let vv = data
        .get("vv")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let last_writer = data
        .get("last_writer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (updated_at, vv, last_writer)
}

/// Reconstruct a notebook/tag delete's `(deleted_at, vv, last_writer)` from a global-log `data`
/// value. Falls back to the log entry's own timestamp and an empty vector for pre-VV records
/// that stored `{ "id": … }`.
fn fs_tombstone_from_data(
    data: &serde_json::Value,
    fallback_ts: DateTime<Utc>,
) -> (DateTime<Utc>, VersionVector, String) {
    let deleted_at = data
        .get("deleted_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(fallback_ts);
    let vv = data
        .get("vv")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let last_writer = data
        .get("last_writer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (deleted_at, vv, last_writer)
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
        ("note", "delete") | ("note", "note_delete") => Some(Change::NoteDelete {
            id,
            deleted_at: ts,
            vv: VersionVector::new(),
            last_writer: String::new(),
        }),
        // Notebooks
        ("notebook", "create") => serde_json::from_value(entry.data)
            .ok()
            .map(|notebook| Change::NotebookCreate { notebook }),
        ("notebook", "update") => serde_json::from_value(entry.data)
            .ok()
            .map(|notebook| Change::NotebookUpdate { notebook }),
        ("notebook", "delete") => {
            let (deleted_at, vv, last_writer) = fs_tombstone_from_data(&entry.data, ts);
            Some(Change::NotebookDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            })
        }
        // Tags
        ("tag", "create") => serde_json::from_value(entry.data)
            .ok()
            .map(|tag| Change::TagCreate { tag }),
        ("tag", "update") => serde_json::from_value(entry.data)
            .ok()
            .map(|tag| Change::TagUpdate { tag }),
        ("tag", "delete") => {
            let (deleted_at, vv, last_writer) = fs_tombstone_from_data(&entry.data, ts);
            Some(Change::TagDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            })
        }
        // NoteTag associations store only the secondary key in the `data` field
        // because the primary key (note_id) is already captured by `entity_id`.
        ("note_tag", "add") => {
            let tag_id: Uuid = entry.data["tag_id"].as_str()?.parse().ok()?;
            let (updated_at, vv, last_writer) = fs_assoc_from_data(&entry.data, ts);
            Some(Change::NoteTagAdd {
                note_id: id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            })
        }
        ("note_tag", "remove") => {
            let tag_id: Uuid = entry.data["tag_id"].as_str()?.parse().ok()?;
            let (updated_at, vv, last_writer) = fs_assoc_from_data(&entry.data, ts);
            Some(Change::NoteTagRemove {
                note_id: id,
                tag_id,
                updated_at,
                vv,
                last_writer,
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
        ("resource", "delete") => {
            let (deleted_at, vv, last_writer) = fs_tombstone_from_data(&entry.data, ts);
            Some(Change::ResourceDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            })
        }
        _ => None,
    }
}

// ── Atomic file writes ────────────────────────────────────────────────────────

/// Write `bytes` to `path` atomically: write a sibling `{path}.tmp`, fsync it, then rename
/// it over the destination.
///
/// - The destination is only ever touched by the rename, so a reader can never observe a
///   half-written file and a failed write (disk full, I/O error) leaves the previous
///   contents intact.
/// - The fsync before the rename closes the power-loss window in which the rename is
///   persisted but the data is not — without it, a crash could replace a good file with
///   an empty or truncated one.
/// - On any failure the temp file is best-effort removed, so failed writes do not
///   accumulate `*.tmp` litter (a crash can still orphan one; see
///   [`FsBackend::sweep_orphan_tmp_files`]).
async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let tmp = path.with_extension("tmp");
    let result = async {
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&tmp, path).await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
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
/// Data is stored as files under the following directory tree:
/// ```text
/// {root}/
///   notes/{uuid}/note.md              — materialized note body (ciphertext when encrypted)
///   notes/{uuid}/meta.msgpack         — materialized metadata + merged version vector (cache)
///   notes/{uuid}/log.{device_id}.msgpack — that device's append-only note op log (source of truth)
///   notebooks/{uuid}.msgpack          — notebook metadata sidecar
///   tags/{uuid}.msgpack               — tag metadata sidecar
///   note_tags/{note_uuid}/{tag_uuid}  — empty sentinel file for each association
///   resources/{uuid}/meta.msgpack     — resource metadata (title, MIME type, file name, size)
///   resources/{uuid}/data             — raw binary payload
///   logs/{device_id}.log              — this device's global NDJSON change log (optional epoch header line)
///   .keeplin/device_id                — persisted UUID that identifies this installation
///   .keeplin/format_version           — integer version stamp written on every startup
///   .keeplin/sync_state.json          — last-sync timestamp
///   .keeplin/offsets/{device_id}      — "{epoch}:{offset}" cursor for each foreign log file
/// ```
///
/// Notes use per-device operation logs merged by version vector (see
/// [`crate::storage::note_log`]); `note.md` / `meta.msgpack` are regenerated projections,
/// never the source of truth. Notebooks, tags, and resources use one MessagePack sidecar
/// each plus the global NDJSON `logs/` journal.
///
/// Syncthing (or any equivalent tool) replicates the entire `{root}` tree to other
/// devices. Because every log file has a single writer it never produces conflict copies.
/// When a foreign device's global log appears under `{root}/logs/`, `receive_changes` reads
/// new entries starting from the stored byte-offset cursor and advances it so each entry is
/// processed exactly once.
pub struct FsBackend {
    /// The root directory of the storage tree.
    root: PathBuf,
    /// The UUID string that uniquely identifies this device's log file. It is read from
    /// `.keeplin/device_id` on startup, or generated and persisted if the file does not
    /// yet exist.
    device_id: String,
    /// Serialises this device's note-log mutations. A per-note log
    /// (`log.{device_id}.msgpack`) is updated read-modify-write, then written via an atomic
    /// temp-then-rename; without this lock, two concurrent writes to the same note read the
    /// same log and the second rename overwrites the first, silently dropping an entry. The
    /// version-vector model assumes a single writer per device log, which concurrent daemon
    /// tasks would otherwise violate. One global mutex (rather than per-note) keeps it simple;
    /// note writes are infrequent enough in offline mode that the reduced parallelism is fine.
    /// Reads need no lock: the atomic rename gives them a consistent view of each log file.
    note_write_lock: Arc<Mutex<()>>,
    /// Serialises this device's global-log (`logs/{device}.log`) mutations. `append_log` appends
    /// under this lock and then may compact the log to a snapshot (a full read-then-rewrite);
    /// without the lock a concurrent append could be lost when compaction replaces the file, or
    /// two compactions could race. Peers only ever read foreign logs, so they need no lock.
    global_log_lock: Arc<Mutex<()>>,
    /// Lazily built [`NoteMetaIndex`] backing the note-listing queries. `None` until the
    /// first listing builds it; thereafter maintained in place by
    /// [`persist_note_projection`](Self::persist_note_projection). An `RwLock` so concurrent
    /// listings share read access and only a write path (or the one-time build) takes it
    /// exclusively.
    note_index: Arc<RwLock<Option<NoteMetaIndex>>>,
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

        let removed = Self::sweep_orphan_tmp_files(&root).await;
        if removed > 0 {
            tracing::info!(
                removed,
                "Removed orphaned .tmp files left by interrupted writes"
            );
        }

        let conflicts = Self::scan_sync_conflicts(&root).await;
        if !conflicts.is_empty() {
            for path in &conflicts {
                tracing::error!(path = %path.display(), "Syncthing conflict copy detected");
            }
            tracing::error!(
                count = conflicts.len(),
                "Syncthing '*.sync-conflict-*' files exist in this store. Every keeplin \
                 file has a single writer, so conflict copies mean two devices are \
                 fighting over the same files — almost always because `.keeplin/` (this \
                 device's identity) was replicated instead of excluded via .stignore. \
                 Fix the Syncthing ignore rules (see README, 'Multi-device setup with \
                 Syncthing'), then reconcile each conflict copy manually before trusting \
                 further writes."
            );
        }

        let (device_id, fresh) = Self::read_or_create_device_id(&root).await?;
        let backend = Self {
            root,
            device_id,
            note_write_lock: Arc::new(Mutex::new(())),
            global_log_lock: Arc::new(Mutex::new(())),
            note_index: Arc::new(RwLock::new(None)),
        };
        backend.ensure_format_version(fresh).await?;
        Ok(backend)
    }

    /// Best-effort startup sweep of orphaned `*.tmp` files left behind when an
    /// [`atomic_write`] was interrupted between creating its temp file and renaming it
    /// (crash, kill, disk-full on an older build without failure cleanup). Returns how
    /// many files were removed.
    ///
    /// Only regular files ending in `.tmp` inside keeplin-managed directories are
    /// touched, and Syncthing's own in-flight temporaries (`.syncthing.*.tmp` — an
    /// unfinished transfer, not garbage) are explicitly left alone. Errors are ignored:
    /// the sweep is hygiene, never a startup blocker, and anything it misses is retried
    /// on the next start.
    async fn sweep_orphan_tmp_files(root: &Path) -> usize {
        let mut removed = 0usize;
        // Directories whose files are written atomically. `notes/`, `note_tags/`, and
        // `resources/` hold one subdirectory per entity, so their entries are swept one
        // level down; the rest hold the target files directly.
        for flat in ["notebooks", "tags", "logs", ".keeplin", ".keeplin/offsets"] {
            removed += Self::sweep_tmp_in_dir(&root.join(flat)).await;
        }
        for nested in ["notes", "note_tags", "resources"] {
            let Ok(mut rd) = tokio::fs::read_dir(root.join(nested)).await else {
                continue;
            };
            while let Ok(Some(entry)) = rd.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    removed += Self::sweep_tmp_in_dir(&entry.path()).await;
                }
            }
        }
        removed
    }

    /// Collect every `*.sync-conflict-*` file in the keeplin-managed directories (and the
    /// store root). Such files are produced by Syncthing when two devices modify the same
    /// file — which, in a store whose every file has a single writer, is the signature of
    /// a replicated `.keeplin/` directory (two devices sharing one identity). The scan is
    /// read-only and best-effort: nothing is deleted (the copies may hold the only good
    /// version of the data) and errors are ignored; the caller logs the findings.
    async fn scan_sync_conflicts(root: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut dirs: Vec<PathBuf> = [
            "",
            "notebooks",
            "tags",
            "logs",
            ".keeplin",
            ".keeplin/offsets",
        ]
        .iter()
        .map(|d| root.join(d))
        .collect();
        for nested in ["notes", "note_tags", "resources"] {
            let Ok(mut rd) = tokio::fs::read_dir(root.join(nested)).await else {
                continue;
            };
            while let Ok(Some(entry)) = rd.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    dirs.push(entry.path());
                }
            }
        }
        for dir in dirs {
            let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
                continue;
            };
            while let Ok(Some(entry)) = rd.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.contains(".sync-conflict-")
                    && entry
                        .file_type()
                        .await
                        .map(|t| t.is_file())
                        .unwrap_or(false)
                {
                    found.push(entry.path());
                }
            }
        }
        found
    }

    /// Remove every orphaned `*.tmp` regular file directly inside `dir` (non-recursive),
    /// skipping Syncthing temporaries. Returns the number removed; all errors ignored.
    async fn sweep_tmp_in_dir(dir: &Path) -> usize {
        let mut removed = 0usize;
        let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
            return 0;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".tmp") || name.starts_with(".syncthing.") {
                continue;
            }
            if !entry
                .file_type()
                .await
                .map(|t| t.is_file())
                .unwrap_or(false)
            {
                continue;
            }
            if tokio::fs::remove_file(entry.path()).await.is_ok() {
                tracing::debug!(path = %entry.path().display(), "Removed orphaned temp file");
                removed += 1;
            }
        }
        removed
    }

    // ── Format version ────────────────────────────────────────────────────────

    /// The current on-disk storage format version.
    ///
    /// Increment this constant when a structural change to the directory layout or
    /// JSON schemas is made that requires an explicit data-migration step. Minor
    /// additions that are handled transparently by serde (such as new optional fields
    /// or serde aliases) do not require a version bump.
    const FORMAT_VERSION: u32 = 5;

    /// How many entries a single device's per-note log may reach before [`append_note_op`]
    /// compacts it back to its frontier via [`note_log::compact_own_log`]. Chosen well above
    /// the handful of entries a note accrues in normal editing, so compaction (a full log
    /// rewrite) is rare while still bounding pathological edit churn on one note.
    const NOTE_LOG_COMPACT_THRESHOLD: usize = 256;

    /// How many entries this device's **global** NDJSON log (`logs/{device}.log`) may reach
    /// before [`append_log`] rewrites it as a current-state snapshot (see
    /// [`compact_global_log_locked`]). Notebooks/tags/resources/associations each collapse to
    /// one snapshot entry regardless of how many times they were edited, so this bounds the log
    /// by entity count rather than by mutation count.
    const GLOBAL_LOG_COMPACT_THRESHOLD: usize = 512;

    /// Only count the global log's entries (to decide whether to compact) once the file exceeds
    /// this many bytes. A `metadata` size check is far cheaper than reading and counting lines,
    /// and no store with fewer than [`GLOBAL_LOG_COMPACT_THRESHOLD`] entries can reach this size,
    /// so the count (and any compaction) is skipped entirely for small logs.
    const GLOBAL_LOG_SOFT_BYTES: u64 = 64 * 1024;

    /// Returns the path of the format-version stamp file: `.keeplin/format_version`.
    fn format_version_path(&self) -> PathBuf {
        self.root.join(".keeplin").join("format_version")
    }

    /// Bring the on-disk store up to [`FORMAT_VERSION`], applying each outstanding migration
    /// step in order and stamping `.keeplin/format_version` **after each one**, so a crash
    /// mid-ladder resumes from the last completed step rather than re-running it.
    ///
    /// `fresh` (from [`read_or_create_device_id`](Self::read_or_create_device_id)) is `true`
    /// for a brand-new store: there is no prior data to migrate, so it is stamped directly at
    /// the current version and no migration step runs — important once a future step does real
    /// work that must not touch an empty store.
    ///
    /// An existing store with **no** stamp file is treated as format `1` (the layout predates
    /// the stamp). A stamp **newer** than this build is rejected rather than opened, so a
    /// downgrade cannot run against a layout it does not understand.
    async fn ensure_format_version(&self, fresh: bool) -> Result<(), StorageError> {
        let path = self.format_version_path();

        if fresh {
            // Nothing to migrate — record the current version as the store's starting point.
            tokio::fs::write(&path, Self::FORMAT_VERSION.to_string()).await?;
            return Ok(());
        }

        let current = if path.exists() {
            tokio::fs::read_to_string(&path)
                .await?
                .trim()
                .parse::<u32>()
                .unwrap_or(1)
        } else {
            1
        };

        if current > Self::FORMAT_VERSION {
            return Err(StorageError::InvalidState(format!(
                "on-disk format version {current} is newer than this build supports \
                 (max {}); upgrade keeplin to open this store",
                Self::FORMAT_VERSION
            )));
        }

        for version in (current + 1)..=Self::FORMAT_VERSION {
            self.apply_format_migration(version).await?;
            // Stamp after each successful step so an interrupted run resumes correctly.
            tokio::fs::write(&path, version.to_string()).await?;
            tracing::info!(version, "Applied filesystem format migration");
        }

        // Stamp once more so a store already at the current version (empty loop above, e.g. a
        // pre-stamp store that happened to be at `FORMAT_VERSION`) still records it.
        tokio::fs::write(&path, Self::FORMAT_VERSION.to_string()).await?;
        Ok(())
    }

    /// Apply the filesystem migration that advances the store **to** `version`.
    ///
    /// Every format bump so far is backward-compatible at parse time, so these steps are
    /// no-ops that only advance the stamp:
    /// - v1 → v2: `serde(alias = "note_id")` / `serde(default)` on [`LogEntry`] parse old logs
    ///   without renaming fields.
    /// - v3/v4: versioned note↔tag associations and resource tombstones are read through
    ///   `serde(default)` fields on older records.
    /// - v5: a compacted global log gains an [`EpochHeader`] first line and offset cursors gain
    ///   an `epoch:offset` form, both treated as optional by the readers (a pre-v5 log is epoch
    ///   `0`, a bare-integer cursor is `(epoch 0, offset)`).
    ///
    /// A future breaking change gets a real body here; the ladder guarantees it runs exactly
    /// once, in order, on stores that need it.
    async fn apply_format_migration(&self, version: u32) -> Result<(), StorageError> {
        match version {
            2..=5 => Ok(()),
            other => Err(StorageError::InvalidState(format!(
                "no filesystem migration defined for format version {other}"
            ))),
        }
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
    /// Returns `(device_id, fresh)` where `fresh` is `true` when the id was just created —
    /// i.e. this is a brand-new store with no prior data. The device-id file is the first
    /// thing written on init, so its absence is a reliable "never initialised" signal, which
    /// [`ensure_format_version`](Self::ensure_format_version) uses to stamp a new store at the
    /// current version rather than replaying the migration ladder over empty data.
    ///
    /// The device identifier is used as the name of this device's log file
    /// (`{root}/logs/{device_id}.log`) and as the Argon2id salt for
    /// `EncryptedBackend`. It must remain stable across restarts.
    async fn read_or_create_device_id(root: &Path) -> Result<(String, bool), StorageError> {
        let path = root.join(".keeplin").join("device_id");
        if path.exists() {
            let id = tokio::fs::read_to_string(&path).await?;
            Ok((id.trim().to_string(), false))
        } else {
            let id = new_id().to_string();
            tokio::fs::write(&path, &id).await?;
            Ok((id, true))
        }
    }

    // ── Log helpers ───────────────────────────────────────────────────────────

    /// Append a single [`LogEntry`] to this device's NDJSON log file, then compact the log to a
    /// current-state snapshot if it has grown past [`GLOBAL_LOG_COMPACT_THRESHOLD`] entries.
    ///
    /// The append and the (occasional) compaction run under `global_log_lock` so a concurrent
    /// append is never lost to a compaction that replaces the file, and two compactions cannot
    /// race. The entry is one JSON line; the file is opened in append mode.
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
        let _guard = self.global_log_lock.lock().await;
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
        file.flush().await?;
        drop(file);
        self.maybe_compact_global_log_locked().await
    }

    /// Compact this device's global log to a current-state snapshot when it exceeds the entry
    /// threshold. **The caller must hold `global_log_lock`.** A cheap `metadata` size gate skips
    /// the line count entirely for small logs (see [`GLOBAL_LOG_SOFT_BYTES`]).
    async fn maybe_compact_global_log_locked(&self) -> Result<(), StorageError> {
        let path = self.device_log_path();
        let size = match tokio::fs::metadata(&path).await {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        if size < Self::GLOBAL_LOG_SOFT_BYTES {
            return Ok(());
        }
        if self.own_log_entry_count().await? <= Self::GLOBAL_LOG_COMPACT_THRESHOLD {
            return Ok(());
        }
        self.compact_global_log_locked().await
    }

    /// Count the change entries (excluding any generation header and blank lines) in this
    /// device's own global log.
    async fn own_log_entry_count(&self) -> Result<usize, StorageError> {
        let file = match tokio::fs::File::open(self.device_log_path()).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut count = 0usize;
        loop {
            line.clear();
            if reader.read_line(&mut line).await? == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() || parse_epoch_header(trimmed).is_some() {
                continue;
            }
            count += 1;
        }
        Ok(count)
    }

    /// Read the generation epoch stored in this device's own global-log header, or `0` when the
    /// log has no header yet (never compacted).
    async fn read_own_epoch(&self) -> Result<u64, StorageError> {
        let (epoch, _len) = self.read_log_header(&self.device_log_path()).await?;
        Ok(epoch)
    }

    /// Rewrite this device's global log as a snapshot of current entity state behind a bumped
    /// generation header. **The caller must hold `global_log_lock`.**
    ///
    /// Notebooks, tags, resources, and note↔tag associations each collapse to a single entry
    /// carrying their current (versioned) state — a `create`/`add` for a live entity or a
    /// `delete`/`remove` tombstone for a soft-deleted one — so the rewritten log is bounded by
    /// entity count rather than mutation count. Because every entry is version-vector resolved
    /// and idempotent on apply, a peer re-reading the whole snapshot (it will, because the epoch
    /// changed) converges: entities it already has newer are skipped, ones it is behind on are
    /// advanced, and tombstones it never saw are harmless no-ops on absent records.
    ///
    /// **Compaction declines to run while any sidecar is unreadable.** The rewrite destroys
    /// the journal's history, so an entity whose sidecar cannot be decoded would silently
    /// vanish from the snapshot — and a peer that had not yet consumed the old entries
    /// would never learn it existed. Skipping compaction is always safe: the journal keeps
    /// appending (it merely keeps growing) and compaction is retried on later writes, so
    /// repairing or restoring the damaged sidecar re-enables it with nothing lost.
    async fn compact_global_log_locked(&self) -> Result<(), StorageError> {
        let new_epoch = self.read_own_epoch().await?.saturating_add(1);
        let (entries, unreadable) = self.build_global_snapshot().await?;
        if unreadable > 0 {
            tracing::error!(
                unreadable,
                "Global-log compaction skipped: unreadable sidecar file(s) would be \
                 silently dropped from the snapshot (see the errors above for paths). \
                 The journal keeps appending until they are repaired or restored from \
                 a backup or another device."
            );
            return Ok(());
        }

        let mut buf = serde_json::to_string(&EpochHeader { epoch: new_epoch })?;
        buf.push('\n');
        for entry in &entries {
            buf.push_str(&serde_json::to_string(entry)?);
            buf.push('\n');
        }

        atomic_write(&self.device_log_path(), buf.as_bytes()).await?;
        tracing::info!(
            epoch = new_epoch,
            entries = entries.len(),
            "Compacted global change log to a snapshot"
        );
        Ok(())
    }

    /// Build the snapshot entries for [`compact_global_log_locked`]: one [`LogEntry`] per
    /// notebook, tag, resource, and note↔tag association currently on disk, reflecting its
    /// present state (a `create`/`add`, or a `delete`/`remove` tombstone when soft-deleted).
    /// Notes are excluded — they sync through their own per-note version-vector logs, not the
    /// global journal.
    ///
    /// Returns `(entries, unreadable)`: `unreadable` counts sidecar files that exist but
    /// cannot be decoded (each reported at error level with its path). The caller uses a
    /// non-zero count to decline the compaction rather than emit a snapshot with entities
    /// silently missing.
    async fn build_global_snapshot(&self) -> Result<(Vec<LogEntry>, usize), StorageError> {
        let ts = now();
        let mut out = Vec::new();
        let mut unreadable = 0usize;

        // Notebooks and tags: sidecar files under their directories.
        for (dir, kind) in [("notebooks", "notebook"), ("tags", "tag")] {
            let mut rd = match tokio::fs::read_dir(self.root.join(dir)).await {
                Ok(rd) => rd,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            while let Some(e) = rd.next_entry().await? {
                let fname = e.file_name().to_string_lossy().into_owned();
                let Some(stem) = fname.strip_suffix(".msgpack") else {
                    continue;
                };
                let Ok(id) = Uuid::parse_str(stem) else {
                    continue;
                };
                let bytes = tokio::fs::read(e.path()).await?;
                let entry = match kind {
                    "notebook" => snapshot_entry_from_sidecar::<Notebook>(&bytes, kind, id, ts),
                    _ => snapshot_entry_from_sidecar::<Tag>(&bytes, kind, id, ts),
                };
                match entry {
                    Some(e) => out.push(e),
                    None => {
                        unreadable += 1;
                        tracing::error!(
                            path = %e.path().display(),
                            "Unreadable {kind} sidecar; restore it from a backup or \
                             another device (global-log compaction is paused until then)"
                        );
                    }
                }
            }
        }

        // Resources: metadata sidecar inside each resource directory.
        if let Ok(mut rd) = tokio::fs::read_dir(self.root.join("resources")).await {
            while let Some(e) = rd.next_entry().await? {
                let Ok(id) = Uuid::parse_str(&e.file_name().to_string_lossy()) else {
                    continue;
                };
                let meta_path = self.resource_meta_path(id);
                let bytes = match tokio::fs::read(&meta_path).await {
                    Ok(bytes) => bytes,
                    // No metadata at all is an orphan of a crashed create (the data file
                    // is written first) — nothing to snapshot, not corruption.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e.into()),
                };
                match snapshot_entry_from_sidecar::<Resource>(&bytes, "resource", id, ts) {
                    Some(e) => out.push(e),
                    None => {
                        unreadable += 1;
                        tracing::error!(
                            path = %meta_path.display(),
                            "Unreadable resource sidecar; restore it from a backup or \
                             another device (global-log compaction is paused until then)"
                        );
                    }
                }
            }
        }

        // Note↔tag associations: one versioned state file per (note, tag).
        if let Ok(mut notes_rd) = tokio::fs::read_dir(self.root.join("note_tags")).await {
            while let Some(note_entry) = notes_rd.next_entry().await? {
                let Ok(note_id) = Uuid::parse_str(&note_entry.file_name().to_string_lossy()) else {
                    continue;
                };
                let mut tags_rd = tokio::fs::read_dir(note_entry.path()).await?;
                while let Some(tag_entry) = tags_rd.next_entry().await? {
                    let Ok(tag_id) = Uuid::parse_str(&tag_entry.file_name().to_string_lossy())
                    else {
                        continue;
                    };
                    let Some(state) = self.read_assoc_state(&tag_entry.path()).await? else {
                        continue;
                    };
                    let op = if state.deleted_at.is_some() {
                        "remove"
                    } else {
                        "add"
                    };
                    out.push(LogEntry {
                        timestamp: ts,
                        entity_type: "note_tag".to_string(),
                        entity_id: note_id,
                        operation: op.to_string(),
                        data: fs_assoc_value(
                            tag_id,
                            state.updated_at,
                            &state.vv,
                            &state.last_writer,
                        ),
                    });
                }
            }
        }

        Ok((out, unreadable))
    }

    /// Returns the path of the byte-offset cursor file for a foreign device:
    /// `.keeplin/offsets/{device_id}`.
    ///
    /// The file stores `"{epoch}:{offset}"` — the foreign log's generation epoch this cursor was
    /// taken against, and the number of bytes consumed within it. When the foreign log is
    /// compacted its epoch increments, so a stale cursor (older epoch) tells `receive_changes` to
    /// re-read the fresh snapshot from the start instead of seeking into now-invalid byte
    /// positions. A bare integer (pre-v5 cursor) is read as `(epoch 0, offset)`.
    fn log_offset_path(&self, device_id: &str) -> PathBuf {
        self.root.join(".keeplin").join("offsets").join(device_id)
    }

    /// Read the stored `(epoch, byte offset)` cursor for a foreign device log, or `(0, 0)` if no
    /// cursor has been recorded yet. A bare-integer file (pre-v5) is read as `(0, offset)`.
    async fn read_log_offset(&self, device_id: &str) -> (u64, u64) {
        let raw = match tokio::fs::read_to_string(self.log_offset_path(device_id)).await {
            Ok(s) => s,
            Err(_) => return (0, 0),
        };
        let raw = raw.trim();
        match raw.split_once(':') {
            Some((epoch, offset)) => (
                epoch.trim().parse().unwrap_or(0),
                offset.trim().parse().unwrap_or(0),
            ),
            None => (0, raw.parse().unwrap_or(0)),
        }
    }

    /// Persist the `(epoch, byte offset)` cursor for a foreign device log using an atomic
    /// write-then-rename so that a crash during the write cannot leave the cursor file in a
    /// partially-written state. A torn cursor file is interpreted as `(0, 0)` by
    /// `read_log_offset`, causing re-delivery of already-processed entries, which is safe
    /// (apply is idempotent and version-vector resolved) but wasteful.
    async fn write_log_offset(
        &self,
        device_id: &str,
        epoch: u64,
        offset: u64,
    ) -> Result<(), StorageError> {
        atomic_write(
            &self.log_offset_path(device_id),
            format!("{epoch}:{offset}").as_bytes(),
        )
        .await
    }

    /// Read a global log's generation header: `(epoch, header_byte_len)`. A log with no header
    /// (never compacted, or pre-v5) reports `(0, 0)`, so reading starts at byte 0. `header_byte_len`
    /// is the length of the header line **including its newline**, i.e. exactly where the first
    /// change entry begins.
    async fn read_log_header(&self, path: &Path) -> Result<(u64, u64), StorageError> {
        let file = match tokio::fs::File::open(path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((0, 0)),
            Err(e) => return Err(e.into()),
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        match parse_epoch_header(line.trim()) {
            Some(epoch) => Ok((epoch, n as u64)),
            None => Ok((0, 0)),
        }
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
                if trimmed.is_empty() || parse_epoch_header(trimmed).is_some() {
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
    /// **Generation epochs.** If the foreign log's generation epoch (its
    /// [`EpochHeader`] first line) differs from the one this cursor was taken against, the log
    /// was compacted (rewritten as a snapshot), so its old byte offset is meaningless. In that
    /// case reading restarts just past the header and re-delivers the whole snapshot; because
    /// every entry is idempotent and version-vector resolved, replaying it converges rather than
    /// duplicating or resurrecting state.
    ///
    /// If writing the offset file fails (for example due to a disk-full condition),
    /// the error is logged as a warning but does not propagate. The consequence is
    /// that the same entries will be returned again on the next call — safe, since apply is
    /// idempotent.
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
            let (writer_epoch, header_len) = self.read_log_header(&dir_entry.path()).await?;
            let (stored_epoch, stored_offset) = self.read_log_offset(&device_id).await;
            // On a generation change, discard the stale offset and re-read from just past the
            // (new) header; otherwise resume from where we left off within the same generation.
            let start = if writer_epoch != stored_epoch {
                header_len
            } else {
                stored_offset
            };

            let mut file = tokio::fs::File::open(dir_entry.path()).await?;
            file.seek(SeekFrom::Start(start)).await?;

            let mut reader = BufReader::new(file);
            let mut line = String::new();
            let mut new_offset = start;
            loop {
                line.clear();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    break;
                }
                new_offset += n as u64;
                let trimmed = line.trim();
                if trimmed.is_empty() || parse_epoch_header(trimmed).is_some() {
                    continue;
                }
                match serde_json::from_str::<LogEntry>(trimmed) {
                    Ok(e) => entries.push(e),
                    Err(err) => {
                        tracing::warn!("Skipping malformed log line: {err}");
                    }
                }
            }

            if writer_epoch != stored_epoch || new_offset > stored_offset {
                if let Err(e) = self
                    .write_log_offset(&device_id, writer_epoch, new_offset)
                    .await
                {
                    tracing::warn!("Could not save log offset for {device_id}: {e}");
                }
            }
        }
        Ok(entries)
    }

    // ── Generic single-file MessagePack sidecar helpers ───────────────────────

    /// Serialise `value` to MessagePack and write it to `path` via [`atomic_write`], so a
    /// concurrent reader never sees a half-written file and a failed write leaves the
    /// previous contents (and no temp litter) behind.
    async fn write_sidecar<T: serde::Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), StorageError> {
        let bytes = rmp_serde::to_vec_named(value)
            .map_err(|e| StorageError::InvalidState(format!("msgpack encode: {e}")))?;
        atomic_write(path, &bytes).await
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

    /// Read the version vector currently stored in a MessagePack sidecar (notebook/tag), or an
    /// empty vector when the file is absent. Used to base a local write's incremented vector on
    /// the current state, so notebooks/tags resolve conflicts with `note_log::resolve` just like
    /// notes and `DbBackend` do. Deserialises only the `vv` field, ignoring the rest.
    async fn sidecar_vv(&self, path: &Path) -> Result<VersionVector, StorageError> {
        #[derive(serde::Deserialize)]
        struct VvProbe {
            #[serde(default)]
            vv: VersionVector,
        }
        if !path.exists() {
            return Ok(VersionVector::new());
        }
        let bytes = tokio::fs::read(path).await?;
        let probe: VvProbe = rmp_serde::from_slice(&bytes)
            .map_err(|e| StorageError::CorruptedData(format!("msgpack decode: {e}")))?;
        Ok(probe.vv)
    }

    /// Compute the version vector for a **local** notebook/tag write: the current stored vector
    /// (empty for a new sidecar) with this device's component incremented.
    async fn next_sidecar_vv(&self, path: &Path) -> Result<VersionVector, StorageError> {
        let mut vv = self.sidecar_vv(path).await?;
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    /// Whether an incoming remote notebook/tag write should replace the local sidecar, decided
    /// by `note_log::resolve` over the stored and incoming `(vv, updated_at, last_writer)`.
    /// Returns `true` when there is no local sidecar yet.
    async fn sidecar_incoming_wins(
        &self,
        path: &Path,
        incoming_vv: &VersionVector,
        incoming_updated: DateTime<Utc>,
        incoming_writer: &str,
    ) -> Result<bool, StorageError> {
        #[derive(serde::Deserialize)]
        struct MetaProbe {
            updated_at: DateTime<Utc>,
            #[serde(default)]
            vv: VersionVector,
            #[serde(default)]
            last_writer: String,
        }
        if !path.exists() {
            return Ok(true);
        }
        let bytes = tokio::fs::read(path).await?;
        let m: MetaProbe = rmp_serde::from_slice(&bytes)
            .map_err(|e| StorageError::CorruptedData(format!("msgpack decode: {e}")))?;
        Ok(matches!(
            resolve(
                &m.vv,
                m.updated_at,
                &m.last_writer,
                incoming_vv,
                incoming_updated,
                incoming_writer,
            ),
            Winner::Incoming
        ))
    }

    // ── note↔tag association version helpers ──────────────────────────────────

    /// Read the versioned state of a note↔tag association file, or `None` when absent.
    ///
    /// Two degenerate shapes both fall back to a "present, minimum priority" marker
    /// (epoch-0 timestamp, empty vector), but for different reasons:
    ///
    /// - An **empty** file is the pre-versioning marker format (its mere existence encoded
    ///   the association). Reading it as attached-with-weakest-priority is the designed
    ///   backward compatibility: any versioned write is causally newer and must win.
    /// - A **non-empty but unparseable** file is corruption. Its true state (attached or
    ///   tombstoned, and with what vector) is unrecoverable, so the same weakest-priority
    ///   marker is deliberately the least-harm reading: the association stays visible
    ///   locally instead of vanishing, and the next versioned state from any peer — the
    ///   only surviving authoritative copy — supersedes it through `resolve`, which is the
    ///   best available recovery. Unlike the marker case this is **reported at error
    ///   level** (it means local data was damaged), while staying non-fatal so one bad
    ///   association cannot block listing or sync.
    async fn read_assoc_state(&self, path: &Path) -> Result<Option<NoteTagState>, StorageError> {
        if !path.exists() {
            return Ok(None);
        }
        let marker = || NoteTagState {
            updated_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default(),
            deleted_at: None,
            vv: VersionVector::new(),
            last_writer: String::new(),
        };
        let bytes = tokio::fs::read(path).await?;
        if bytes.is_empty() {
            return Ok(Some(marker()));
        }
        match rmp_serde::from_slice(&bytes) {
            Ok(state) => Ok(Some(state)),
            Err(e) => {
                tracing::error!(
                    path = %path.display(),
                    "Unreadable note↔tag association state; treating it as attached with \
                     minimum priority so the next versioned peer state supersedes it \
                     (restore the file from a backup or another device to recover it): {e}"
                );
                Ok(Some(marker()))
            }
        }
    }

    /// Version vector for a **local** association write: current vector (empty if new) with this
    /// device's component incremented.
    async fn next_assoc_vv(&self, path: &Path) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .read_assoc_state(path)
            .await?
            .map(|s| s.vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    /// Whether an incoming association write should replace the local state, via [`resolve`].
    /// `true` when the pair has no local file.
    async fn assoc_incoming_wins(
        &self,
        path: &Path,
        incoming_vv: &VersionVector,
        incoming_updated: DateTime<Utc>,
        incoming_writer: &str,
    ) -> Result<bool, StorageError> {
        match self.read_assoc_state(path).await? {
            None => Ok(true),
            Some(s) => Ok(matches!(
                resolve(
                    &s.vv,
                    s.updated_at,
                    &s.last_writer,
                    incoming_vv,
                    incoming_updated,
                    incoming_writer,
                ),
                Winner::Incoming
            )),
        }
    }

    /// Write an association's versioned state, creating the `note_tags/{note}` directory first.
    async fn write_assoc_state(
        &self,
        note_id: Uuid,
        tag_id: Uuid,
        state: &NoteTagState,
    ) -> Result<(), StorageError> {
        tokio::fs::create_dir_all(self.note_tag_dir(note_id)).await?;
        self.write_sidecar(&self.note_tag_path(note_id, tag_id), state)
            .await
    }

    // ── resource version helpers ──────────────────────────────────────────────

    /// Read a resource's metadata sidecar, or `None` when absent.
    async fn read_resource_meta(&self, id: Uuid) -> Result<Option<Resource>, StorageError> {
        match self
            .read_sidecar::<Resource>(&self.resource_meta_path(id), id)
            .await
        {
            Ok(r) => Ok(Some(r)),
            Err(StorageError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Version vector for a **local** resource write (create or delete): current vector (empty
    /// if new) with this device's component incremented.
    async fn next_resource_vv(&self, id: Uuid) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .read_resource_meta(id)
            .await?
            .map(|r| r.vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    /// Whether an incoming resource change should replace the local metadata, via [`resolve`].
    /// The resource has no `updated_at`, so the tiebreak timestamp is `deleted_at` when
    /// tombstoned else `created_at`. `true` when the resource has no local metadata.
    async fn resource_incoming_wins(
        &self,
        id: Uuid,
        incoming_vv: &VersionVector,
        incoming_ts: DateTime<Utc>,
        incoming_writer: &str,
    ) -> Result<bool, StorageError> {
        match self.read_resource_meta(id).await? {
            None => Ok(true),
            Some(r) => {
                let local_ts = r.deleted_at.unwrap_or(r.created_at);
                Ok(matches!(
                    resolve(
                        &r.vv,
                        local_ts,
                        &r.last_writer,
                        incoming_vv,
                        incoming_ts,
                        incoming_writer,
                    ),
                    Winner::Incoming
                ))
            }
        }
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
    ///
    /// A skipped log means that device's entire history for this note is missing from the
    /// merge — a silent-data-loss risk, not a routine condition — so it is reported at
    /// **error** level with the note id and file path. The file itself is left in place
    /// (never deleted or renamed: it belongs to another device and Syncthing would
    /// replicate a local rename back to its writer), so a copy that recovers or is
    /// restored from backup re-enters the merge on the next read.
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
                    Err(e) => tracing::error!(
                        note_id = %id,
                        path = %entry.path().display(),
                        "Unreadable note log excluded from merge — this device's history \
                         for the note is not being applied; restore the file from a backup \
                         or another device to recover it: {e}"
                    ),
                }
            }
        }
        Ok(logs)
    }

    /// Merge all of a note's per-device logs into its current state **without** touching
    /// disk. Reads use this so that reading a note never rewrites `note.md`/`meta.msgpack`
    /// (avoiding write amplification) and never advances the sync-detection cache — a read
    /// must not consume a peer change that the next sync is supposed to report. Returns the
    /// merged note, or `None` when the note has no log entries at all.
    async fn merge_note(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
        let logs = self.read_note_logs(id).await?;
        Ok(note_log::merge(&logs).note)
    }

    /// Merge all of a note's per-device logs into its current state and refresh the
    /// `note.md` + `meta.msgpack` projection. Returns the merged note, or `None` when the
    /// note has no log entries at all. Used by the write and sync paths (not by reads).
    async fn materialize(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
        let logs = self.read_note_logs(id).await?;
        let merged = note_log::merge(&logs);
        match merged.note {
            None => Ok(None),
            Some(note) => {
                if merged.conflict {
                    tracing::warn!(%id, "Concurrent note edit resolved by the (timestamp, device-id) tiebreak");
                }
                self.persist_note_projection(&note, &merged.vv).await?;
                Ok(Some(note))
            }
        }
    }

    /// Write the projection: the body to `note.md` and the metadata (body blanked, since
    /// it lives in `note.md`) plus the version vector to `meta.msgpack`. Both writes are
    /// atomic temp-then-rename.
    ///
    /// This is the single choke point every note write (local edit or sync apply) passes
    /// through, so it is also where the in-memory [`NoteMetaIndex`] is kept current: the
    /// on-disk projection is written first, then the index entry is updated (only if the
    /// index has already been built — otherwise the eventual build reads the fresh
    /// projection). Doing it in that order means a crash between the two leaves the index
    /// no staler than the projection, and a not-yet-built index misses nothing.
    async fn persist_note_projection(
        &self,
        note: &Note,
        vv: &VersionVector,
    ) -> Result<(), StorageError> {
        tokio::fs::create_dir_all(self.note_dir(note.id)).await?;
        atomic_write(&self.note_md_path(note.id), note.body.as_bytes()).await?;
        let mut meta_note = note.clone();
        meta_note.body = String::new();
        self.write_sidecar(
            &self.note_meta_path(note.id),
            &NoteMeta {
                note: meta_note,
                vv: vv.clone(),
            },
        )
        .await?;
        if let Some(idx) = self.note_index.write().await.as_mut() {
            idx.apply(note);
        }
        Ok(())
    }

    /// Read the current on-disk projection of a note (metadata from `meta.msgpack`, body
    /// from `note.md`), or `None` when it has no projection yet. Used only to build the
    /// [`NoteMetaIndex`] cheaply — one small read instead of merging every per-device log.
    async fn read_note_projection(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
        let meta: NoteMeta = match self.read_sidecar(&self.note_meta_path(id), id).await {
            Ok(m) => m,
            Err(StorageError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(Some(meta.note))
    }

    /// Run `f` against the note index, building it first (one directory scan) when it is
    /// absent. The double-checked write lock means concurrent callers trigger at most one
    /// build.
    async fn with_note_index<R>(
        &self,
        f: impl FnOnce(&NoteMetaIndex) -> R,
    ) -> Result<R, StorageError> {
        {
            let guard = self.note_index.read().await;
            if let Some(idx) = guard.as_ref() {
                return Ok(f(idx));
            }
        }
        let mut guard = self.note_index.write().await;
        if guard.is_none() {
            *guard = Some(self.build_note_index().await?);
        }
        Ok(f(guard.as_ref().expect("index was just built")))
    }

    /// Build the note index by scanning every note directory. Each note's metadata comes
    /// from its `meta.msgpack` projection; a note that has none yet (a peer note whose log
    /// arrived but was never materialized here) falls back to a full merge, so nothing is
    /// missed. Only **live** (non-tombstoned) notes are indexed.
    async fn build_note_index(&self) -> Result<NoteMetaIndex, StorageError> {
        let mut idx = NoteMetaIndex::default();
        let mut dir = match tokio::fs::read_dir(self.root.join("notes")).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(idx),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = dir.next_entry().await? {
            let Ok(id) = Uuid::parse_str(&entry.file_name().to_string_lossy()) else {
                continue;
            };
            let note = match self.read_note_projection(id).await {
                Ok(Some(note)) => Some(note),
                // No projection (or an unreadable one): recover via a full log merge.
                Ok(None) => self.merge_note(id).await?,
                Err(e) => {
                    tracing::warn!("Rebuilding note {id} from logs (projection unreadable): {e}");
                    self.merge_note(id).await?
                }
            };
            if let Some(note) = note {
                if note.deleted_at.is_none() {
                    idx.apply(&note);
                }
            }
        }
        Ok(idx)
    }

    /// Merge the given note ids into full notes for a listing page, skipping any that no
    /// longer merge to a live note (a race with a concurrent delete/move). Page-bounded, so
    /// the per-note merge cost is paid only for the returned page, never the whole store.
    async fn materialize_page(&self, ids: Vec<Uuid>) -> Result<Vec<Note>, StorageError> {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            match self.merge_note(id).await {
                Ok(Some(n)) if n.deleted_at.is_none() => out.push(n),
                Ok(_) => {}
                Err(e) => tracing::warn!("Could not merge note {id} for listing: {e}"),
            }
        }
        Ok(out)
    }

    /// Append an operation to this device's note log, then re-materialize the note and
    /// return the merged result. This is the single entry point for every local note
    /// mutation (create, update, delete).
    async fn append_note_op(&self, id: Uuid, op: NoteOp) -> Result<Note, StorageError> {
        // Serialise the whole read-log → append → write-log critical section so a concurrent
        // writer cannot overwrite this entry (see `note_write_lock`).
        let _write_guard = self.note_write_lock.lock().await;
        tokio::fs::create_dir_all(self.note_dir(id)).await?;
        // Base the new entry's version vector on the merge of every log currently on disk
        // (not on the `meta.msgpack` cache), so an edit causally follows all state present at
        // write time even though reads no longer refresh that cache. This keeps the
        // "read-then-edit dominates" behaviour without letting reads write to disk.
        let mut vv = note_log::merge(&self.read_note_logs(id).await?).vv;
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
        // Bound this device's own note log: once it grows past the threshold, collapse it to
        // its frontier (see `note_log::compact_own_log`). Safe because this log has a single
        // writer, so its last entry dominates all earlier ones and `merge` is unaffected.
        if log.len() > Self::NOTE_LOG_COMPACT_THRESHOLD {
            log = note_log::compact_own_log(&log);
        }
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
                        Some(deleted_at) => changes.push(Change::NoteDelete {
                            id,
                            deleted_at,
                            vv: merged.vv.clone(),
                            last_writer: String::new(),
                        }),
                        None => changes.push(Change::NoteUpdate { note }),
                    }
                }
            }
        }
        Ok(changes)
    }
}

// ── Streaming pagination helper ───────────────────────────────────────────────

/// An item tagged with its `(created_at_rfc3339, id)` pagination key, ordered by that key
/// alone so [`PageCollector`]'s max-heap can evict the largest candidate.
struct KeyedItem<T> {
    key: (String, Uuid),
    item: T,
}

impl<T> PartialEq for KeyedItem<T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl<T> Eq for KeyedItem<T> {}
impl<T> PartialOrd for KeyedItem<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for KeyedItem<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

/// Streaming replacement for collect-everything-then-[`paginate`]: feed it every candidate
/// item and it retains only the `limit + 1` smallest `(created_at, id)` keys past the
/// cursor (a max-heap evicts the largest), so building one page holds O(page) items in
/// memory instead of the whole store. The `+ 1` overflow slot is how it learns whether a
/// next page exists. Cursor semantics and the produced token match [`paginate`] exactly.
struct PageCollector<T> {
    limit: usize,
    cursor: Option<(String, Uuid)>,
    heap: std::collections::BinaryHeap<KeyedItem<T>>,
}

impl<T> PageCollector<T> {
    /// `token` is the `"<created_at_rfc3339>|<uuid>"` cursor of the previous page's last
    /// item; `None`, empty, or unparseable tokens start from the first item (as
    /// [`paginate`] does).
    fn new(limit: usize, token: Option<&str>) -> Self {
        let cursor = token
            .filter(|t| !t.is_empty())
            .and_then(|t| t.split_once('|'))
            .and_then(|(ts, id)| Uuid::parse_str(id).ok().map(|id| (ts.to_string(), id)));
        Self {
            limit,
            cursor,
            heap: std::collections::BinaryHeap::with_capacity(limit + 2),
        }
    }

    /// Offer one candidate. Items at or before the cursor are skipped; the rest compete
    /// for the `limit + 1` retained slots.
    fn push(&mut self, key: (String, Uuid), item: T) {
        if let Some(cursor) = &self.cursor {
            // Same predicate as `paginate`'s partition_point: skip keys <= the cursor pair.
            if (key.0.as_str(), key.1) <= (cursor.0.as_str(), cursor.1) {
                return;
            }
        }
        if self.heap.len() <= self.limit {
            self.heap.push(KeyedItem { key, item });
        } else if let Some(top) = self.heap.peek() {
            if key < top.key {
                self.heap.pop();
                self.heap.push(KeyedItem { key, item });
            }
        }
    }

    /// Produce `(page, next_token)`: the retained items in ascending key order, trimmed to
    /// `limit`, with a cursor when the overflow slot proved more items exist.
    fn into_page(self) -> (Vec<T>, Option<String>) {
        let mut items = self.heap.into_sorted_vec();
        let has_more = items.len() > self.limit;
        items.truncate(self.limit);
        let next_token = if has_more {
            items
                .last()
                .map(|last| format!("{}|{}", last.key.0, last.key.1))
        } else {
            None
        };
        (items.into_iter().map(|k| k.item).collect(), next_token)
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
        // Reads merge live from the per-device logs, so they always reflect the latest state
        // — even immediately after Syncthing brings in a peer's log — without writing the
        // projection back (a read never mutates the store).
        self.merge_note(id)
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
        let limit = super::effective_page_size(page_size) as usize;
        // Select and paginate the page's ids from the in-memory index (no per-note log
        // merge), then materialize only that page. Ordered by `(created_at, id)`.
        let (ids, next) = self
            .with_note_index(|idx| {
                let mut collector = PageCollector::new(limit, page_token.as_deref());
                for (id, entry) in &idx.entries {
                    collector.push((entry.created_at.to_sortable_rfc3339(), *id), *id);
                }
                collector.into_page()
            })
            .await?;
        Ok((self.materialize_page(ids).await?, next))
    }

    async fn list_notes_in_notebook(
        &self,
        notebook_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        // Keyed by the notebook's manual order: the effective sort key, zero-padded so its
        // lexicographic order (how `PageCollector` compares) is numeric.
        let (ids, next) = self
            .with_note_index(|idx| {
                let mut collector = PageCollector::new(limit, page_token.as_deref());
                for (id, entry) in &idx.entries {
                    if entry.notebook_id == notebook_id {
                        collector.push((format!("{:010}", entry.effective_sort_key), *id), *id);
                    }
                }
                collector.into_page()
            })
            .await?;
        Ok((self.materialize_page(ids).await?, next))
    }

    async fn list_starred_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let (ids, next) = self
            .with_note_index(|idx| {
                let mut collector = PageCollector::new(limit, page_token.as_deref());
                for (id, entry) in &idx.entries {
                    if entry.is_starred {
                        collector.push((entry.created_at.to_sortable_rfc3339(), *id), *id);
                    }
                }
                collector.into_page()
            })
            .await?;
        Ok((self.materialize_page(ids).await?, next))
    }

    async fn notebook_sort_profile(
        &self,
        notebook_id: Uuid,
    ) -> Result<NotebookSortProfile, StorageError> {
        let keys = self
            .with_note_index(|idx| {
                idx.entries
                    .values()
                    .filter(|e| e.notebook_id == notebook_id)
                    .map(|e| e.effective_sort_key)
                    .collect::<Vec<u32>>()
            })
            .await?;
        Ok(NotebookSortProfile::from_effective_keys(keys))
    }
}

// ── NotebookRepository impl ───────────────────────────────────────────────────

#[async_trait]
impl NotebookRepository for FsBackend {
    async fn create_notebook(&self, mut notebook: Notebook) -> Result<Notebook, StorageError> {
        let path = self.notebook_path(notebook.id);
        notebook.vv = self.next_sidecar_vv(&path).await?;
        notebook.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &notebook).await?;
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

    async fn update_notebook(&self, mut notebook: Notebook) -> Result<Notebook, StorageError> {
        let path = self.notebook_path(notebook.id);
        if !path.exists() {
            return Err(StorageError::NotFound(notebook.id.to_string()));
        }
        notebook.vv = self.next_sidecar_vv(&path).await?;
        notebook.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &notebook).await?;
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
        let path = self.notebook_path(id);
        let mut nb: Notebook = self.read_sidecar(&path, id).await?;
        let ts = now();
        nb.deleted_at = Some(ts);
        nb.updated_at = ts;
        note_log::increment(&mut nb.vv, &self.device_id);
        nb.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &nb).await?;
        self.append_log(
            "notebook",
            id,
            "delete",
            fs_tombstone_value(ts, &nb.vv, &nb.last_writer),
        )
        .await?;
        tracing::info!(%id, "Notebook deleted");
        Ok(())
    }

    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let mut notebooks = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("notebooks")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(stem) = fname.strip_suffix(".msgpack") {
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
            (nb.created_at.to_sortable_rfc3339(), nb.id)
        }))
    }
}

// ── TagRepository impl ────────────────────────────────────────────────────────

#[async_trait]
impl TagRepository for FsBackend {
    async fn create_tag(&self, mut tag: Tag) -> Result<Tag, StorageError> {
        let path = self.tag_path(tag.id);
        tag.vv = self.next_sidecar_vv(&path).await?;
        tag.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &tag).await?;
        self.append_log("tag", tag.id, "create", serde_json::to_value(&tag)?)
            .await?;
        tracing::info!(id = %tag.id, "Tag created");
        Ok(tag)
    }

    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        self.read_sidecar(&self.tag_path(id), id).await
    }

    async fn update_tag(&self, mut tag: Tag) -> Result<Tag, StorageError> {
        let path = self.tag_path(tag.id);
        if !path.exists() {
            return Err(StorageError::NotFound(tag.id.to_string()));
        }
        tag.vv = self.next_sidecar_vv(&path).await?;
        tag.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &tag).await?;
        self.append_log("tag", tag.id, "update", serde_json::to_value(&tag)?)
            .await?;
        tracing::info!(id = %tag.id, "Tag updated");
        Ok(tag)
    }

    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        let path = self.tag_path(id);
        let mut tag: Tag = self.read_sidecar(&path, id).await?;
        let ts = now();
        tag.deleted_at = Some(ts);
        tag.updated_at = ts;
        note_log::increment(&mut tag.vv, &self.device_id);
        tag.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &tag).await?;
        self.append_log(
            "tag",
            id,
            "delete",
            fs_tombstone_value(ts, &tag.vv, &tag.last_writer),
        )
        .await?;
        tracing::info!(%id, "Tag deleted");
        Ok(())
    }

    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let mut tags = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("tags")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(stem) = fname.strip_suffix(".msgpack") {
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
            (t.created_at.to_sortable_rfc3339(), t.id)
        }))
    }

    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        // Both ends must exist and be live: the API must not create a dangling
        // association. `apply_change` deliberately skips this check — sync delivery
        // order is not guaranteed, so an association may arrive before its note or tag.
        match self.merge_note(note_tag.note_id).await? {
            Some(n) if n.deleted_at.is_none() => {}
            _ => return Err(StorageError::NotFound(note_tag.note_id.to_string())),
        }
        let tag: Tag = self
            .read_sidecar(&self.tag_path(note_tag.tag_id), note_tag.tag_id)
            .await?;
        if tag.deleted_at.is_some() {
            return Err(StorageError::NotFound(note_tag.tag_id.to_string()));
        }
        let path = self.note_tag_path(note_tag.note_id, note_tag.tag_id);
        let vv = self.next_assoc_vv(&path).await?;
        let ts = now();
        // An add is the association's *present* state (deleted_at = None), versioned so a
        // concurrent add-vs-remove converges through `resolve`.
        let state = NoteTagState {
            updated_at: ts,
            deleted_at: None,
            vv: vv.clone(),
            last_writer: self.device_id.clone(),
        };
        self.write_assoc_state(note_tag.note_id, note_tag.tag_id, &state)
            .await?;
        self.append_log(
            "note_tag",
            note_tag.note_id,
            "add",
            fs_assoc_value(note_tag.tag_id, ts, &vv, &self.device_id),
        )
        .await?;
        Ok(())
    }

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        let path = self.note_tag_path(note_id, tag_id);
        let vv = self.next_assoc_vv(&path).await?;
        let ts = now();
        // A remove is a *tombstone* (deleted_at set) kept so it can beat a concurrent add.
        let state = NoteTagState {
            updated_at: ts,
            deleted_at: Some(ts),
            vv: vv.clone(),
            last_writer: self.device_id.clone(),
        };
        self.write_assoc_state(note_id, tag_id, &state).await?;
        self.append_log(
            "note_tag",
            note_id,
            "remove",
            fs_assoc_value(tag_id, ts, &vv, &self.device_id),
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
        let limit = super::effective_page_size(page_size) as usize;
        let dir_path = self.note_tag_dir(note_id);
        if !dir_path.exists() {
            return Ok((vec![], None));
        }
        let mut tags = Vec::new();
        let mut dir = tokio::fs::read_dir(&dir_path).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name().to_string_lossy().to_string();
            let Ok(tag_id) = Uuid::parse_str(&fname) else {
                continue;
            };
            // Skip a detached association (tombstone).
            match self.read_assoc_state(&entry.path()).await {
                Ok(Some(s)) if s.deleted_at.is_some() => continue,
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Could not read note_tag {note_id}/{tag_id}: {e}");
                    continue;
                }
            }
            match self.read_tag(tag_id).await {
                Ok(t) if t.deleted_at.is_none() => tags.push(t),
                Ok(_) => {}
                Err(e) => tracing::warn!("Could not load tag {tag_id} for note {note_id}: {e}"),
            }
        }
        tags.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(paginate(tags, limit, page_token.as_deref(), |t| {
            (t.created_at.to_sortable_rfc3339(), t.id)
        }))
    }
}

// ── ResourceRepository impl ───────────────────────────────────────────────────

#[async_trait]
impl ResourceRepository for FsBackend {
    async fn create_resource(
        &self,
        mut resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let dir = self.resource_dir(resource.id);
        tokio::fs::create_dir_all(&dir).await?;
        resource.vv = self.next_resource_vv(resource.id).await?;
        resource.last_writer = self.device_id.clone();
        // Write the binary payload first, then the metadata file. `read_resource` treats
        // the presence of `meta.msgpack` as proof the resource exists, so writing it last
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
        // Soft-deleted resources read as NotFound (the metadata tombstone is kept for sync).
        if resource.deleted_at.is_some() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let data = tokio::fs::read(self.resource_data_path(id)).await?;
        Ok((resource, data))
    }

    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        let meta_path = self.resource_meta_path(id);
        let mut resource: Resource = self.read_sidecar(&meta_path, id).await?;
        // Soft-delete: set the tombstone and bump the vector; the binary payload is retained
        // (reclaiming it is a separate compaction concern).
        let ts = now();
        resource.deleted_at = Some(ts);
        note_log::increment(&mut resource.vv, &self.device_id);
        resource.last_writer = self.device_id.clone();
        self.write_sidecar(&meta_path, &resource).await?;
        self.append_log(
            "resource",
            id,
            "delete",
            fs_tombstone_value(ts, &resource.vv, &resource.last_writer),
        )
        .await?;
        tracing::info!(%id, "Resource deleted");
        Ok(())
    }

    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let mut resources = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("resources")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let id_str = entry.file_name().to_string_lossy().to_string();
            if let Ok(id) = Uuid::parse_str(&id_str) {
                let meta_path = self.resource_meta_path(id);
                match self.read_sidecar::<Resource>(&meta_path, id).await {
                    Ok(r) if r.deleted_at.is_none() => resources.push(r),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("Could not load resource {id}: {e}"),
                }
            }
        }
        resources.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(paginate(resources, limit, page_token.as_deref(), |r| {
            (r.created_at.to_sortable_rfc3339(), r.id)
        }))
    }

    async fn purge_deleted_resources(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, StorageError> {
        let mut purged = 0u64;
        let mut dir = match tokio::fs::read_dir(self.root.join("resources")).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = dir.next_entry().await? {
            let Ok(id) = Uuid::parse_str(&entry.file_name().to_string_lossy()) else {
                continue;
            };
            // Only a readable tombstone older than the cutoff qualifies; a live resource,
            // a missing meta (orphan of a crashed create), or an unreadable meta
            // (conservatively skipped) all keep their payload.
            let meta = match self.read_resource_meta(id).await {
                Ok(Some(meta)) => meta,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!("Skipping resource {id} during purge (unreadable meta): {e}");
                    continue;
                }
            };
            let Some(deleted_at) = meta.deleted_at else {
                continue;
            };
            if deleted_at >= older_than {
                continue;
            }
            // Removing the data file replicates as a deletion through Syncthing — safe,
            // because every peer converges on the same tombstone; a late concurrent
            // revive rewrites the file (see the trait docs on the cutoff window).
            match tokio::fs::remove_file(self.resource_data_path(id)).await {
                Ok(()) => purged += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        if purged > 0 {
            tracing::info!(purged, "Reclaimed payloads of soft-deleted resources");
        }
        Ok(purged)
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
            // Notebooks — version-vector conflict resolution (see `note_log::resolve`),
            // matching notes and `DbBackend`.
            Change::NotebookCreate { notebook } | Change::NotebookUpdate { notebook } => {
                let path = self.notebook_path(notebook.id);
                if self
                    .sidecar_incoming_wins(
                        &path,
                        &notebook.vv,
                        notebook.updated_at,
                        &notebook.last_writer,
                    )
                    .await?
                {
                    self.write_sidecar(&path, &notebook).await?;
                    tracing::debug!(id = %notebook.id, "Applied remote notebook change");
                } else {
                    tracing::debug!(id = %notebook.id, "Skipped stale remote notebook change");
                }
            }
            Change::NotebookDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                let path = self.notebook_path(id);
                if path.exists()
                    && self
                        .sidecar_incoming_wins(&path, &vv, deleted_at, &last_writer)
                        .await?
                {
                    let mut nb: Notebook = self.read_sidecar(&path, id).await?;
                    nb.deleted_at = Some(deleted_at);
                    nb.updated_at = deleted_at;
                    nb.vv = vv;
                    nb.last_writer = last_writer;
                    self.write_sidecar(&path, &nb).await?;
                    tracing::debug!(%id, "Applied remote notebook delete");
                }
            }
            // Tags — version-vector conflict resolution (see `note_log::resolve`).
            Change::TagCreate { tag } | Change::TagUpdate { tag } => {
                let path = self.tag_path(tag.id);
                if self
                    .sidecar_incoming_wins(&path, &tag.vv, tag.updated_at, &tag.last_writer)
                    .await?
                {
                    self.write_sidecar(&path, &tag).await?;
                    tracing::debug!(id = %tag.id, "Applied remote tag change");
                } else {
                    tracing::debug!(id = %tag.id, "Skipped stale remote tag change");
                }
            }
            Change::TagDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                let path = self.tag_path(id);
                if path.exists()
                    && self
                        .sidecar_incoming_wins(&path, &vv, deleted_at, &last_writer)
                        .await?
                {
                    let mut t: Tag = self.read_sidecar(&path, id).await?;
                    t.deleted_at = Some(deleted_at);
                    t.updated_at = deleted_at;
                    t.vv = vv;
                    t.last_writer = last_writer;
                    self.write_sidecar(&path, &t).await?;
                    tracing::debug!(%id, "Applied remote tag delete");
                }
            }
            // NoteTag associations
            Change::NoteTagAdd {
                note_id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            } => {
                let path = self.note_tag_path(note_id, tag_id);
                if self
                    .assoc_incoming_wins(&path, &vv, updated_at, &last_writer)
                    .await?
                {
                    let state = NoteTagState {
                        updated_at,
                        deleted_at: None,
                        vv,
                        last_writer,
                    };
                    self.write_assoc_state(note_id, tag_id, &state).await?;
                    tracing::debug!(%note_id, %tag_id, "Applied remote note_tag add");
                }
            }
            Change::NoteTagRemove {
                note_id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            } => {
                let path = self.note_tag_path(note_id, tag_id);
                if self
                    .assoc_incoming_wins(&path, &vv, updated_at, &last_writer)
                    .await?
                {
                    let state = NoteTagState {
                        updated_at,
                        deleted_at: Some(updated_at),
                        vv,
                        last_writer,
                    };
                    self.write_assoc_state(note_id, tag_id, &state).await?;
                    tracing::debug!(%note_id, %tag_id, "Applied remote note_tag remove");
                }
            }
            // Resource changes from a DbBackend peer include the binary payload
            // (data=Some) because the database stores bytes directly and can embed
            // them in the change record. Resource changes originating from another
            // FsBackend peer set data=None because Syncthing has already replicated
            // the `resources/{id}/data` file through the filesystem. In the latter
            // case, writing the metadata file is sufficient; the data file is already
            // in place or will arrive shortly via replication.
            Change::ResourceCreate { resource, data } => {
                let ts = resource.deleted_at.unwrap_or(resource.created_at);
                if self
                    .resource_incoming_wins(resource.id, &resource.vv, ts, &resource.last_writer)
                    .await?
                {
                    tokio::fs::create_dir_all(self.resource_dir(resource.id)).await?;
                    self.write_sidecar(&self.resource_meta_path(resource.id), &resource)
                        .await?;
                    if let Some(bytes) = data {
                        tokio::fs::write(self.resource_data_path(resource.id), &bytes).await?;
                    }
                    tracing::debug!(id = %resource.id, "Applied remote resource create");
                }
            }
            Change::ResourceDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                // Soft-delete: tombstone the metadata (keep the blob) only when the delete wins.
                if let Some(mut resource) = self.read_resource_meta(id).await? {
                    if self
                        .resource_incoming_wins(id, &vv, deleted_at, &last_writer)
                        .await?
                    {
                        resource.deleted_at = Some(deleted_at);
                        resource.vv = vv;
                        resource.last_writer = last_writer;
                        self.write_sidecar(&self.resource_meta_path(id), &resource)
                            .await?;
                        tracing::debug!(%id, "Applied remote resource delete");
                    }
                }
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
        // Time-based pruning is not offered: a peer that has not yet consumed an entry tracks it
        // by byte offset, so dropping entries older than a timestamp could make it miss changes.
        // The global log is instead bounded automatically by generation-epoch snapshot compaction
        // in `append_log` (see `compact_global_log_locked`), which rewrites current state rather
        // than deleting history a peer still needs, so this method remains a no-op returning zero.
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the lost-update race: many concurrent updates to the *same* note
    /// must all land in this device's append-only log (none dropped by a racing rename).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_same_note_updates_keep_every_log_entry() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(FsBackend::new(dir.path()).await.unwrap());
        let note = backend.create_note(Note::new("t", "v0")).await.unwrap();
        let id = note.id;

        let updates = 20usize;
        let mut handles = Vec::new();
        for i in 0..updates {
            let b = Arc::clone(&backend);
            let mut edited = note.clone();
            handles.push(tokio::spawn(async move {
                edited.body = format!("v{i}");
                b.update_note(edited).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        // Single device → one log file; it must hold the create plus every update.
        let logs = backend.read_note_logs(id).await.unwrap();
        let total: usize = logs.iter().map(|l| l.len()).sum();
        assert_eq!(total, 1 + updates, "create + {updates} updates, none lost");
    }

    /// A read must merge live from the logs without rewriting the `note.md`/`meta.msgpack`
    /// projection (reads are pure; the log is the source of truth).
    #[tokio::test]
    async fn read_does_not_rewrite_projection() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FsBackend::new(dir.path()).await.unwrap();
        let note = backend.create_note(Note::new("t", "body")).await.unwrap();
        let meta = backend.note_meta_path(note.id);
        let md = backend.note_md_path(note.id);

        // Drop the projection; the read must still work from the log and not recreate it.
        tokio::fs::remove_file(&meta).await.unwrap();
        tokio::fs::remove_file(&md).await.unwrap();

        let read = backend.read_note(note.id).await.unwrap();
        assert_eq!(read.body, "body");
        assert!(!meta.exists(), "read must not rewrite meta.msgpack");
        assert!(!md.exists(), "read must not rewrite note.md");
    }

    /// The heap-based `PageCollector` must paginate exactly like the sort-then-`paginate`
    /// path it replaced: same page contents, same order, same cursors, across every page.
    #[tokio::test]
    async fn list_notes_pages_match_full_walk() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FsBackend::new(dir.path()).await.unwrap();
        let total = 23usize;
        let mut created = Vec::new();
        for i in 0..total {
            created.push(
                backend
                    .create_note(Note::new(format!("t{i}"), "b"))
                    .await
                    .unwrap(),
            );
        }
        // One deleted note must not appear in any page.
        backend.delete_note(created[5].id).await.unwrap();
        created.remove(5);
        created.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

        let mut walked = Vec::new();
        let mut token = None;
        loop {
            let (page, next) = backend.list_notes(7, token).await.unwrap();
            assert!(page.len() <= 7);
            walked.extend(page);
            match next {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        assert_eq!(
            walked.iter().map(|n| n.id).collect::<Vec<_>>(),
            created.iter().map(|n| n.id).collect::<Vec<_>>(),
            "paged walk must reproduce the full (created_at, id) order"
        );
    }

    /// Startup must remove orphaned `*.tmp` files from interrupted atomic writes in every
    /// managed directory, while leaving Syncthing's in-flight temporaries untouched.
    #[tokio::test]
    async fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones() {
        let dir = tempfile::tempdir().unwrap();
        let note_id = {
            let be = FsBackend::new(dir.path()).await.unwrap();
            be.create_note(Note::new("t", "b")).await.unwrap().id
        };
        let planted = [
            dir.path()
                .join("notes")
                .join(note_id.to_string())
                .join("meta.tmp"),
            dir.path().join("notebooks").join("junk.tmp"),
            dir.path().join(".keeplin").join("sync_state.tmp"),
            dir.path().join(".keeplin").join("offsets").join("dev.tmp"),
        ];
        for p in &planted {
            std::fs::write(p, b"junk").unwrap();
        }
        let syncthing = dir
            .path()
            .join("notebooks")
            .join(".syncthing.abc.msgpack.tmp");
        std::fs::write(&syncthing, b"in-flight transfer").unwrap();

        let be = FsBackend::new(dir.path()).await.unwrap();
        for p in &planted {
            assert!(!p.exists(), "must be swept: {}", p.display());
        }
        assert!(syncthing.exists(), "Syncthing temp must be left alone");
        assert_eq!(be.read_note(note_id).await.unwrap().body, "b");
    }

    /// A failed atomic write must leave no `*.tmp` litter and never touch the destination.
    #[tokio::test]
    async fn failed_atomic_write_cleans_up_its_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        // A directory as the destination makes the final rename fail deterministically.
        let dest = dir.path().join("blocked");
        std::fs::create_dir(&dest).unwrap();

        assert!(atomic_write(&dest, b"payload").await.is_err());
        assert!(
            !dest.with_extension("tmp").exists(),
            "temp file must be removed after a failed write"
        );
        assert!(dest.is_dir(), "destination must be untouched");
    }

    /// A corrupt association state file must read as "attached with minimum priority":
    /// still visible locally, and superseded by the next versioned peer state — the only
    /// surviving authoritative copy — instead of blocking listing or sync.
    #[tokio::test]
    async fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it() {
        let dir = tempfile::tempdir().unwrap();
        let be = FsBackend::new(dir.path()).await.unwrap();
        let note = be.create_note(Note::new("n", "")).await.unwrap();
        let tag = be.create_tag(Tag::new("t")).await.unwrap();
        be.add_note_tag(NoteTag {
            note_id: note.id,
            tag_id: tag.id,
        })
        .await
        .unwrap();

        // Damage the association's versioned state file.
        let path = dir
            .path()
            .join("note_tags")
            .join(note.id.to_string())
            .join(tag.id.to_string());
        std::fs::write(&path, b"not msgpack at all").unwrap();

        // Least-harm reading: the tag still lists as attached.
        let (tags, _) = be.list_note_tags(note.id, 0, None).await.unwrap();
        assert_eq!(tags.len(), 1, "corrupt state must not hide the association");

        // Any versioned peer state must beat the epoch-0 fallback marker.
        let mut vv = VersionVector::new();
        note_log::increment(&mut vv, "peer");
        be.apply_change(Change::NoteTagRemove {
            note_id: note.id,
            tag_id: tag.id,
            updated_at: now(),
            vv,
            last_writer: "peer".to_string(),
        })
        .await
        .unwrap();
        let (tags, _) = be.list_note_tags(note.id, 0, None).await.unwrap();
        assert!(
            tags.is_empty(),
            "a versioned remove must supersede the corrupt marker"
        );
    }

    /// Compaction must decline to rewrite the journal while a sidecar is unreadable —
    /// the snapshot would silently omit that entity — and resume once it is repaired.
    #[tokio::test]
    async fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair() {
        let dir = tempfile::tempdir().unwrap();
        let be = FsBackend::new(dir.path()).await.unwrap();
        let nb = be.create_notebook(Notebook::new("kept")).await.unwrap();
        be.create_tag(Tag::new("t")).await.unwrap();
        let log_path = be.device_log_path();
        let before = tokio::fs::read_to_string(&log_path).await.unwrap();

        // Corrupt the notebook sidecar, then try to compact: the journal must be intact.
        let sidecar = be.notebook_path(nb.id);
        std::fs::write(&sidecar, b"definitely not msgpack").unwrap();
        {
            let _guard = be.global_log_lock.lock().await;
            be.compact_global_log_locked().await.unwrap();
        }
        let after = tokio::fs::read_to_string(&log_path).await.unwrap();
        assert_eq!(before, after, "journal must not be rewritten while corrupt");
        assert_eq!(
            be.read_own_epoch().await.unwrap(),
            0,
            "no snapshot generation was produced"
        );

        // Repair the sidecar (as a restore from another device would) and retry.
        let bytes = rmp_serde::to_vec_named(&nb).unwrap();
        std::fs::write(&sidecar, bytes).unwrap();
        {
            let _guard = be.global_log_lock.lock().await;
            be.compact_global_log_locked().await.unwrap();
        }
        assert_eq!(
            be.read_own_epoch().await.unwrap(),
            1,
            "compaction resumes once the sidecar is readable"
        );
        let snapshot = tokio::fs::read_to_string(&log_path).await.unwrap();
        assert!(
            snapshot.contains(&nb.id.to_string()),
            "the repaired notebook is present in the snapshot"
        );
    }

    /// Syncthing conflict copies — the signature of a replicated `.keeplin/` — must be
    /// detected wherever they appear, without deleting them or blocking startup.
    #[tokio::test]
    async fn detects_syncthing_conflict_copies_without_removing_them() {
        let dir = tempfile::tempdir().unwrap();
        let note_id = {
            let be = FsBackend::new(dir.path()).await.unwrap();
            be.create_note(Note::new("t", "b")).await.unwrap().id
        };
        let conflicts = [
            dir.path()
                .join(".keeplin")
                .join("device_id.sync-conflict-20260702-120000-AAAAAAA"),
            dir.path()
                .join("notebooks")
                .join("junk.sync-conflict-20260702-120000-BBBBBBB.msgpack"),
            dir.path()
                .join("notes")
                .join(note_id.to_string())
                .join("log.dev.sync-conflict-20260702-120000-CCCCCCC.msgpack"),
        ];
        for p in &conflicts {
            std::fs::write(p, b"conflict copy").unwrap();
        }

        let found = FsBackend::scan_sync_conflicts(dir.path()).await;
        assert_eq!(
            found.len(),
            conflicts.len(),
            "all copies detected: {found:?}"
        );

        // Startup only reports — the copies may hold the sole good version of the data,
        // so they are never deleted, and the store still opens.
        let be = FsBackend::new(dir.path()).await.unwrap();
        for p in &conflicts {
            assert!(
                p.exists(),
                "conflict copy must be preserved: {}",
                p.display()
            );
        }
        assert_eq!(be.read_note(note_id).await.unwrap().body, "b");
    }

    /// Purging must free only the payloads of tombstones older than the cutoff, keep the
    /// tombstone metadata (so the delete keeps converging), and leave a later re-create
    /// of the same id fully functional.
    #[tokio::test]
    async fn purge_reclaims_old_tombstoned_payloads_only() {
        let dir = tempfile::tempdir().unwrap();
        let be = FsBackend::new(dir.path()).await.unwrap();

        let dead = Resource::new("dead", "text/plain", "d.txt", 4);
        let dead_id = dead.id;
        be.create_resource(dead, b"dead".to_vec()).await.unwrap();
        be.delete_resource(dead_id).await.unwrap();

        let live = Resource::new("live", "text/plain", "l.txt", 4);
        let live_id = live.id;
        be.create_resource(live, b"live".to_vec()).await.unwrap();

        // A cutoff before the tombstone purges nothing.
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).unwrap();
        assert_eq!(be.purge_deleted_resources(epoch).await.unwrap(), 0);
        assert!(be.resource_data_path(dead_id).exists());

        // A cutoff after it frees exactly the dead payload.
        assert_eq!(be.purge_deleted_resources(now()).await.unwrap(), 1);
        assert!(!be.resource_data_path(dead_id).exists(), "dead bytes freed");
        assert!(
            be.resource_meta_path(dead_id).exists(),
            "tombstone metadata must survive the purge"
        );
        assert!(matches!(
            be.read_resource(dead_id).await,
            Err(StorageError::NotFound(_))
        ));
        let (_, bytes) = be.read_resource(live_id).await.unwrap();
        assert_eq!(bytes, b"live", "live resources are untouched");

        // Purging is idempotent, and the id can be recreated afterwards.
        assert_eq!(be.purge_deleted_resources(now()).await.unwrap(), 0);
        let mut revived = Resource::new("revived", "text/plain", "r.txt", 3);
        revived.id = dead_id;
        be.create_resource(revived, b"new".to_vec()).await.unwrap();
        let (_, bytes) = be.read_resource(dead_id).await.unwrap();
        assert_eq!(bytes, b"new");
    }

    #[tokio::test]
    async fn fresh_store_is_stamped_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let be = FsBackend::new(dir.path()).await.unwrap();
        let stamp = tokio::fs::read_to_string(be.format_version_path())
            .await
            .unwrap();
        assert_eq!(
            stamp.trim().parse::<u32>().unwrap(),
            FsBackend::FORMAT_VERSION,
            "a brand-new store starts stamped at the current format version"
        );
    }

    #[tokio::test]
    async fn migrates_a_legacy_stamp_and_preserves_data() {
        let dir = tempfile::tempdir().unwrap();

        // Create a store, write a note, then roll the format stamp back to 1 to simulate a
        // layout produced by a build that predates the current format version.
        let note_id = {
            let be = FsBackend::new(dir.path()).await.unwrap();
            let note = be.create_note(Note::new("legacy", "kept")).await.unwrap();
            tokio::fs::write(be.format_version_path(), "1")
                .await
                .unwrap();
            note.id
        };

        // Reopening runs the migration ladder up to the current version…
        let be = FsBackend::new(dir.path()).await.unwrap();
        let stamp = tokio::fs::read_to_string(be.format_version_path())
            .await
            .unwrap();
        assert_eq!(
            stamp.trim().parse::<u32>().unwrap(),
            FsBackend::FORMAT_VERSION
        );
        // …without disturbing the data (every historical step is parse-compatible).
        assert_eq!(be.read_note(note_id).await.unwrap().body, "kept");
    }

    #[tokio::test]
    async fn refuses_to_open_a_newer_format() {
        let dir = tempfile::tempdir().unwrap();
        {
            let be = FsBackend::new(dir.path()).await.unwrap();
            let future = (FsBackend::FORMAT_VERSION + 1).to_string();
            tokio::fs::write(be.format_version_path(), future)
                .await
                .unwrap();
        }
        // FsBackend has no Debug, so match rather than unwrap_err.
        let err = match FsBackend::new(dir.path()).await {
            Ok(_) => panic!("opening a newer on-disk format must be refused"),
            Err(e) => e,
        };
        assert!(
            matches!(err, StorageError::InvalidState(ref m) if m.contains("newer than this build")),
            "got: {err:?}"
        );
    }
}
