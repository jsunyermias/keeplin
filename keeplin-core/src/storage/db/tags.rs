// md:Overview

use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{now, NoteTag, Tag},
};

use crate::storage::{SortableRfc3339, TagRepository};

use super::convert::{assoc_data, build_page, parse_cursor, tombstone_data, vv_to_json};
use super::DbBackend;

// md:impl TagRepository for DbBackend
#[async_trait]
impl TagRepository for DbBackend {
    // md:impl TagRepository for DbBackend > fn create_tag
    async fn create_tag(&self, mut tag: Tag) -> Result<Tag, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        tag.vv = self.next_local_vv("tags", &tag.id.to_string()).await?;
        tag.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            self.conn
                .execute(
                    "INSERT INTO tags (id,title,created_at,updated_at,deleted_at,vv,last_writer,system)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    libsql::params![
                        tag.id.to_string(),
                        tag.title.clone(),
                        tag.created_at.to_sortable_rfc3339(),
                        tag.updated_at.to_sortable_rfc3339(),
                        tag.deleted_at.map(|d| d.to_sortable_rfc3339()),
                        vv_to_json(&tag.vv),
                        tag.last_writer.clone(),
                        tag.system as i64,
                    ],
                )
                .await?;
            let data = serde_json::to_value(&tag).ok().map(|v| v.to_string());
            self.record_change("tag", &tag.id.to_string(), "create", data)
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %tag.id, "Tag created");
                Ok(tag)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    // md:impl TagRepository for DbBackend > fn read_tag
    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at,vv,last_writer,system
                 FROM tags WHERE id = ?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Self::row_to_tag(&row),
            None => Err(StorageError::NotFound(id.to_string())),
        }
    }

    // md:impl TagRepository for DbBackend > fn update_tag
    async fn update_tag(&self, mut tag: Tag) -> Result<Tag, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        tag.vv = self.next_local_vv("tags", &tag.id.to_string()).await?;
        tag.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let affected = self
                .conn
                .execute(
                    "UPDATE tags SET title=?2,updated_at=?3,deleted_at=?4,vv=?5,last_writer=?6,system=?7 WHERE id=?1",
                    libsql::params![
                        tag.id.to_string(),
                        tag.title.clone(),
                        tag.updated_at.to_sortable_rfc3339(),
                        tag.deleted_at.map(|d| d.to_sortable_rfc3339()),
                        vv_to_json(&tag.vv),
                        tag.last_writer.clone(),
                        tag.system as i64,
                    ],
                )
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(tag.id.to_string()));
            }
            let data = serde_json::to_value(&tag).ok().map(|v| v.to_string());
            self.record_change("tag", &tag.id.to_string(), "update", data)
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %tag.id, "Tag updated");
                Ok(tag)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    // md:impl TagRepository for DbBackend > fn delete_tag
    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let vv = self.next_local_vv("tags", &id.to_string()).await?;
        let writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let ts = now();
            let affected = self
                .conn
                .execute(
                    "UPDATE tags SET deleted_at=?2, updated_at=?2, vv=?3, last_writer=?4 WHERE id=?1",
                    libsql::params![id.to_string(), ts.to_sortable_rfc3339(), vv_to_json(&vv), writer.clone()],
                )
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(id.to_string()));
            }
            self.record_change("tag", &id.to_string(), "delete", Some(tombstone_data(ts, &vv, &writer)))
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(%id, "Tag deleted");
                Ok(())
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    // md:impl TagRepository for DbBackend > fn list_tags
    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at,vv,last_writer,system
                 FROM tags
                 WHERE deleted_at IS NULL
                   AND (
                     ?1 = '' OR created_at > ?2
                     OR (created_at = ?2 AND id > ?3)
                   )
                 ORDER BY created_at ASC, id ASC
                 LIMIT ?4",
                libsql::params![cursor_ts.clone(), cursor_ts, cursor_id, limit + 1],
            )
            .await?;
        let mut tags = Vec::new();
        while let Some(row) = rows.next().await? {
            tags.push(Self::row_to_tag(&row)?);
        }
        Ok(build_page(tags, limit as usize, |t| {
            format!("{}|{}", t.created_at.to_sortable_rfc3339(), t.id)
        }))
    }

    // md:impl TagRepository for DbBackend > fn add_note_tag
    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let note_id = note_tag.note_id.to_string();
        let tag_id = note_tag.tag_id.to_string();
        let vv = self.next_assoc_vv(&note_id, &tag_id).await?;
        let writer = self.device_id.clone();
        let ts = now();
        let r: Result<(), StorageError> = async {
            if !self.row_is_live("notes", &note_id).await? {
                return Err(StorageError::NotFound(note_id.clone()));
            }
            if !self.row_is_live("tags", &tag_id).await? {
                return Err(StorageError::NotFound(tag_id.clone()));
            }
            self.upsert_assoc(&note_id, &tag_id, ts, None, &vv, &writer)
                .await?;
            let data = assoc_data(note_tag.tag_id, ts, &vv, &writer);
            self.record_change("note_tag", &note_id, "add", Some(data))
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                Ok(())
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    // md:impl TagRepository for DbBackend > fn remove_note_tag
    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let note_id_s = note_id.to_string();
        let tag_id_s = tag_id.to_string();
        let vv = self.next_assoc_vv(&note_id_s, &tag_id_s).await?;
        let writer = self.device_id.clone();
        let ts = now();
        let r: Result<(), StorageError> = async {
            self.upsert_assoc(&note_id_s, &tag_id_s, ts, Some(ts), &vv, &writer)
                .await?;
            let data = assoc_data(tag_id, ts, &vv, &writer);
            self.record_change("note_tag", &note_id_s, "remove", Some(data))
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                Ok(())
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    // md:impl TagRepository for DbBackend > fn list_note_tags
    async fn list_note_tags(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT t.id,t.title,t.created_at,t.updated_at,t.deleted_at,t.vv,t.last_writer,t.system
                 FROM tags t
                 JOIN note_tags nt ON t.id = nt.tag_id
                 WHERE nt.note_id = ?1 AND nt.deleted_at IS NULL AND t.deleted_at IS NULL
                   AND (
                     ?2 = '' OR t.created_at > ?3
                     OR (t.created_at = ?3 AND t.id > ?4)
                   )
                 ORDER BY t.created_at ASC, t.id ASC
                 LIMIT ?5",
                libsql::params![
                    note_id.to_string(),
                    cursor_ts.clone(),
                    cursor_ts,
                    cursor_id,
                    limit + 1
                ],
            )
            .await?;
        let mut tags = Vec::new();
        while let Some(row) = rows.next().await? {
            tags.push(Self::row_to_tag(&row)?);
        }
        Ok(build_page(tags, limit as usize, |t| {
            format!("{}|{}", t.created_at.to_sortable_rfc3339(), t.id)
        }))
    }
}
