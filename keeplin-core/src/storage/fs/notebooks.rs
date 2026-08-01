// md:Overview
use std::path::PathBuf;

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{now, Notebook};
use crate::storage::note_log;
use crate::storage::{NotebookRepository, SortableRfc3339};

use super::convert::fs_tombstone_value;
use super::pagination::paginate;
use super::FsBackend;

// md:impl FsBackend
impl FsBackend {
    // md:impl FsBackend > fn notebook_path
    pub(super) fn notebook_path(&self, id: Uuid) -> PathBuf {
        self.root.join("notebooks").join(format!("{id}.ndjson"))
    }
}

// md:impl NotebookRepository for FsBackend
#[async_trait]
impl NotebookRepository for FsBackend {
    // md:impl NotebookRepository for FsBackend > fn create_notebook
    async fn create_notebook(&self, mut notebook: Notebook) -> Result<Notebook, StorageError> {
        let path = self.notebook_path(notebook.id);
        notebook.vv = self.next_sidecar_vv(&path).await?;
        notebook.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &notebook).await?;
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

    // md:impl NotebookRepository for FsBackend > fn read_notebook
    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        self.read_sidecar(&self.notebook_path(id), id).await
    }

    // md:impl NotebookRepository for FsBackend > fn update_notebook
    async fn update_notebook(&self, mut notebook: Notebook) -> Result<Notebook, StorageError> {
        let path = self.notebook_path(notebook.id);
        if !path.exists() {
            return Err(StorageError::NotFound(notebook.id.to_string()));
        }
        notebook.vv = self.next_sidecar_vv(&path).await?;
        notebook.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &notebook).await?;
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

    // md:impl NotebookRepository for FsBackend > fn delete_notebook
    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        let path = self.notebook_path(id);
        let mut nb: Notebook = self.read_sidecar(&path, id).await?;
        let ts = now();
        nb.deleted_at = Some(ts);
        nb.updated_at = ts;
        note_log::increment(&mut nb.vv, &self.device_id);
        nb.last_writer = self.device_id.clone();
        self.write_sidecar(&path, &nb).await?;
        self.append_log(
            "notebook",
            id,
            "delete",
            fs_tombstone_value(ts, &nb.vv, &nb.last_writer),
        )
        .await?;
        tracing::info!(%id, "Notebook deleted");
        Ok(())
    }

    // md:impl NotebookRepository for FsBackend > fn list_notebooks
    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        let limit = crate::storage::effective_page_size(page_size) as usize;
        let mut notebooks = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("notebooks")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(stem) = fname.strip_suffix(".ndjson") {
                if let Ok(id) = Uuid::parse_str(stem) {
                    match self.read_sidecar::<Notebook>(&entry.path(), id).await {
                        Ok(nb) if nb.deleted_at.is_none() => notebooks.push(nb),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("Could not load notebook {id}: {e}"),
                    }
                }
            }
        }
        notebooks.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(paginate(notebooks, limit, page_token.as_deref(), |nb| {
            (nb.created_at.to_sortable_rfc3339(), nb.id)
        }))
    }
}
