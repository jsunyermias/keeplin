// md:Overview

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::Message;

use crate::{
    error::StorageError,
    models::{new_id, Change},
};

use crate::storage::{SortableRfc3339, SyncBackend};

use super::convert::{bookmarks_to_json, links_to_json, vv_to_json};
use super::DbBackend;

// md:impl SyncBackend for DbBackend
#[async_trait]
impl SyncBackend for DbBackend {
    // md:impl SyncBackend for DbBackend > fn get_changes_since
    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError> {
        let _read_guard = self.lock.read().await;
        let since_str = since.to_sortable_rfc3339();
        let mut rows = self
            .conn
            .query(
                "SELECT entity_type, entity_id, operation, changed_at, data
                 FROM entity_changes
                 WHERE changed_at > ?1
                 ORDER BY id ASC",
                [since_str],
            )
            .await?;

        let mut changes = Vec::new();
        while let Some(row) = rows.next().await? {
            let entity_type: String = row.get(0)?;
            let entity_id: String = row.get(1)?;
            let operation: String = row.get(2)?;
            let changed_at = Self::parse_required_dt(row.get::<String>(3)?)?;
            let data_str: Option<String> = row.get(4)?;
            let data: serde_json::Value = data_str
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::Value::Null);

            match Self::row_to_change(&entity_type, &entity_id, &operation, changed_at, &data) {
                Some(change) => changes.push(change),
                None => tracing::warn!(
                    entity_type,
                    operation,
                    "Unknown entity_changes entry; skipped"
                ),
            }
        }
        Ok(changes)
    }

    // md:impl SyncBackend for DbBackend > fn apply_change
    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        match change {
            Change::NoteCreate { note } | Change::NoteUpdate { note } => {
                if !self
                    .incoming_wins(
                        "notes",
                        &note.id.to_string(),
                        &note.vv,
                        note.updated_at,
                        &note.last_writer,
                    )
                    .await?
                {
                    return Ok(());
                }
                self.begin().await?;
                let r: Result<(), StorageError> = async {
                    let prior_deleted = {
                        let mut rows = self
                            .conn
                            .query(
                                "SELECT deleted_at FROM notes WHERE id = ?1",
                                [note.id.to_string()],
                            )
                            .await?;
                        match rows.next().await? {
                            Some(row) => row.get::<Option<String>>(0)?,
                            None => None,
                        }
                    };
                    self.refresh_note_links(&note).await?;
                    self.conn
                        .execute(
                            "INSERT OR REPLACE INTO notes
                             (id,title,body,notebook_id,is_todo,todo_due,todo_completed,created_at,updated_at,deleted_at,alias,bookmarks,links,vv,last_writer,is_pinned,is_starred,sort_key)
                             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
                            libsql::params![
                                note.id.to_string(),
                                note.title,
                                note.body,
                                note.notebook_id.to_string(),
                                note.is_todo as i64,
                                note.todo_due.map(|d| d.to_sortable_rfc3339()),
                                note.todo_completed.map(|d| d.to_sortable_rfc3339()),
                                note.created_at.to_sortable_rfc3339(),
                                note.updated_at.to_sortable_rfc3339(),
                                note.deleted_at.map(|d| d.to_sortable_rfc3339()),
                                note.alias.clone(),
                                bookmarks_to_json(&note.bookmarks),
                                links_to_json(&note.links),
                                vv_to_json(&note.vv),
                                note.last_writer.clone(),
                                note.is_pinned as i64,
                                note.is_starred as i64,
                                note.sort_key as i64,
                            ],
                        )
                        .await?;
                    if note.deleted_at.is_none() {
                        if let Some(old_ts) = prior_deleted {
                            self.conn
                                .execute(
                                    "UPDATE resources SET deleted_at = NULL WHERE note_id = ?1 AND deleted_at = ?2",
                                    libsql::params![note.id.to_string(), old_ts],
                                )
                                .await?;
                        }
                    }
                    Ok(())
                }
                .await;
                if let Err(e) = r {
                    self.rollback().await;
                    return Err(e);
                }
                self.commit().await?;
            }
            Change::NoteDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                if !self
                    .incoming_wins("notes", &id.to_string(), &vv, deleted_at, &last_writer)
                    .await?
                {
                    return Ok(());
                }
                let affected = self
                    .conn
                    .execute(
                        "UPDATE notes SET deleted_at = ?2, updated_at = ?2, vv = ?3, last_writer = ?4 WHERE id = ?1",
                        libsql::params![
                            id.to_string(),
                            deleted_at.to_sortable_rfc3339(),
                            vv_to_json(&vv),
                            last_writer.clone(),
                        ],
                    )
                    .await?;
                if affected == 0 {
                    self.conn
                        .execute(
                            "INSERT OR IGNORE INTO notes (id, title, created_at, updated_at, deleted_at, vv, last_writer)
                             VALUES (?1, '', ?2, ?2, ?2, ?3, ?4)",
                            libsql::params![
                                id.to_string(),
                                deleted_at.to_sortable_rfc3339(),
                                vv_to_json(&vv),
                                last_writer,
                            ],
                        )
                        .await?;
                }
                self.conn
                    .execute(
                        "UPDATE resources SET deleted_at = ?2 WHERE note_id = ?1 AND deleted_at IS NULL",
                        libsql::params![id.to_string(), deleted_at.to_sortable_rfc3339()],
                    )
                    .await?;
            }
            Change::NotebookCreate { notebook } | Change::NotebookUpdate { notebook } => {
                if !self
                    .incoming_wins(
                        "notebooks",
                        &notebook.id.to_string(),
                        &notebook.vv,
                        notebook.updated_at,
                        &notebook.last_writer,
                    )
                    .await?
                {
                    return Ok(());
                }
                self.conn
                    .execute(
                        "INSERT OR REPLACE INTO notebooks (id,title,created_at,updated_at,deleted_at,alias,vv,last_writer)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                        libsql::params![
                            notebook.id.to_string(),
                            notebook.title,
                            notebook.created_at.to_sortable_rfc3339(),
                            notebook.updated_at.to_sortable_rfc3339(),
                            notebook.deleted_at.map(|d| d.to_sortable_rfc3339()),
                            notebook.alias.clone(),
                            vv_to_json(&notebook.vv),
                            notebook.last_writer.clone(),
                        ],
                    )
                    .await?;
            }
            Change::NotebookDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                if !self
                    .incoming_wins("notebooks", &id.to_string(), &vv, deleted_at, &last_writer)
                    .await?
                {
                    return Ok(());
                }
                let affected = self
                    .conn
                    .execute(
                        "UPDATE notebooks SET deleted_at = ?2, updated_at = ?2, vv = ?3, last_writer = ?4 WHERE id = ?1",
                        libsql::params![id.to_string(), deleted_at.to_sortable_rfc3339(), vv_to_json(&vv), last_writer.clone()],
                    )
                    .await?;
                if affected == 0 {
                    self.conn
                        .execute(
                            "INSERT OR IGNORE INTO notebooks (id, title, created_at, updated_at, deleted_at, vv, last_writer)
                             VALUES (?1, '', ?2, ?2, ?2, ?3, ?4)",
                            libsql::params![id.to_string(), deleted_at.to_sortable_rfc3339(), vv_to_json(&vv), last_writer],
                        )
                        .await?;
                }
            }
            Change::TagCreate { tag } | Change::TagUpdate { tag } => {
                if !self
                    .incoming_wins(
                        "tags",
                        &tag.id.to_string(),
                        &tag.vv,
                        tag.updated_at,
                        &tag.last_writer,
                    )
                    .await?
                {
                    return Ok(());
                }
                self.conn
                    .execute(
                        "INSERT OR REPLACE INTO tags (id,title,created_at,updated_at,deleted_at,vv,last_writer)
                         VALUES (?1,?2,?3,?4,?5,?6,?7)",
                        libsql::params![
                            tag.id.to_string(),
                            tag.title,
                            tag.created_at.to_sortable_rfc3339(),
                            tag.updated_at.to_sortable_rfc3339(),
                            tag.deleted_at.map(|d| d.to_sortable_rfc3339()),
                            vv_to_json(&tag.vv),
                            tag.last_writer.clone(),
                        ],
                    )
                    .await?;
            }
            Change::TagDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                if !self
                    .incoming_wins("tags", &id.to_string(), &vv, deleted_at, &last_writer)
                    .await?
                {
                    return Ok(());
                }
                let affected = self
                    .conn
                    .execute(
                        "UPDATE tags SET deleted_at = ?2, updated_at = ?2, vv = ?3, last_writer = ?4 WHERE id = ?1",
                        libsql::params![id.to_string(), deleted_at.to_sortable_rfc3339(), vv_to_json(&vv), last_writer.clone()],
                    )
                    .await?;
                if affected == 0 {
                    self.conn
                        .execute(
                            "INSERT OR IGNORE INTO tags (id, title, created_at, updated_at, deleted_at, vv, last_writer)
                             VALUES (?1, '', ?2, ?2, ?2, ?3, ?4)",
                            libsql::params![id.to_string(), deleted_at.to_sortable_rfc3339(), vv_to_json(&vv), last_writer],
                        )
                        .await?;
                }
            }
            Change::NoteTagAdd {
                note_id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            } => {
                let (n, t) = (note_id.to_string(), tag_id.to_string());
                if self
                    .assoc_incoming_wins(&n, &t, &vv, updated_at, &last_writer)
                    .await?
                {
                    self.upsert_assoc(&n, &t, updated_at, None, &vv, &last_writer)
                        .await?;
                }
            }
            Change::NoteTagRemove {
                note_id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            } => {
                let (n, t) = (note_id.to_string(), tag_id.to_string());
                if self
                    .assoc_incoming_wins(&n, &t, &vv, updated_at, &last_writer)
                    .await?
                {
                    self.upsert_assoc(&n, &t, updated_at, Some(updated_at), &vv, &last_writer)
                        .await?;
                }
            }
            Change::ResourceCreate { resource, data } => {
                let id = resource.id.to_string();
                let ts = resource.deleted_at.unwrap_or(resource.created_at);
                if self
                    .resource_incoming_wins(&id, &resource.vv, ts, &resource.last_writer)
                    .await?
                {
                    let blob = data.unwrap_or_default();
                    self.conn
                        .execute(
                            "INSERT OR REPLACE INTO resources (id,title,mime_type,file_name,size,data,created_at,deleted_at,vv,last_writer,duration_ms,width,height,note_id)
                             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                            libsql::params![
                                id,
                                resource.title,
                                resource.mime_type,
                                resource.file_name,
                                resource.size as i64,
                                blob,
                                resource.created_at.to_sortable_rfc3339(),
                                resource.deleted_at.map(|d| d.to_sortable_rfc3339()),
                                vv_to_json(&resource.vv),
                                resource.last_writer,
                                resource.duration_ms.map(|d| d as i64),
                                resource.dimensions.map(|(w, _)| w as i64),
                                resource.dimensions.map(|(_, h)| h as i64),
                                resource.note_id.to_string(),
                            ],
                        )
                        .await?;
                }
            }
            Change::ResourceDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => {
                if self
                    .resource_incoming_wins(&id.to_string(), &vv, deleted_at, &last_writer)
                    .await?
                {
                    let affected = self
                        .conn
                        .execute(
                            "UPDATE resources SET deleted_at=?2, vv=?3, last_writer=?4 WHERE id=?1",
                            libsql::params![
                                id.to_string(),
                                deleted_at.to_sortable_rfc3339(),
                                vv_to_json(&vv),
                                last_writer.clone(),
                            ],
                        )
                        .await?;
                    if affected == 0 {
                        self.conn
                            .execute(
                                "INSERT OR IGNORE INTO resources (id, title, mime_type, file_name, size, created_at, deleted_at, vv, last_writer)
                                 VALUES (?1, '', '', '', 0, ?2, ?2, ?3, ?4)",
                                libsql::params![
                                    id.to_string(),
                                    deleted_at.to_sortable_rfc3339(),
                                    vv_to_json(&vv),
                                    last_writer,
                                ],
                            )
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }

    // md:impl SyncBackend for DbBackend > fn get_last_sync_time
    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query("SELECT value FROM sync_state WHERE key = 'last_sync'", ())
            .await?;
        match rows.next().await? {
            Some(row) => {
                let s: String = row.get(0)?;
                s.parse::<DateTime<Utc>>()
                    .map_err(|e| StorageError::InvalidState(e.to_string()))
            }
            None => Ok(DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default()),
        }
    }

    // md:impl SyncBackend for DbBackend > fn update_sync_time
    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO sync_state (key, value) VALUES ('last_sync', ?1)",
                [ts.to_sortable_rfc3339()],
            )
            .await?;
        Ok(())
    }

    // md:impl SyncBackend for DbBackend > fn send_changes
    async fn send_changes(&self, changes: Vec<Change>) -> Result<(), StorageError> {
        if changes.is_empty() {
            return Ok(());
        }
        if self.server_url.is_empty() {
            tracing::debug!("No server_url configured; changes stay local");
            return Ok(());
        }
        let n = changes.len();
        let batch_id = new_id();
        let payload = serde_json::json!({
            "type": "changes",
            "batch_id": batch_id,
            "device_id": self.device_id,
            "changes": changes,
        })
        .to_string();

        for attempt in 0u32..=3 {
            let mut guard = self.ws.lock().await;
            Self::ensure_ws(
                &mut guard,
                &self.server_url,
                &self.auth_token,
                &self.device_id,
            )
            .await;
            let Some(ws) = guard.as_mut() else {
                return Err(StorageError::WebSocket(format!(
                    "cannot send {n} change(s): no WebSocket connection to {}",
                    self.server_url
                )));
            };
            match ws.send(Message::Text(payload.clone())).await {
                Ok(()) => {
                    tracing::info!(count = n, %batch_id, "Changes sent via WebSocket");
                    return Ok(());
                }
                Err(e) => {
                    *guard = None;
                    if attempt < 3 {
                        let delay = Duration::from_secs(2u64.pow(attempt));
                        tracing::warn!(attempt, ?delay, "WS send failed, retrying: {e}");
                        drop(guard);
                        tokio::time::sleep(delay).await;
                    } else {
                        return Err(StorageError::WebSocket(e.to_string()));
                    }
                }
            }
        }
        Ok(())
    }

    // md:impl SyncBackend for DbBackend > fn receive_changes
    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        let mut guard = self.ws.lock().await;
        Self::ensure_ws(
            &mut guard,
            &self.server_url,
            &self.auth_token,
            &self.device_id,
        )
        .await;
        if guard.is_none() {
            tracing::warn!("No WebSocket connection; no changes received");
            return Ok(vec![]);
        }
        const MAX_WS_MESSAGES: usize = 1_000;
        let drain_timeout = Duration::from_millis(100);
        let mut changes = Vec::new();
        let mut connection_closed = false;
        let mut msg_count = 0usize;
        {
            let ws = guard.as_mut().unwrap();
            loop {
                if msg_count >= MAX_WS_MESSAGES {
                    tracing::warn!(
                        limit = MAX_WS_MESSAGES,
                        "WebSocket message limit reached; remaining messages will be delivered on the next sync cycle"
                    );
                    break;
                }
                match timeout(drain_timeout, ws.next()).await {
                    Ok(Some(Ok(Message::Text(text)))) => {
                        msg_count += 1;
                        let v: serde_json::Value = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!("Skipping malformed WebSocket frame: {e}");
                                continue;
                            }
                        };
                        if v["type"] == "changes" {
                            if let Ok(batch) =
                                serde_json::from_value::<Vec<Change>>(v["changes"].clone())
                            {
                                tracing::info!(
                                    count = batch.len(),
                                    "Changes received via WebSocket"
                                );
                                changes.extend(batch);
                            }
                        }
                    }
                    Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => {
                        connection_closed = true;
                        break;
                    }
                    Err(_elapsed) => break,
                    Ok(Some(Ok(_))) => {}
                }
            }
        }
        if connection_closed {
            *guard = None;
        }
        Ok(changes)
    }

    // md:impl SyncBackend for DbBackend > fn prune_change_journal
    async fn prune_change_journal(&self, older_than: DateTime<Utc>) -> Result<u64, StorageError> {
        let _write_guard = self.lock.write().await;
        let affected = self
            .conn
            .execute(
                "DELETE FROM entity_changes WHERE changed_at < ?1",
                [older_than.to_sortable_rfc3339()],
            )
            .await?;
        tracing::info!(rows = affected, "Pruned entity_changes journal");
        Ok(affected)
    }

    // md:impl SyncBackend for DbBackend > fn get_device_id
    async fn get_device_id(&self) -> Result<String, StorageError> {
        Ok(self.device_id.clone())
    }
}
