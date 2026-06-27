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

        for dir in &["notes", "resources", ".keeplin", "logs", "notebooks", "tags", "note_tags"] {
            tokio::fs::create_dir_all(root.join(dir)).await?;
        }

        let device_id = Self::read_or_create_device_id(&root).await?;

        Ok(Self { root, device_id })
    }

    // ── Path helpers — Notes ──────────────────────────────────────────────────

    fn note_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("notes").join(id.to_string())
    }

    fn meta_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("meta.json")
    }

    fn device_log_path(&self) -> PathBuf {
        self.root
            .join("logs")
            .join(format!("{}.log", self.device_id))
    }

    // ── Path helpers — Notebooks ──────────────────────────────────────────────

    fn notebook_path(&self, id: Uuid) -> PathBuf {
        self.root.join("notebooks").join(format!("{id}.json"))
    }

    // ── Path helpers — Tags ───────────────────────────────────────────────────

    fn tag_path(&self, id: Uuid) -> PathBuf {
        self.root.join("tags").join(format!("{id}.json"))
    }

    // ── Path helpers — NoteTag ────────────────────────────────────────────────

    fn note_tag_dir(&self, note_id: Uuid) -> PathBuf {
        self.root.join("note_tags").join(note_id.to_string())
    }

    fn note_tag_path(&self, note_id: Uuid, tag_id: Uuid) -> PathBuf {
        self.note_tag_dir(note_id).join(tag_id.to_string())
    }

    // ── Path helpers — Resources ──────────────────────────────────────────────

    fn resource_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("resources").join(id.to_string())
    }

    fn resource_meta_path(&self, id: Uuid) -> PathBuf {
        self.resource_dir(id).join("meta.json")
    }

    fn resource_data_path(&self, id: Uuid) -> PathBuf {
        self.resource_dir(id).join("data")
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
        let target = self.meta_path(note.id);
        let tmp = target.with_extension("tmp");
        tokio::fs::write(&tmp, serde_json::to_string_pretty(note)?).await?;
        tokio::fs::rename(&tmp, &target).await?;
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

    // ── Generic single-file JSON helpers ──────────────────────────────────────

    async fn write_json<T: serde::Serialize>(&self, path: &Path, value: &T) -> Result<(), StorageError> {
        let raw = serde_json::to_string_pretty(value)?;
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, raw).await?;
        tokio::fs::rename(&tmp, path).await?;
        Ok(())
    }

    async fn read_json<T: serde::de::DeserializeOwned>(&self, path: &Path, id: Uuid) -> Result<T, StorageError> {
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
                    Ok(_) => {}
                    Err(e) => tracing::warn!("Could not load note {id}: {e}"),
                }
            }
        }
        Ok(notes)
    }

    // ── Notebooks ─────────────────────────────────────────────────────────────

    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        self.write_json(&self.notebook_path(notebook.id), &notebook).await?;
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
        self.write_json(&self.notebook_path(notebook.id), &notebook).await?;
        tracing::info!(id = %notebook.id, "Notebook updated");
        Ok(notebook)
    }

    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        let mut nb: Notebook = self.read_json(&self.notebook_path(id), id).await?;
        nb.deleted_at = Some(now());
        self.write_json(&self.notebook_path(id), &nb).await?;
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
        tracing::info!(id = %tag.id, "Tag updated");
        Ok(tag)
    }

    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        let mut tag: Tag = self.read_json(&self.tag_path(id), id).await?;
        tag.deleted_at = Some(now());
        self.write_json(&self.tag_path(id), &tag).await?;
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
        Ok(())
    }

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        let path = self.note_tag_path(note_id, tag_id);
        if path.exists() {
            tokio::fs::remove_file(path).await?;
        }
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

    async fn create_resource(&self, resource: Resource, data: Vec<u8>) -> Result<Resource, StorageError> {
        let dir = self.resource_dir(resource.id);
        tokio::fs::create_dir_all(&dir).await?;
        self.write_json(&self.resource_meta_path(resource.id), &resource).await?;
        tokio::fs::write(self.resource_data_path(resource.id), &data).await?;
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
        tokio::fs::remove_dir_all(dir).await?;
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

    async fn send_changes(&self, _changes: Vec<Change>) -> Result<(), StorageError> {
        tracing::debug!("Offline mode: changes are replicated passively via the filesystem");
        Ok(())
    }

    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        let since = self.get_last_sync_time().await?;
        self.get_changes_since(since).await
    }

    async fn get_device_id(&self) -> Result<String, StorageError> {
        Ok(self.device_id.clone())
    }
}
