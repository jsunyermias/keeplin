// md:Overview

use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{now, Note},
};

use crate::storage::{NoteRepository, SortableRfc3339};

use super::convert::{
    bookmarks_to_json, build_page, links_to_json, parse_cursor, tombstone_data, vv_to_json,
};
use super::DbBackend;

// md:impl NoteRepository for DbBackend
#[async_trait]
impl NoteRepository for DbBackend {
    // md:impl NoteRepository for DbBackend > fn create_note
    async fn create_note(&self, mut note: Note) -> Result<Note, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        note.vv = self.next_local_vv("notes", &note.id.to_string()).await?;
        note.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            self.conn
                .execute(
                    "INSERT INTO notes
                     (id, title, body, notebook_id, is_todo, todo_due, todo_completed, created_at, updated_at, deleted_at, alias, bookmarks, links, vv, last_writer, is_pinned, is_starred, sort_key)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
                    libsql::params![
                        note.id.to_string(),
                        note.title.clone(),
                        note.body.clone(),
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
            self.refresh_note_links(&note).await?;
            let data = serde_json::to_value(&note).ok().map(|v| v.to_string());
            self.record_change("note", &note.id.to_string(), "create", data).await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %note.id, "Note created");
                Ok(note)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    // md:impl NoteRepository for DbBackend > fn read_note
    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,body,notebook_id,is_todo,todo_due,todo_completed,
                        created_at,updated_at,deleted_at,alias,bookmarks,links,vv,last_writer,
                        is_pinned,is_starred,sort_key
                 FROM notes WHERE id = ?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Self::row_to_note(&row),
            None => Err(StorageError::NotFound(id.to_string())),
        }
    }

    // md:impl NoteRepository for DbBackend > fn update_note
    async fn update_note(&self, mut note: Note) -> Result<Note, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        note.vv = self.next_local_vv("notes", &note.id.to_string()).await?;
        note.last_writer = self.device_id.clone();
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
            let affected = self
                .conn
                .execute(
                    "UPDATE notes SET
                     title=?2, body=?3, notebook_id=?4, is_todo=?5, todo_due=?6,
                     todo_completed=?7, updated_at=?8, deleted_at=?9,
                     alias=?10, bookmarks=?11, links=?12, vv=?13, last_writer=?14,
                     is_pinned=?15, is_starred=?16, sort_key=?17
                     WHERE id = ?1",
                    libsql::params![
                        note.id.to_string(),
                        note.title.clone(),
                        note.body.clone(),
                        note.notebook_id.to_string(),
                        note.is_todo as i64,
                        note.todo_due.map(|d| d.to_sortable_rfc3339()),
                        note.todo_completed.map(|d| d.to_sortable_rfc3339()),
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
            if affected == 0 {
                return Err(StorageError::NotFound(note.id.to_string()));
            }
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
            self.refresh_note_links(&note).await?;
            let data = serde_json::to_value(&note).ok().map(|v| v.to_string());
            self.record_change("note", &note.id.to_string(), "update", data)
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %note.id, "Note updated");
                Ok(note)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    // md:impl NoteRepository for DbBackend > fn delete_note
    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let vv = self.next_local_vv("notes", &id.to_string()).await?;
        let writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let ts = now();
            let affected = self
                .conn
                .execute(
                    "UPDATE notes SET deleted_at = ?2, updated_at = ?2, vv = ?3, last_writer = ?4 WHERE id = ?1",
                    libsql::params![id.to_string(), ts.to_sortable_rfc3339(), vv_to_json(&vv), writer.clone()],
                )
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(id.to_string()));
            }
            self.conn
                .execute(
                    "UPDATE resources SET deleted_at = ?2 WHERE note_id = ?1 AND deleted_at IS NULL",
                    libsql::params![id.to_string(), ts.to_sortable_rfc3339()],
                )
                .await?;
            self.record_change("note", &id.to_string(), "delete", Some(tombstone_data(ts, &vv, &writer)))
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(%id, "Note deleted");
                Ok(())
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    // md:impl NoteRepository for DbBackend > fn list_notes
    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,body,notebook_id,is_todo,todo_due,todo_completed,
                        created_at,updated_at,deleted_at,alias,bookmarks,links,vv,last_writer,
                        is_pinned,is_starred,sort_key
                 FROM notes
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
        let mut notes = Vec::new();
        while let Some(row) = rows.next().await? {
            notes.push(Self::row_to_note(&row)?);
        }
        Ok(build_page(notes, limit as usize, |n| {
            format!("{}|{}", n.created_at.to_sortable_rfc3339(), n.id)
        }))
    }

    // md:impl NoteRepository for DbBackend > fn note_backlinks
    async fn note_backlinks(
        &self,
        target_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT n.id,n.title,n.body,n.notebook_id,n.is_todo,n.todo_due,n.todo_completed,
                        n.created_at,n.updated_at,n.deleted_at,n.alias,n.bookmarks,n.links,n.vv,n.last_writer,
                        n.is_pinned,n.is_starred,n.sort_key
                 FROM note_links nl
                 JOIN notes n ON n.id = nl.source_note_id
                 WHERE nl.target_note_id = ?1 AND n.deleted_at IS NULL
                   AND (
                     ?2 = '' OR n.created_at > ?3
                     OR (n.created_at = ?3 AND n.id > ?4)
                   )
                 ORDER BY n.created_at ASC, n.id ASC
                 LIMIT ?5",
                libsql::params![
                    target_id.to_string(),
                    cursor_ts.clone(),
                    cursor_ts,
                    cursor_id,
                    limit + 1
                ],
            )
            .await?;
        let mut notes = Vec::new();
        while let Some(row) = rows.next().await? {
            notes.push(Self::row_to_note(&row)?);
        }
        Ok(build_page(notes, limit as usize, |n| {
            format!("{}|{}", n.created_at.to_sortable_rfc3339(), n.id)
        }))
    }

    // md:impl NoteRepository for DbBackend > fn list_notes_in_notebook
    async fn list_notes_in_notebook(
        &self,
        notebook_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_key, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,body,notebook_id,is_todo,todo_due,todo_completed,
                        created_at,updated_at,deleted_at,alias,bookmarks,links,vv,last_writer,
                        is_pinned,is_starred,sort_key
                 FROM notes
                 WHERE notebook_id = ?1 AND deleted_at IS NULL
                   AND (
                     ?2 = ''
                     OR (CASE WHEN sort_key = 0 THEN 1000 ELSE sort_key END) > CAST(?2 AS INTEGER)
                     OR ((CASE WHEN sort_key = 0 THEN 1000 ELSE sort_key END) = CAST(?2 AS INTEGER)
                         AND id > ?3)
                   )
                 ORDER BY (CASE WHEN sort_key = 0 THEN 1000 ELSE sort_key END) ASC, id ASC
                 LIMIT ?4",
                libsql::params![notebook_id.to_string(), cursor_key, cursor_id, limit + 1],
            )
            .await?;
        let mut notes = Vec::new();
        while let Some(row) = rows.next().await? {
            notes.push(Self::row_to_note(&row)?);
        }
        Ok(build_page(notes, limit as usize, |n| {
            format!("{}|{}", n.effective_sort_key(), n.id)
        }))
    }

    // md:impl NoteRepository for DbBackend > fn list_starred_notes
    async fn list_starred_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,body,notebook_id,is_todo,todo_due,todo_completed,
                        created_at,updated_at,deleted_at,alias,bookmarks,links,vv,last_writer,
                        is_pinned,is_starred,sort_key
                 FROM notes
                 WHERE is_starred = 1 AND deleted_at IS NULL
                   AND (
                     ?1 = '' OR created_at > ?2
                     OR (created_at = ?2 AND id > ?3)
                   )
                 ORDER BY created_at ASC, id ASC
                 LIMIT ?4",
                libsql::params![cursor_ts.clone(), cursor_ts, cursor_id, limit + 1],
            )
            .await?;
        let mut notes = Vec::new();
        while let Some(row) = rows.next().await? {
            notes.push(Self::row_to_note(&row)?);
        }
        Ok(build_page(notes, limit as usize, |n| {
            format!("{}|{}", n.created_at.to_sortable_rfc3339(), n.id)
        }))
    }

    // md:impl NoteRepository for DbBackend > fn notebook_sort_profile
    async fn notebook_sort_profile(
        &self,
        notebook_id: Uuid,
    ) -> Result<super::NotebookSortProfile, StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT sort_key FROM notes WHERE notebook_id = ?1 AND deleted_at IS NULL",
                [notebook_id.to_string()],
            )
            .await?;
        let mut keys = Vec::new();
        while let Some(row) = rows.next().await? {
            let raw = row.get::<i64>(0)?.max(0) as u32;
            keys.push(if raw == 0 {
                Note::DEFAULT_SORT_KEY
            } else {
                raw
            });
        }
        Ok(super::NotebookSortProfile::from_effective_keys(keys))
    }
}
