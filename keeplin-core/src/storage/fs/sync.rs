// md:Overview
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{Change, Notebook, Resource, Tag, SYSTEM_RESOURCE_NOTE_ID};
use crate::storage::note_log::VersionVector;
use crate::storage::SyncBackend;

use super::convert::log_entry_to_change;
use super::resources::{content_hash, StoredResource};
use super::tags::NoteTagState;
use super::FsBackend;

// md:SyncState
#[derive(Debug, Serialize, Deserialize)]
struct SyncState {
    last_sync: DateTime<Utc>,
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
                let prior_deleted = self.merge_note(note.id).await?.and_then(|n| n.deleted_at);
                let materialized = self.materialize(note.id).await?;
                if materialized.is_some_and(|n| n.deleted_at.is_none()) {
                    if let Some(old_ts) = prior_deleted {
                        self.cascade_unstamp_resources(note.id, old_ts).await?;
                    }
                }
                tracing::debug!(id = %note.id, "Materialized remote note change");
            }
            Change::NoteDelete { id, deleted_at, .. } => {
                self.materialize(id).await?;
                self.cascade_stamp_resources(id, deleted_at).await?;
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
                            system: false,
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
                    tokio::fs::create_dir_all(self.note_resources_dir(resource.note_id)).await?;
                    let blob_hash = match &data {
                        Some(bytes) => {
                            let hash = content_hash(bytes);
                            tokio::fs::write(
                                self.resource_blob_path(resource.note_id, &hash),
                                bytes,
                            )
                            .await?;
                            hash
                        }
                        None => self
                            .read_resource_sidecar(resource.id)
                            .await?
                            .map(|(_, s)| s.blob_hash)
                            .unwrap_or_default(),
                    };
                    let stored = StoredResource {
                        resource: resource.clone(),
                        blob_hash,
                    };
                    self.write_sidecar(
                        &self.resource_meta_path(resource.note_id, resource.id),
                        &stored,
                    )
                    .await?;
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
                    let (note_id, mut stored) = match self.read_resource_sidecar(id).await? {
                        Some(found) => found,
                        None => (
                            SYSTEM_RESOURCE_NOTE_ID,
                            StoredResource {
                                resource: Resource {
                                    id,
                                    note_id: SYSTEM_RESOURCE_NOTE_ID,
                                    title: String::new(),
                                    mime_type: String::new(),
                                    file_name: String::new(),
                                    size: 0,
                                    duration_ms: None,
                                    dimensions: None,
                                    created_at: deleted_at,
                                    deleted_at: None,
                                    vv: VersionVector::new(),
                                    last_writer: String::new(),
                                },
                                blob_hash: String::new(),
                            },
                        ),
                    };
                    stored.resource.deleted_at = Some(deleted_at);
                    stored.resource.vv = vv;
                    stored.resource.last_writer = last_writer;
                    tokio::fs::create_dir_all(self.note_resources_dir(note_id)).await?;
                    self.write_sidecar(&self.resource_meta_path(note_id, id), &stored)
                        .await?;
                    tracing::debug!(%id, "Applied remote resource delete");
                }
            }
        }
        Ok(())
    }

    // md:impl SyncBackend for FsBackend > fn get_last_sync_time
    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
        let path = self.root.join(".keeplin").join("sync_state.ndjson");
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
        let path = self.root.join(".keeplin").join("sync_state.ndjson");
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
