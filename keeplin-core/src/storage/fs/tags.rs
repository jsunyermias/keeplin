// md:Overview
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{now, NoteTag, Tag};
use crate::storage::note_log::{self, resolve, VersionVector, Winner};
use crate::storage::{SortableRfc3339, TagRepository};

use super::convert::{fs_assoc_value, fs_tombstone_value};
use super::pagination::paginate;
use super::FsBackend;

// md:NoteTagState
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct NoteTagState {
    pub(super) updated_at: DateTime<Utc>,
    #[serde(default)]
    pub(super) deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(super) vv: VersionVector,
    #[serde(default)]
    pub(super) last_writer: String,
}

// md:impl FsBackend
impl FsBackend {
    // md:impl FsBackend > fn tag_path
    pub(super) fn tag_path(&self, id: Uuid) -> PathBuf {
        self.root.join("tags").join(format!("{id}.ndjson"))
    }

    // md:impl FsBackend > fn note_tag_dir
    pub(super) fn note_tag_dir(&self, note_id: Uuid) -> PathBuf {
        self.root.join("note_tags").join(note_id.to_string())
    }

    // md:impl FsBackend > fn note_tag_path
    pub(super) fn note_tag_path(&self, note_id: Uuid, tag_id: Uuid) -> PathBuf {
        self.note_tag_dir(note_id).join(tag_id.to_string())
    }

    // md:impl FsBackend > fn read_assoc_state
    pub(super) async fn read_assoc_state(
        &self,
        path: &Path,
    ) -> Result<Option<NoteTagState>, StorageError> {
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
        match serde_json::from_slice(&bytes) {
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
    pub(super) async fn next_assoc_vv(&self, path: &Path) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .read_assoc_state(path)
            .await?
            .map(|s| s.vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    // md:impl FsBackend > fn assoc_incoming_wins
    pub(super) async fn assoc_incoming_wins(
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
    pub(super) async fn write_assoc_state(
        &self,
        note_id: Uuid,
        tag_id: Uuid,
        state: &NoteTagState,
    ) -> Result<(), StorageError> {
        tokio::fs::create_dir_all(self.note_tag_dir(note_id)).await?;
        self.write_sidecar(&self.note_tag_path(note_id, tag_id), state)
            .await
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
        let limit = crate::storage::effective_page_size(page_size) as usize;
        let mut tags = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("tags")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(stem) = fname.strip_suffix(".ndjson") {
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
        let limit = crate::storage::effective_page_size(page_size) as usize;
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
