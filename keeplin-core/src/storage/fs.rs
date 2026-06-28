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

/// One line in a device's NDJSON change log.
/// `entity_type` defaults to "note" so old v1 log files (which had no such field)
/// are parsed correctly.  `entity_id` accepts the old "note_id" key as an alias.
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

/// Convert a `LogEntry` to a `Change`.  Returns `None` for unknown combinations
/// (malformed or future log entries); callers should skip such entries.
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
        // NoteTag associations (tag_id stored as {"tag_id": "..."} in data)
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
        // Resources — logs carry metadata only; data is replicated by Syncthing
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

    /// Current storage format version.  Bump when a breaking structural change
    /// makes old data unreadable without migration.
    const FORMAT_VERSION: u32 = 2;

    fn format_version_path(&self) -> PathBuf {
        self.root.join(".keeplin").join("format_version")
    }

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
            // v1 → v2: log entries are backward-compatible via serde aliases,
            // so no data migration is required — just stamp the new version.
            tracing::info!(
                from = current,
                to = Self::FORMAT_VERSION,
                "Migrating FsBackend format"
            );
        }

        // Always write (or overwrite) the version stamp.
        tokio::fs::write(&path, Self::FORMAT_VERSION.to_string()).await?;
        Ok(())
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

    fn log_offset_path(&self, device_id: &str) -> PathBuf {
        self.root.join(".keeplin").join("offsets").join(device_id)
    }

    async fn read_log_offset(&self, device_id: &str) -> u64 {
        tokio::fs::read_to_string(self.log_offset_path(device_id))
            .await
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    /// Atomically write the byte offset to avoid a torn read after a crash.
    async fn write_log_offset(&self, device_id: &str, offset: u64) -> Result<(), StorageError> {
        let path = self.log_offset_path(device_id);
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, offset.to_string()).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    /// Read all entries from other devices' logs that are newer than `since`.
    /// Does NOT advance the stored byte offset — safe to call multiple times.
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

    /// Stream new lines from each remote device log starting from the stored
    /// byte offset, then advance the offset so we never re-read old entries.
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
            // Resources — write meta always; write data file only when the Change carries it
            // (DbBackend-sourced Changes have data=Some; FsBackend/Syncthing replicates the
            //  file independently so data=None here is expected and correct)
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
