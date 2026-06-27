use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag, new_id, now},
};

use super::StorageBackend;

// ── Log entry ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct LogEntry {
    timestamp: DateTime<Utc>,
    note_id: Uuid,
    operation: String,
    data: serde_json::Value,
}

// ── Sync state ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct SyncState {
    last_sync: DateTime<Utc>,
}

// ── FsBackend ─────────────────────────────────────────────────────────────────

/// Filesystem-backed storage. Changes are logged to per-device log files
/// under `{root}/logs/`. Syncthing (or any external tool) replicates those
/// log files; `get_changes_since` reads the *other* devices' logs to discover
/// what changed on remote devices.
pub struct FsBackend {
    root: PathBuf,
    device_id: String,
}

impl FsBackend {
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let root: PathBuf = root.into();

        tokio::fs::create_dir_all(root.join("notes")).await?;
        tokio::fs::create_dir_all(root.join("resources")).await?;
        tokio::fs::create_dir_all(root.join(".keeplin")).await?;
        tokio::fs::create_dir_all(root.join("logs")).await?;

        let device_id = Self::read_or_create_device_id(&root).await?;

        Ok(Self { root, device_id })
    }

    // ── Path helpers ──────────────────────────────────────────────────────────

    fn note_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("notes").join(id.to_string())
    }

    fn meta_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("meta.json")
    }

    fn body_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("body.md")
    }

    fn device_log_path(&self) -> PathBuf {
        self.root
            .join("logs")
            .join(format!("{}.log", self.device_id))
    }

    // ── Device ID ─────────────────────────────────────────────────────────────

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

    async fn append_log(
        &self,
        note_id: Uuid,
        operation: &str,
        data: serde_json::Value,
    ) -> Result<(), StorageError> {
        let entry = LogEntry {
            timestamp: now(),
            note_id,
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

    async fn read_other_logs(&self) -> Result<Vec<LogEntry>, StorageError> {
        let mut entries = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("logs")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name();
            let fname = fname.to_string_lossy();
            // Skip our own log
            if fname == format!("{}.log", self.device_id) {
                continue;
            }
            if !fname.ends_with(".log") {
                continue;
            }
            let content = tokio::fs::read_to_string(entry.path()).await?;
            for line in content.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<LogEntry>(line) {
                    Ok(e) => entries.push(e),
                    Err(err) => {
                        tracing::warn!("Skipping malformed log line: {err}");
                    }
                }
            }
        }
        Ok(entries)
    }

    // ── Note persistence ──────────────────────────────────────────────────────

    async fn write_note(&self, note: &Note) -> Result<(), StorageError> {
        let dir = self.note_dir(note.id);
        tokio::fs::create_dir_all(&dir).await?;
        let meta = serde_json::to_string_pretty(note)?;
        tokio::fs::write(self.meta_path(note.id), meta).await?;
        tokio::fs::write(self.body_path(note.id), &note.body).await?;
        Ok(())
    }

    async fn load_note(&self, id: Uuid) -> Result<Note, StorageError> {
        let meta_path = self.meta_path(id);
        if !meta_path.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let raw = tokio::fs::read_to_string(meta_path).await?;
        let note: Note = serde_json::from_str(&raw)?;
        Ok(note)
    }
}

// ── StorageBackend impl ───────────────────────────────────────────────────────

#[async_trait]
impl StorageBackend for FsBackend {
    // ── Notes ─────────────────────────────────────────────────────────────────

    async fn create_note(&self, note: Note) -> Result<Note, StorageError> {
        self.write_note(&note).await?;
        let data = serde_json::to_value(&note)?;
        self.append_log(note.id, "create", data).await?;
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
        let data = serde_json::to_value(&note)?;
        self.append_log(note.id, "update", data).await?;
        tracing::info!(id = %note.id, "Note updated");
        Ok(note)
    }

    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        let mut note = self.load_note(id).await?;
        note.deleted_at = Some(now());
        self.write_note(&note).await?;
        self.append_log(id, "delete", serde_json::json!({ "id": id }))
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
                    Ok(_) => {} // soft-deleted, skip
                    Err(e) => tracing::warn!("Could not load note {id}: {e}"),
                }
            }
        }
        Ok(notes)
    }

    // ── Notebooks (deferred) ──────────────────────────────────────────────────

    async fn create_notebook(&self, _notebook: Notebook) -> Result<Notebook, StorageError> {
        unimplemented!("Notebook support is planned for a later phase")
    }

    async fn read_notebook(&self, _id: Uuid) -> Result<Notebook, StorageError> {
        unimplemented!("Notebook support is planned for a later phase")
    }

    async fn update_notebook(&self, _notebook: Notebook) -> Result<Notebook, StorageError> {
        unimplemented!("Notebook support is planned for a later phase")
    }

    async fn delete_notebook(&self, _id: Uuid) -> Result<(), StorageError> {
        unimplemented!("Notebook support is planned for a later phase")
    }

    async fn list_notebooks(&self) -> Result<Vec<Notebook>, StorageError> {
        unimplemented!("Notebook support is planned for a later phase")
    }

    // ── Tags (deferred) ───────────────────────────────────────────────────────

    async fn create_tag(&self, _tag: Tag) -> Result<Tag, StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn read_tag(&self, _id: Uuid) -> Result<Tag, StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn update_tag(&self, _tag: Tag) -> Result<Tag, StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn delete_tag(&self, _id: Uuid) -> Result<(), StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn list_tags(&self) -> Result<Vec<Tag>, StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn add_note_tag(&self, _note_tag: NoteTag) -> Result<(), StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn remove_note_tag(&self, _note_id: Uuid, _tag_id: Uuid) -> Result<(), StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    async fn list_note_tags(&self, _note_id: Uuid) -> Result<Vec<Tag>, StorageError> {
        unimplemented!("Tag support is planned for a later phase")
    }

    // ── Resources (deferred) ──────────────────────────────────────────────────

    async fn create_resource(
        &self,
        _resource: Resource,
        _data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        unimplemented!("Resource support is planned for a later phase")
    }

    async fn read_resource(&self, _id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        unimplemented!("Resource support is planned for a later phase")
    }

    async fn delete_resource(&self, _id: Uuid) -> Result<(), StorageError> {
        unimplemented!("Resource support is planned for a later phase")
    }

    async fn list_resources(&self) -> Result<Vec<Resource>, StorageError> {
        unimplemented!("Resource support is planned for a later phase")
    }

    // ── Synchronisation ───────────────────────────────────────────────────────

    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError> {
        let entries = self.read_other_logs().await?;
        let mut changes = Vec::new();
        for entry in entries {
            if entry.timestamp <= since {
                continue;
            }
            let change = match entry.operation.as_str() {
                "create" => {
                    let note: Note = serde_json::from_value(entry.data)?;
                    Change::Create { note }
                }
                "update" => {
                    let note: Note = serde_json::from_value(entry.data)?;
                    Change::Update { note }
                }
                "delete" => {
                    let id: Uuid = serde_json::from_value(entry.data["id"].clone())?;
                    Change::Delete { id }
                }
                op => {
                    tracing::warn!("Unknown log operation: {op}");
                    continue;
                }
            };
            changes.push(change);
        }
        Ok(changes)
    }

    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        match change {
            Change::Create { note } => {
                self.write_note(&note).await?;
                tracing::debug!(id = %note.id, "Applied remote create");
            }
            Change::Update { note } => {
                self.write_note(&note).await?;
                tracing::debug!(id = %note.id, "Applied remote update");
            }
            Change::Delete { id } => {
                if self.meta_path(id).exists() {
                    let mut note = self.load_note(id).await?;
                    note.deleted_at = Some(now());
                    self.write_note(&note).await?;
                }
                tracing::debug!(%id, "Applied remote delete");
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
        let raw = serde_json::to_string_pretty(&state)?;
        tokio::fs::write(
            self.root.join(".keeplin").join("sync_state.json"),
            raw,
        )
        .await?;
        Ok(())
    }

    /// In offline mode, Syncthing handles replication; there is nothing to
    /// *actively* send. Changes are already written to the device log by each
    /// CRUD operation.
    async fn send_changes(&self, _changes: Vec<Change>) -> Result<(), StorageError> {
        tracing::debug!("Offline mode: changes are replicated passively via the filesystem");
        Ok(())
    }

    /// In offline mode, incoming changes are discovered by `get_changes_since`
    /// by reading the other devices' log files.
    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        let since = self.get_last_sync_time().await?;
        self.get_changes_since(since).await
    }

    async fn get_device_id(&self) -> Result<String, StorageError> {
        Ok(self.device_id.clone())
    }
}
