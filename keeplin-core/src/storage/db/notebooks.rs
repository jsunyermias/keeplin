// md:Overview

use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{now, Notebook},
};

use crate::storage::{NotebookRepository, SortableRfc3339};

use super::convert::{build_page, parse_cursor, tombstone_data, vv_to_json};
use super::DbBackend;

// md:impl NotebookRepository for DbBackend
#[async_trait]
impl NotebookRepository for DbBackend {
    // md:impl NotebookRepository for DbBackend > fn create_notebook
    async fn create_notebook(&self, mut notebook: Notebook) -> Result<Notebook, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        notebook.vv = self
            .next_local_vv("notebooks", &notebook.id.to_string())
            .await?;
        notebook.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            self.conn
                .execute(
                    "INSERT INTO notebooks (id,title,created_at,updated_at,deleted_at,alias,vv,last_writer)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    libsql::params![
                        notebook.id.to_string(),
                        notebook.title.clone(),
                        notebook.created_at.to_sortable_rfc3339(),
                        notebook.updated_at.to_sortable_rfc3339(),
                        notebook.deleted_at.map(|d| d.to_sortable_rfc3339()),
                        notebook.alias.clone(),
                        vv_to_json(&notebook.vv),
                        notebook.last_writer.clone(),
                    ],
                )
                .await?;
            let data = serde_json::to_value(&notebook).ok().map(|v| v.to_string());
            self.record_change("notebook", &notebook.id.to_string(), "create", data)
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %notebook.id, "Notebook created");
                Ok(notebook)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    // md:impl NotebookRepository for DbBackend > fn read_notebook
    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at,alias,vv,last_writer
                 FROM notebooks WHERE id = ?1",
                [id.to_string()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Self::row_to_notebook(&row),
            None => Err(StorageError::NotFound(id.to_string())),
        }
    }

    // md:impl NotebookRepository for DbBackend > fn update_notebook
    async fn update_notebook(&self, mut notebook: Notebook) -> Result<Notebook, StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        notebook.vv = self
            .next_local_vv("notebooks", &notebook.id.to_string())
            .await?;
        notebook.last_writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let affected = self
                .conn
                .execute(
                    "UPDATE notebooks SET title=?2,updated_at=?3,deleted_at=?4,alias=?5,vv=?6,last_writer=?7 WHERE id=?1",
                    libsql::params![
                        notebook.id.to_string(),
                        notebook.title.clone(),
                        notebook.updated_at.to_sortable_rfc3339(),
                        notebook.deleted_at.map(|d| d.to_sortable_rfc3339()),
                        notebook.alias.clone(),
                        vv_to_json(&notebook.vv),
                        notebook.last_writer.clone(),
                    ],
                )
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(notebook.id.to_string()));
            }
            let data = serde_json::to_value(&notebook).ok().map(|v| v.to_string());
            self.record_change("notebook", &notebook.id.to_string(), "update", data)
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(id = %notebook.id, "Notebook updated");
                Ok(notebook)
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    // md:impl NotebookRepository for DbBackend > fn delete_notebook
    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        let _write_guard = self.lock.write().await;
        self.begin().await?;
        let vv = self.next_local_vv("notebooks", &id.to_string()).await?;
        let writer = self.device_id.clone();
        let r: Result<(), StorageError> = async {
            let ts = now();
            let affected = self
                .conn
                .execute(
                    "UPDATE notebooks SET deleted_at=?2, updated_at=?2, vv=?3, last_writer=?4 WHERE id=?1",
                    libsql::params![id.to_string(), ts.to_sortable_rfc3339(), vv_to_json(&vv), writer.clone()],
                )
                .await?;
            if affected == 0 {
                return Err(StorageError::NotFound(id.to_string()));
            }
            self.record_change("notebook", &id.to_string(), "delete", Some(tombstone_data(ts, &vv, &writer)))
                .await
        }
        .await;
        match r {
            Ok(()) => {
                self.commit().await?;
                tracing::info!(%id, "Notebook deleted");
                Ok(())
            }
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    // md:impl NotebookRepository for DbBackend > fn list_notebooks
    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        let _read_guard = self.lock.read().await;
        let limit = super::effective_page_size(page_size);
        let (cursor_ts, cursor_id) = parse_cursor(page_token.as_deref());
        let mut rows = self
            .conn
            .query(
                "SELECT id,title,created_at,updated_at,deleted_at,alias,vv,last_writer
                 FROM notebooks
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
        let mut notebooks = Vec::new();
        while let Some(row) = rows.next().await? {
            notebooks.push(Self::row_to_notebook(&row)?);
        }
        Ok(build_page(notebooks, limit as usize, |nb| {
            format!("{}|{}", nb.created_at.to_sortable_rfc3339(), nb.id)
        }))
    }
}
