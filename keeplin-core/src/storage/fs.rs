// md:Overview
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

use super::backend::DEFAULT_HISTORY_LIMIT;
use super::note_log::{self, resolve, NoteLogEntry, NoteOp, VersionVector, Winner};
use super::{
    EntityVersion, HistoryRepository, NoteRepository, NotebookRepository, NotebookSortProfile,
    ResourceRepository, SortableRfc3339, SyncBackend, TagRepository,
};

// md:NoteMeta
#[derive(Debug, Serialize, Deserialize)]
struct NoteMeta {
    note: Note,
    vv: VersionVector,
}

// md:NoteMetaEntry
#[derive(Debug, Clone)]
struct NoteMetaEntry {
    notebook_id: Uuid,
    created_at: DateTime<Utc>,
    effective_sort_key: u32,
    is_starred: bool,
}

// md:impl NoteMetaEntry
impl NoteMetaEntry {
    // md:impl NoteMetaEntry > fn from_note
    fn from_note(note: &Note) -> Self {
        Self {
            notebook_id: note.notebook_id,
            created_at: note.created_at,
            effective_sort_key: note.effective_sort_key(),
            is_starred: note.is_starred,
        }
    }
}

// md:NoteMetaIndex
#[derive(Debug, Default)]
struct NoteMetaIndex {
    entries: std::collections::HashMap<Uuid, NoteMetaEntry>,
}

// md:impl NoteMetaIndex
impl NoteMetaIndex {
    // md:impl NoteMetaIndex > fn apply
    fn apply(&mut self, note: &Note) {
        if note.deleted_at.is_some() {
            self.entries.remove(&note.id);
        } else {
            self.entries.insert(note.id, NoteMetaEntry::from_note(note));
        }
    }
}

// md:NoteTagState
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

// md:LogEntry
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

// md:fn default_entity_type
fn default_entity_type() -> String {
    "note".to_string()
}

// md:EpochHeader
#[derive(Debug, Serialize, Deserialize)]
struct EpochHeader {
    #[serde(rename = "__keeplin_epoch__")]
    epoch: u64,
}

// md:fn parse_epoch_header
fn parse_epoch_header(line: &str) -> Option<u64> {
    serde_json::from_str::<EpochHeader>(line)
        .ok()
        .map(|h| h.epoch)
}

// md:fn fs_tombstone_value
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

// md:fn fs_assoc_value
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

// md:fn snapshot_entry_from_sidecar
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

// md:fn snapshot_entry_from_value
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

// md:fn fs_assoc_from_data
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

// md:fn fs_tombstone_from_data
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

// md:fn log_entry_to_change
fn log_entry_to_change(entry: LogEntry) -> Option<Change> {
    let id = entry.entity_id;
    let ts = entry.timestamp;
    match (entry.entity_type.as_str(), entry.operation.as_str()) {
        ("note", "create") | ("note", "note_create") => serde_json::from_value(entry.data)
            .ok()
            .map(|note| Change::NoteCreate { note }),
        ("note", "update") | ("note", "note_update") => serde_json::from_value(entry.data)
            .ok()
            .map(|note| Change::NoteUpdate { note }),
        ("note", "delete") | ("note", "note_delete") => {
            let (deleted_at, vv, last_writer) = fs_tombstone_from_data(&entry.data, ts);
            Some(Change::NoteDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            })
        }
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

// md:fn atomic_write
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

// md:SyncState
#[derive(Debug, Serialize, Deserialize)]
struct SyncState {
    last_sync: DateTime<Utc>,
}

// md:FsBackend
pub struct FsBackend {
    root: PathBuf,
    device_id: String,
    note_write_lock: Arc<Mutex<()>>,
    global_log_lock: Arc<Mutex<()>>,
    note_index: Arc<RwLock<Option<NoteMetaIndex>>>,
}

// md:impl FsBackend
impl FsBackend {
    // md:impl FsBackend > fn new
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

    // md:impl FsBackend > fn sweep_orphan_tmp_files
    async fn sweep_orphan_tmp_files(root: &Path) -> usize {
        let mut removed = 0usize;
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

    // md:impl FsBackend > fn scan_sync_conflicts
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

    // md:impl FsBackend > fn sweep_tmp_in_dir
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

    const FORMAT_VERSION: u32 = 5;

    const NOTE_LOG_COMPACT_THRESHOLD: usize = 256;

    const GLOBAL_LOG_COMPACT_THRESHOLD: usize = 512;

    const GLOBAL_LOG_SOFT_BYTES: u64 = 64 * 1024;

    // md:impl FsBackend > fn format_version_path
    fn format_version_path(&self) -> PathBuf {
        self.root.join(".keeplin").join("format_version")
    }

    // md:impl FsBackend > fn ensure_format_version
    async fn ensure_format_version(&self, fresh: bool) -> Result<(), StorageError> {
        let path = self.format_version_path();

        if fresh {
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
            tokio::fs::write(&path, version.to_string()).await?;
            tracing::info!(version, "Applied filesystem format migration");
        }

        tokio::fs::write(&path, Self::FORMAT_VERSION.to_string()).await?;
        Ok(())
    }

    // md:impl FsBackend > fn apply_format_migration
    async fn apply_format_migration(&self, version: u32) -> Result<(), StorageError> {
        match version {
            2..=5 => Ok(()),
            other => Err(StorageError::InvalidState(format!(
                "no filesystem migration defined for format version {other}"
            ))),
        }
    }

    // md:impl FsBackend > fn note_dir
    fn note_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("notes").join(id.to_string())
    }

    // md:impl FsBackend > fn note_md_path
    fn note_md_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("note.md")
    }

    // md:impl FsBackend > fn note_meta_path
    fn note_meta_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("meta.msgpack")
    }

    // md:impl FsBackend > fn note_log_path
    fn note_log_path(&self, id: Uuid, device_id: &str) -> PathBuf {
        self.note_dir(id).join(format!("log.{device_id}.msgpack"))
    }

    // md:impl FsBackend > fn device_log_path
    fn device_log_path(&self) -> PathBuf {
        self.root
            .join("logs")
            .join(format!("{}.log", self.device_id))
    }

    // md:impl FsBackend > fn notebook_path
    fn notebook_path(&self, id: Uuid) -> PathBuf {
        self.root.join("notebooks").join(format!("{id}.msgpack"))
    }

    // md:impl FsBackend > fn tag_path
    fn tag_path(&self, id: Uuid) -> PathBuf {
        self.root.join("tags").join(format!("{id}.msgpack"))
    }

    // md:impl FsBackend > fn note_tag_dir
    fn note_tag_dir(&self, note_id: Uuid) -> PathBuf {
        self.root.join("note_tags").join(note_id.to_string())
    }

    // md:impl FsBackend > fn note_tag_path
    fn note_tag_path(&self, note_id: Uuid, tag_id: Uuid) -> PathBuf {
        self.note_tag_dir(note_id).join(tag_id.to_string())
    }

    // md:impl FsBackend > fn resource_dir
    fn resource_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("resources").join(id.to_string())
    }

    // md:impl FsBackend > fn resource_meta_path
    fn resource_meta_path(&self, id: Uuid) -> PathBuf {
        self.resource_dir(id).join("meta.msgpack")
    }

    // md:impl FsBackend > fn resource_data_path
    fn resource_data_path(&self, id: Uuid) -> PathBuf {
        self.resource_dir(id).join("data")
    }

    // md:impl FsBackend > fn read_or_create_device_id
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

    // md:impl FsBackend > fn append_log
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

    // md:impl FsBackend > fn maybe_compact_global_log_locked
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

    // md:impl FsBackend > fn own_log_entry_count
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

    // md:impl FsBackend > fn read_own_epoch
    async fn read_own_epoch(&self) -> Result<u64, StorageError> {
        let (epoch, _len) = self.read_log_header(&self.device_log_path()).await?;
        Ok(epoch)
    }

    // md:impl FsBackend > fn compact_global_log_locked
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

    // md:impl FsBackend > fn build_global_snapshot
    async fn build_global_snapshot(&self) -> Result<(Vec<LogEntry>, usize), StorageError> {
        let ts = now();
        let mut out = Vec::new();
        let mut unreadable = 0usize;

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

        if let Ok(mut rd) = tokio::fs::read_dir(self.root.join("resources")).await {
            while let Some(e) = rd.next_entry().await? {
                let Ok(id) = Uuid::parse_str(&e.file_name().to_string_lossy()) else {
                    continue;
                };
                let meta_path = self.resource_meta_path(id);
                let bytes = match tokio::fs::read(&meta_path).await {
                    Ok(bytes) => bytes,
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

    // md:impl FsBackend > fn log_offset_path
    fn log_offset_path(&self, device_id: &str) -> PathBuf {
        self.root.join(".keeplin").join("offsets").join(device_id)
    }

    // md:impl FsBackend > fn read_log_offset
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

    // md:impl FsBackend > fn write_log_offset
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

    // md:impl FsBackend > fn read_log_header
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

    // md:impl FsBackend > fn read_other_logs_since
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

    // md:impl FsBackend > fn read_new_entries
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

    // md:impl FsBackend > fn write_sidecar
    async fn write_sidecar<T: serde::Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), StorageError> {
        let bytes = rmp_serde::to_vec_named(value)
            .map_err(|e| StorageError::InvalidState(format!("msgpack encode: {e}")))?;
        atomic_write(path, &bytes).await
    }

    // md:impl FsBackend > fn read_sidecar
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

    // md:impl FsBackend > fn sidecar_vv
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

    // md:impl FsBackend > fn next_sidecar_vv
    async fn next_sidecar_vv(&self, path: &Path) -> Result<VersionVector, StorageError> {
        let mut vv = self.sidecar_vv(path).await?;
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    // md:impl FsBackend > fn sidecar_incoming_wins
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

    // md:impl FsBackend > fn read_assoc_state
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

    // md:impl FsBackend > fn next_assoc_vv
    async fn next_assoc_vv(&self, path: &Path) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .read_assoc_state(path)
            .await?
            .map(|s| s.vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    // md:impl FsBackend > fn assoc_incoming_wins
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

    // md:impl FsBackend > fn write_assoc_state
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

    // md:impl FsBackend > fn read_resource_meta
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

    // md:impl FsBackend > fn next_resource_vv
    async fn next_resource_vv(&self, id: Uuid) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .read_resource_meta(id)
            .await?
            .map(|r| r.vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    // md:impl FsBackend > fn resource_incoming_wins
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

    // md:impl FsBackend > fn note_vv
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

    // md:impl FsBackend > fn read_note_logs
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

    // md:impl FsBackend > fn merge_note
    async fn merge_note(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
        let logs = self.read_note_logs(id).await?;
        Ok(note_log::merge(&logs).note)
    }

    // md:impl FsBackend > fn materialize
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

    // md:impl FsBackend > fn persist_note_projection
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

    // md:impl FsBackend > fn read_note_projection
    async fn read_note_projection(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
        let meta: NoteMeta = match self.read_sidecar(&self.note_meta_path(id), id).await {
            Ok(m) => m,
            Err(StorageError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(Some(meta.note))
    }

    // md:impl FsBackend > fn with_note_index
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

    // md:impl FsBackend > fn build_note_index
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

    // md:impl FsBackend > fn materialize_page
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

    // md:impl FsBackend > fn append_note_op
    async fn append_note_op(&self, id: Uuid, op: NoteOp) -> Result<Note, StorageError> {
        let _write_guard = self.note_write_lock.lock().await;
        tokio::fs::create_dir_all(self.note_dir(id)).await?;
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
        if log.len() > Self::NOTE_LOG_COMPACT_THRESHOLD {
            log = note_log::compact_own_log(&log);
        }
        self.write_sidecar(&log_path, &log).await?;
        self.materialize(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))
    }

    // md:impl FsBackend > fn collect_advanced_notes
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
            if merged.vv != old_vv {
                if let Some(note) = merged.note {
                    self.persist_note_projection(&note, &merged.vv).await?;
                    match note.deleted_at {
                        Some(deleted_at) => changes.push(Change::NoteDelete {
                            id,
                            deleted_at,
                            vv: merged.winner_vv.clone(),
                            last_writer: merged.winner_device.clone(),
                        }),
                        None => changes.push(Change::NoteUpdate { note }),
                    }
                }
            }
        }
        Ok(changes)
    }
}

// md:KeyedItem
struct KeyedItem<T> {
    key: (String, Uuid),
    item: T,
}

// md:impl PartialEq for KeyedItem
impl<T> PartialEq for KeyedItem<T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
// md:impl Eq for KeyedItem
impl<T> Eq for KeyedItem<T> {}
// md:impl PartialOrd for KeyedItem
impl<T> PartialOrd for KeyedItem<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
// md:impl Ord for KeyedItem
impl<T> Ord for KeyedItem<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

// md:PageCollector
struct PageCollector<T> {
    limit: usize,
    cursor: Option<(String, Uuid)>,
    heap: std::collections::BinaryHeap<KeyedItem<T>>,
}

// md:impl PageCollector
impl<T> PageCollector<T> {
    // md:impl PageCollector > fn new
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

    // md:impl PageCollector > fn push
    fn push(&mut self, key: (String, Uuid), item: T) {
        if let Some(cursor) = &self.cursor {
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

    // md:impl PageCollector > fn into_page
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

// md:fn paginate
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

// md:impl NoteRepository for FsBackend
#[async_trait]
impl NoteRepository for FsBackend {
    // md:impl NoteRepository for FsBackend > fn create_note
    async fn create_note(&self, note: Note) -> Result<Note, StorageError> {
        let merged = self.append_note_op(note.id, NoteOp::Upsert(note)).await?;
        tracing::info!(id = %merged.id, "Note created");
        Ok(merged)
    }

    // md:impl NoteRepository for FsBackend > fn read_note
    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        self.merge_note(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))
    }

    // md:impl NoteRepository for FsBackend > fn update_note
    async fn update_note(&self, note: Note) -> Result<Note, StorageError> {
        if self.read_note_logs(note.id).await?.is_empty() {
            return Err(StorageError::NotFound(note.id.to_string()));
        }
        let merged = self.append_note_op(note.id, NoteOp::Upsert(note)).await?;
        tracing::info!(id = %merged.id, "Note updated");
        Ok(merged)
    }

    // md:impl NoteRepository for FsBackend > fn delete_note
    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        if self.read_note_logs(id).await?.is_empty() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        self.append_note_op(id, NoteOp::Tombstone { deleted_at: now() })
            .await?;
        tracing::info!(%id, "Note deleted");
        Ok(())
    }

    // md:impl NoteRepository for FsBackend > fn list_notes
    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
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

    // md:impl NoteRepository for FsBackend > fn list_notes_in_notebook
    async fn list_notes_in_notebook(
        &self,
        notebook_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
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

    // md:impl NoteRepository for FsBackend > fn list_starred_notes
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

    // md:impl NoteRepository for FsBackend > fn notebook_sort_profile
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

// md:impl NotebookRepository for FsBackend
#[async_trait]
impl NotebookRepository for FsBackend {
    // md:impl NotebookRepository for FsBackend > fn create_notebook
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

    // md:impl NotebookRepository for FsBackend > fn read_notebook
    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        self.read_sidecar(&self.notebook_path(id), id).await
    }

    // md:impl NotebookRepository for FsBackend > fn update_notebook
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

    // md:impl NotebookRepository for FsBackend > fn delete_notebook
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

    // md:impl NotebookRepository for FsBackend > fn list_notebooks
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

// md:impl TagRepository for FsBackend
#[async_trait]
impl TagRepository for FsBackend {
    // md:impl TagRepository for FsBackend > fn create_tag
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

    // md:impl TagRepository for FsBackend > fn read_tag
    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        self.read_sidecar(&self.tag_path(id), id).await
    }

    // md:impl TagRepository for FsBackend > fn update_tag
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

    // md:impl TagRepository for FsBackend > fn delete_tag
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

    // md:impl TagRepository for FsBackend > fn list_tags
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

    // md:impl TagRepository for FsBackend > fn add_note_tag
    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
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

    // md:impl TagRepository for FsBackend > fn remove_note_tag
    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        let path = self.note_tag_path(note_id, tag_id);
        let vv = self.next_assoc_vv(&path).await?;
        let ts = now();
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

    // md:impl TagRepository for FsBackend > fn list_note_tags
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

// md:impl ResourceRepository for FsBackend
#[async_trait]
impl ResourceRepository for FsBackend {
    // md:impl ResourceRepository for FsBackend > fn create_resource
    async fn create_resource(
        &self,
        mut resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let dir = self.resource_dir(resource.id);
        tokio::fs::create_dir_all(&dir).await?;
        resource.vv = self.next_resource_vv(resource.id).await?;
        resource.last_writer = self.device_id.clone();
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

    // md:impl ResourceRepository for FsBackend > fn read_resource
    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let meta_path = self.resource_meta_path(id);
        if !meta_path.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let resource: Resource = self.read_sidecar(&meta_path, id).await?;
        if resource.deleted_at.is_some() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let data = tokio::fs::read(self.resource_data_path(id)).await?;
        Ok((resource, data))
    }

    // md:impl ResourceRepository for FsBackend > fn delete_resource
    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        let meta_path = self.resource_meta_path(id);
        let mut resource: Resource = self.read_sidecar(&meta_path, id).await?;
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

    // md:impl ResourceRepository for FsBackend > fn list_resources
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

    // md:impl ResourceRepository for FsBackend > fn purge_deleted_resources
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

// md:impl SyncBackend for FsBackend
#[async_trait]
impl SyncBackend for FsBackend {
    // md:impl SyncBackend for FsBackend > fn get_changes_since
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

    // md:impl SyncBackend for FsBackend > fn apply_change
    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        match change {
            Change::NoteCreate { note } | Change::NoteUpdate { note } => {
                self.materialize(note.id).await?;
                tracing::debug!(id = %note.id, "Materialized remote note change");
            }
            Change::NoteDelete { id, .. } => {
                self.materialize(id).await?;
                tracing::debug!(%id, "Materialized remote note delete");
            }
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
                if self
                    .sidecar_incoming_wins(&path, &vv, deleted_at, &last_writer)
                    .await?
                {
                    let mut nb: Notebook = match self.read_sidecar(&path, id).await {
                        Ok(nb) => nb,
                        Err(StorageError::NotFound(_)) => Notebook {
                            id,
                            title: String::new(),
                            created_at: deleted_at,
                            updated_at: deleted_at,
                            deleted_at: None,
                            alias: None,
                            vv: VersionVector::new(),
                            last_writer: String::new(),
                        },
                        Err(e) => return Err(e),
                    };
                    nb.deleted_at = Some(deleted_at);
                    nb.updated_at = deleted_at;
                    nb.vv = vv;
                    nb.last_writer = last_writer;
                    self.write_sidecar(&path, &nb).await?;
                    tracing::debug!(%id, "Applied remote notebook delete");
                }
            }
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
                if self
                    .sidecar_incoming_wins(&path, &vv, deleted_at, &last_writer)
                    .await?
                {
                    let mut t: Tag = match self.read_sidecar(&path, id).await {
                        Ok(t) => t,
                        Err(StorageError::NotFound(_)) => Tag {
                            id,
                            title: String::new(),
                            created_at: deleted_at,
                            updated_at: deleted_at,
                            deleted_at: None,
                            vv: VersionVector::new(),
                            last_writer: String::new(),
                        },
                        Err(e) => return Err(e),
                    };
                    t.deleted_at = Some(deleted_at);
                    t.updated_at = deleted_at;
                    t.vv = vv;
                    t.last_writer = last_writer;
                    self.write_sidecar(&path, &t).await?;
                    tracing::debug!(%id, "Applied remote tag delete");
                }
            }
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
                if self
                    .resource_incoming_wins(id, &vv, deleted_at, &last_writer)
                    .await?
                {
                    let mut resource = match self.read_resource_meta(id).await? {
                        Some(r) => r,
                        None => Resource {
                            id,
                            title: String::new(),
                            mime_type: String::new(),
                            file_name: String::new(),
                            size: 0,
                            created_at: deleted_at,
                            deleted_at: None,
                            vv: VersionVector::new(),
                            last_writer: String::new(),
                        },
                    };
                    resource.deleted_at = Some(deleted_at);
                    resource.vv = vv;
                    resource.last_writer = last_writer;
                    tokio::fs::create_dir_all(self.resource_dir(id)).await?;
                    self.write_sidecar(&self.resource_meta_path(id), &resource)
                        .await?;
                    tracing::debug!(%id, "Applied remote resource delete");
                }
            }
        }
        Ok(())
    }

    // md:impl SyncBackend for FsBackend > fn get_last_sync_time
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

    // md:impl SyncBackend for FsBackend > fn update_sync_time
    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        let state = SyncState { last_sync: ts };
        let path = self.root.join(".keeplin").join("sync_state.msgpack");
        self.write_sidecar(&path, &state).await
    }

    // md:impl SyncBackend for FsBackend > fn send_changes
    async fn send_changes(&self, _changes: Vec<Change>) -> Result<(), StorageError> {
        tracing::debug!("Offline mode: changes are replicated passively via the filesystem");
        Ok(())
    }

    // md:impl SyncBackend for FsBackend > fn receive_changes
    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
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
        changes.extend(self.collect_advanced_notes().await?);
        Ok(changes)
    }

    // md:impl SyncBackend for FsBackend > fn get_device_id
    async fn get_device_id(&self) -> Result<String, StorageError> {
        Ok(self.device_id.clone())
    }

    // md:impl SyncBackend for FsBackend > fn prune_change_journal
    async fn prune_change_journal(&self, _older_than: DateTime<Utc>) -> Result<u64, StorageError> {
        Ok(0)
    }
}

// md:impl FsBackend (global history)
impl FsBackend {
    // md:impl FsBackend (global history) > fn read_all_global_entries
    async fn read_all_global_entries(&self) -> Result<Vec<(String, LogEntry)>, StorageError> {
        let dir = self.root.join("logs");
        let mut out = Vec::new();
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(device) = name.strip_suffix(".log") else {
                continue;
            };
            let bytes = tokio::fs::read(entry.path()).await?;
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines() {
                if line.trim().is_empty() || parse_epoch_header(line).is_some() {
                    continue;
                }
                if let Ok(le) = serde_json::from_str::<LogEntry>(line) {
                    out.push((device.to_string(), le));
                }
            }
        }
        Ok(out)
    }
}

// md:impl HistoryRepository for FsBackend
#[async_trait]
impl HistoryRepository for FsBackend {
    // md:impl HistoryRepository for FsBackend > fn note_history
    async fn note_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Note>>, StorageError> {
        let logs = self.read_note_logs(id).await?;
        let mut versions: Vec<EntityVersion<Note>> = logs
            .into_iter()
            .flatten()
            .map(|e| EntityVersion {
                timestamp: e.timestamp,
                device_id: e.device_id,
                entity: match e.op {
                    NoteOp::Upsert(note) => Some(note),
                    NoteOp::Tombstone { .. } => None,
                },
            })
            .collect();
        sort_and_cap(&mut versions, limit);
        Ok(versions)
    }

    // md:impl HistoryRepository for FsBackend > fn notebook_history
    async fn notebook_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Notebook>>, StorageError> {
        let entries = self.read_all_global_entries().await?;
        let mut versions: Vec<EntityVersion<Notebook>> = entries
            .into_iter()
            .filter(|(_, le)| le.entity_type == "notebook" && le.entity_id == id)
            .filter_map(|(device, le)| {
                let entity = match le.operation.as_str() {
                    "create" | "update" => Some(serde_json::from_value::<Notebook>(le.data).ok()?),
                    "delete" => None,
                    _ => return None,
                };
                Some(EntityVersion {
                    timestamp: le.timestamp,
                    device_id: device,
                    entity,
                })
            })
            .collect();
        sort_and_cap(&mut versions, limit);
        Ok(versions)
    }
}

// md:fn sort_and_cap
fn sort_and_cap<T>(versions: &mut Vec<EntityVersion<T>>, limit: u32) {
    versions.sort_by(|a, b| {
        b.timestamp
            .cmp(&a.timestamp)
            .then_with(|| b.device_id.cmp(&a.device_id))
    });
    let cap = if limit == 0 {
        DEFAULT_HISTORY_LIMIT
    } else {
        limit
    } as usize;
    versions.truncate(cap);
}

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;

    // md:mod tests > fn concurrent_same_note_updates_keep_every_log_entry
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

        let logs = backend.read_note_logs(id).await.unwrap();
        let total: usize = logs.iter().map(|l| l.len()).sum();
        assert_eq!(total, 1 + updates, "create + {updates} updates, none lost");
    }

    // md:mod tests > fn read_does_not_rewrite_projection
    #[tokio::test]
    async fn read_does_not_rewrite_projection() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FsBackend::new(dir.path()).await.unwrap();
        let note = backend.create_note(Note::new("t", "body")).await.unwrap();
        let meta = backend.note_meta_path(note.id);
        let md = backend.note_md_path(note.id);

        tokio::fs::remove_file(&meta).await.unwrap();
        tokio::fs::remove_file(&md).await.unwrap();

        let read = backend.read_note(note.id).await.unwrap();
        assert_eq!(read.body, "body");
        assert!(!meta.exists(), "read must not rewrite meta.msgpack");
        assert!(!md.exists(), "read must not rewrite note.md");
    }

    // md:mod tests > fn list_notes_pages_match_full_walk
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

    // md:mod tests > fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones
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

    // md:mod tests > fn failed_atomic_write_cleans_up_its_temp_file
    #[tokio::test]
    async fn failed_atomic_write_cleans_up_its_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("blocked");
        std::fs::create_dir(&dest).unwrap();

        assert!(atomic_write(&dest, b"payload").await.is_err());
        assert!(
            !dest.with_extension("tmp").exists(),
            "temp file must be removed after a failed write"
        );
        assert!(dest.is_dir(), "destination must be untouched");
    }

    // md:mod tests > fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it
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

        let path = dir
            .path()
            .join("note_tags")
            .join(note.id.to_string())
            .join(tag.id.to_string());
        std::fs::write(&path, b"not msgpack at all").unwrap();

        let (tags, _) = be.list_note_tags(note.id, 0, None).await.unwrap();
        assert_eq!(tags.len(), 1, "corrupt state must not hide the association");

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

    // md:mod tests > fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair
    #[tokio::test]
    async fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair() {
        let dir = tempfile::tempdir().unwrap();
        let be = FsBackend::new(dir.path()).await.unwrap();
        let nb = be.create_notebook(Notebook::new("kept")).await.unwrap();
        be.create_tag(Tag::new("t")).await.unwrap();
        let log_path = be.device_log_path();
        let before = tokio::fs::read_to_string(&log_path).await.unwrap();

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

    // md:mod tests > fn detects_syncthing_conflict_copies_without_removing_them
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

    // md:mod tests > fn purge_reclaims_old_tombstoned_payloads_only
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

        let epoch = DateTime::<Utc>::from_timestamp(0, 0).unwrap();
        assert_eq!(be.purge_deleted_resources(epoch).await.unwrap(), 0);
        assert!(be.resource_data_path(dead_id).exists());

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

        assert_eq!(be.purge_deleted_resources(now()).await.unwrap(), 0);
        let mut revived = Resource::new("revived", "text/plain", "r.txt", 3);
        revived.id = dead_id;
        be.create_resource(revived, b"new".to_vec()).await.unwrap();
        let (_, bytes) = be.read_resource(dead_id).await.unwrap();
        assert_eq!(bytes, b"new");
    }

    // md:mod tests > fn fresh_store_is_stamped_current_version
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

    // md:mod tests > fn migrates_a_legacy_stamp_and_preserves_data
    #[tokio::test]
    async fn migrates_a_legacy_stamp_and_preserves_data() {
        let dir = tempfile::tempdir().unwrap();

        let note_id = {
            let be = FsBackend::new(dir.path()).await.unwrap();
            let note = be.create_note(Note::new("legacy", "kept")).await.unwrap();
            tokio::fs::write(be.format_version_path(), "1")
                .await
                .unwrap();
            note.id
        };

        let be = FsBackend::new(dir.path()).await.unwrap();
        let stamp = tokio::fs::read_to_string(be.format_version_path())
            .await
            .unwrap();
        assert_eq!(
            stamp.trim().parse::<u32>().unwrap(),
            FsBackend::FORMAT_VERSION
        );
        assert_eq!(be.read_note(note_id).await.unwrap().body, "kept");
    }

    // md:mod tests > fn refuses_to_open_a_newer_format
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
