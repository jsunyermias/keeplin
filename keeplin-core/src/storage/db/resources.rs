// md:Overview

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{now, Resource},
};

use crate::storage::{ResourceRepository, SortableRfc3339};

use super::convert::{build_page, parse_cursor, tombstone_data, vv_to_json};
use super::DbBackend;

// md:impl ResourceRepository for DbBackend
#[async_trait]
impl ResourceRepository for DbBackend {
    // md:impl ResourceRepository for DbBackend > fn create_resource
    async fn create_resource(
        &self,
        mut resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        resource.vv = self.next_resource_vv(&resource.id.to_string()).await?;
        resource.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let data_b64 = STANDARD.encode(&data);
            self.conn
                .execute(
                    "INSERT INTO resources (id,title,mime_type,file_name,size,data,created_at,deleted_at,vv,last_writer,duration_ms,width,height,note_id)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                    libsql::params![
                        resource.id.to_string(),
                        resource.title.clone(),
                        resource.mime_type.clone(),
                        resource.file_name.clone(),
                        resource.size as i64,
                        data,
                        resource.created_at.to_sortable_rfc3339(),
                        resource.deleted_at.map(|d| d.to_sortable_rfc3339()),
                        vv_to_json(&resource.vv),
                        resource.last_writer.clone(),
                        resource.duration_ms.map(|d| d as i64),
                        resource.dimensions.map(|(w, _)| w as i64),
                        resource.dimensions.map(|(_, h)| h as i64),
                        resource.note_id.to_string(),
                    ],
                )
                .await?;
            let change_data = serde_json::to_value(&resource).ok().map(|mut v| {
                v["_data_b64"] = serde_json::Value::String(data_b64);
                v.to_string()
            });
            self.record_change("resource", &resource.id.to_string(), "create", change_data)
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %resource.id, "Resource created");
                Ok(resource)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    // md:impl ResourceRepository for DbBackend > fn read_resource
    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,mime_type,file_name,size,created_at,deleted_at,vv,last_writer,duration_ms,width,height,note_id,data
                 FROM resources WHERE id=?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            None => Err(StorageError::NotFound(id.to_string())),
            Some(row) => {
                let resource = Self::row_to_resource(&row)?;
                if resource.deleted_at.is_some() {
                    return Err(StorageError::NotFound(id.to_string()));
                }
                let blob: Vec<u8> = row.get(13)?;
                Ok((resource, blob))
            }
        }
    }

    // md:impl ResourceRepository for DbBackend > fn delete_resource
    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let vv = self.next_resource_vv(&id.to_string()).await?;
        let writer = self.device_id.clone();
        let ts = now();
        let r: Result<(), StorageError> = async {
            let affected = self
                .conn
                .execute(
                    "UPDATE resources SET deleted_at=?2, vv=?3, last_writer=?4 WHERE id=?1",
                    libsql::params![
                        id.to_string(),
                        ts.to_sortable_rfc3339(),
                        vv_to_json(&vv),
                        writer.clone()
                    ],
                )
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(id.to_string()));
            }
            self.record_change(
                "resource",
                &id.to_string(),
                "delete",
                Some(tombstone_data(ts, &vv, &writer)),
            )
            .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(%id, "Resource deleted");
                Ok(())
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    // md:impl ResourceRepository for DbBackend > fn list_resources
    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,mime_type,file_name,size,created_at,deleted_at,vv,last_writer,duration_ms,width,height,note_id
                 FROM resources
                 WHERE deleted_at IS NULL
                   AND (?1 = '' OR created_at > ?2 OR (created_at = ?2 AND id > ?3))
                 ORDER BY created_at ASC, id ASC
                 LIMIT ?4",
                libsql::params![cursor_ts.clone(), cursor_ts, cursor_id, limit + 1],
            )
            .await?;
        let mut resources = Vec::new();
        while let Some(row) = rows.next().await? {
            resources.push(Self::row_to_resource(&row)?);
        }
        Ok(build_page(resources, limit as usize, |r| {
            format!("{}|{}", r.created_at.to_sortable_rfc3339(), r.id)
        }))
    }

    // md:impl ResourceRepository for DbBackend > fn list_resources_for_note
    async fn list_resources_for_note(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,mime_type,file_name,size,created_at,deleted_at,vv,last_writer,duration_ms,width,height,note_id
                 FROM resources
                 WHERE note_id = ?5 AND deleted_at IS NULL
                   AND (?1 = '' OR created_at > ?2 OR (created_at = ?2 AND id > ?3))
                 ORDER BY created_at ASC, id ASC
                 LIMIT ?4",
                libsql::params![cursor_ts.clone(), cursor_ts, cursor_id, limit + 1, note_id.to_string()],
            )
            .await?;
        let mut resources = Vec::new();
        while let Some(row) = rows.next().await? {
            resources.push(Self::row_to_resource(&row)?);
        }
        Ok(build_page(resources, limit as usize, |r| {
            format!("{}|{}", r.created_at.to_sortable_rfc3339(), r.id)
        }))
    }

    // md:impl ResourceRepository for DbBackend > fn purge_deleted_resources
    async fn purge_deleted_resources(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, StorageError> {
        let _write_guard = self.lock.write().await;
        let purged = self
            .conn
            .execute(
                "UPDATE resources SET data = NULL
                 WHERE deleted_at IS NOT NULL AND deleted_at < ?1 AND data IS NOT NULL",
                libsql::params![older_than.to_sortable_rfc3339()],
            )
            .await?;
        if purged > 0 {
            tracing::info!(purged, "Reclaimed payloads of soft-deleted resources");
        }
        Ok(purged)
    }
}
