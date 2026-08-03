// md:Overview
use std::path::PathBuf;

use async_trait::async_trait;
use blake2::{Blake2s256, Digest};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{now, Resource};
use crate::storage::note_log::{self, resolve, VersionVector, Winner};
use crate::storage::{ResourceRepository, SortableRfc3339};

use super::convert::fs_tombstone_value;
use super::pagination::paginate;
use super::FsBackend;

// md:fn content_hash
pub(super) fn content_hash(data: &[u8]) -> String {
    let digest = Blake2s256::digest(data);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

// md:StoredResource
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StoredResource {
    #[serde(flatten)]
    pub(super) resource: Resource,
    #[serde(default)]
    pub(super) blob_hash: String,
}

// md:impl FsBackend
impl FsBackend {
    // md:impl FsBackend > fn note_resources_dir
    pub(super) fn note_resources_dir(&self, note_id: Uuid) -> PathBuf {
        self.note_dir(note_id).join("resources")
    }

    // md:impl FsBackend > fn resource_meta_path
    pub(super) fn resource_meta_path(&self, note_id: Uuid, id: Uuid) -> PathBuf {
        self.note_resources_dir(note_id)
            .join(format!("{id}.meta.ndjson"))
    }

    // md:impl FsBackend > fn resource_blob_path
    pub(super) fn resource_blob_path(&self, note_id: Uuid, hash: &str) -> PathBuf {
        self.note_resources_dir(note_id)
            .join(format!("{hash}.knrs"))
    }

    // md:impl FsBackend > fn all_note_ids
    pub(super) async fn all_note_ids(&self) -> Result<Vec<Uuid>, StorageError> {
        let mut ids = Vec::new();
        let mut rd = match tokio::fs::read_dir(self.root.join("notes")).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            if let Ok(id) = Uuid::parse_str(&entry.file_name().to_string_lossy()) {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    // md:impl FsBackend > fn note_resource_ids
    pub(super) async fn note_resource_ids(&self, note_id: Uuid) -> Result<Vec<Uuid>, StorageError> {
        let mut ids = Vec::new();
        let mut rd = match tokio::fs::read_dir(self.note_resources_dir(note_id)).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".meta.ndjson") else {
                continue;
            };
            if let Ok(id) = Uuid::parse_str(stem) {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    // md:impl FsBackend > fn locate_resource_note
    pub(super) async fn locate_resource_note(
        &self,
        id: Uuid,
    ) -> Result<Option<Uuid>, StorageError> {
        for note_id in self.all_note_ids().await? {
            if self.resource_meta_path(note_id, id).exists() {
                return Ok(Some(note_id));
            }
        }
        Ok(None)
    }

    // md:impl FsBackend > fn read_resource_sidecar
    pub(super) async fn read_resource_sidecar(
        &self,
        id: Uuid,
    ) -> Result<Option<(Uuid, StoredResource)>, StorageError> {
        let Some(note_id) = self.locate_resource_note(id).await? else {
            return Ok(None);
        };
        let stored: StoredResource = self
            .read_sidecar(&self.resource_meta_path(note_id, id), id)
            .await?;
        Ok(Some((note_id, stored)))
    }

    // md:impl FsBackend > fn read_resource_meta
    pub(super) async fn read_resource_meta(
        &self,
        id: Uuid,
    ) -> Result<Option<Resource>, StorageError> {
        Ok(self
            .read_resource_sidecar(id)
            .await?
            .map(|(_, stored)| stored.resource))
    }

    // md:impl FsBackend > fn next_resource_vv
    pub(super) async fn next_resource_vv(&self, id: Uuid) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .read_resource_meta(id)
            .await?
            .map(|r| r.vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    // md:impl FsBackend > fn resource_incoming_wins
    pub(super) async fn resource_incoming_wins(
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

    // md:impl FsBackend > fn cascade_stamp_resources
    pub(super) async fn cascade_stamp_resources(
        &self,
        note_id: Uuid,
        deleted_at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        for id in self.note_resource_ids(note_id).await? {
            let meta_path = self.resource_meta_path(note_id, id);
            let mut stored: StoredResource = match self.read_sidecar(&meta_path, id).await {
                Ok(s) => s,
                Err(StorageError::NotFound(_)) => continue,
                Err(e) => return Err(e),
            };
            if stored.resource.deleted_at.is_none() {
                stored.resource.deleted_at = Some(deleted_at);
                self.write_sidecar(&meta_path, &stored).await?;
            }
        }
        Ok(())
    }

    // md:impl FsBackend > fn cascade_unstamp_resources
    pub(super) async fn cascade_unstamp_resources(
        &self,
        note_id: Uuid,
        deleted_at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        for id in self.note_resource_ids(note_id).await? {
            let meta_path = self.resource_meta_path(note_id, id);
            let mut stored: StoredResource = match self.read_sidecar(&meta_path, id).await {
                Ok(s) => s,
                Err(StorageError::NotFound(_)) => continue,
                Err(e) => return Err(e),
            };
            if stored.resource.deleted_at == Some(deleted_at) {
                stored.resource.deleted_at = None;
                self.write_sidecar(&meta_path, &stored).await?;
            }
        }
        Ok(())
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
        let hash = content_hash(&data);
        tokio::fs::create_dir_all(self.note_resources_dir(resource.note_id)).await?;
        resource.vv = self.next_resource_vv(resource.id).await?;
        resource.last_writer = self.device_id.clone();
        tokio::fs::write(self.resource_blob_path(resource.note_id, &hash), &data).await?;
        let stored = StoredResource {
            resource: resource.clone(),
            blob_hash: hash,
        };
        self.write_sidecar(
            &self.resource_meta_path(resource.note_id, resource.id),
            &stored,
        )
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
        let Some((note_id, stored)) = self.read_resource_sidecar(id).await? else {
            return Err(StorageError::NotFound(id.to_string()));
        };
        if stored.resource.deleted_at.is_some() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let data = tokio::fs::read(self.resource_blob_path(note_id, &stored.blob_hash)).await?;
        Ok((stored.resource, data))
    }

    // md:impl ResourceRepository for FsBackend > fn delete_resource
    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        let Some((note_id, mut stored)) = self.read_resource_sidecar(id).await? else {
            return Err(StorageError::NotFound(id.to_string()));
        };
        let ts = now();
        stored.resource.deleted_at = Some(ts);
        note_log::increment(&mut stored.resource.vv, &self.device_id);
        stored.resource.last_writer = self.device_id.clone();
        self.write_sidecar(&self.resource_meta_path(note_id, id), &stored)
            .await?;
        self.append_log(
            "resource",
            id,
            "delete",
            fs_tombstone_value(ts, &stored.resource.vv, &stored.resource.last_writer),
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
        let limit = crate::storage::effective_page_size(page_size) as usize;
        let mut resources = Vec::new();
        for note_id in self.all_note_ids().await? {
            for id in self.note_resource_ids(note_id).await? {
                let meta_path = self.resource_meta_path(note_id, id);
                match self.read_sidecar::<StoredResource>(&meta_path, id).await {
                    Ok(s) if s.resource.deleted_at.is_none() => resources.push(s.resource),
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

    // md:impl ResourceRepository for FsBackend > fn list_resources_for_note
    async fn list_resources_for_note(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let limit = crate::storage::effective_page_size(page_size) as usize;
        let mut resources = Vec::new();
        for id in self.note_resource_ids(note_id).await? {
            let meta_path = self.resource_meta_path(note_id, id);
            match self.read_sidecar::<StoredResource>(&meta_path, id).await {
                Ok(s) if s.resource.deleted_at.is_none() => resources.push(s.resource),
                Ok(_) => {}
                Err(e) => tracing::warn!("Could not load resource {id}: {e}"),
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
        for note_id in self.all_note_ids().await? {
            let mut stored_list = Vec::new();
            for id in self.note_resource_ids(note_id).await? {
                match self
                    .read_sidecar::<StoredResource>(&self.resource_meta_path(note_id, id), id)
                    .await
                {
                    Ok(s) => stored_list.push(s),
                    Err(StorageError::NotFound(_)) => {}
                    Err(e) => {
                        tracing::warn!(
                            "Skipping resource {id} during purge (unreadable meta): {e}"
                        );
                    }
                }
            }
            let live_hashes: std::collections::HashSet<&str> = stored_list
                .iter()
                .filter(|s| s.resource.deleted_at.is_none())
                .map(|s| s.blob_hash.as_str())
                .collect();
            for stored in &stored_list {
                let Some(deleted_at) = stored.resource.deleted_at else {
                    continue;
                };
                if deleted_at >= older_than {
                    continue;
                }
                if stored.blob_hash.is_empty() || live_hashes.contains(stored.blob_hash.as_str()) {
                    continue;
                }
                match tokio::fs::remove_file(self.resource_blob_path(note_id, &stored.blob_hash))
                    .await
                {
                    Ok(()) => purged += 1,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        if purged > 0 {
            tracing::info!(purged, "Reclaimed payloads of soft-deleted resources");
        }
        Ok(purged)
    }
}
