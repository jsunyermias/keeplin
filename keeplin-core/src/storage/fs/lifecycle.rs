// md:Overview
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::error::StorageError;
use crate::models::new_id;

use super::notes::NoteMetaIndex;
use super::FsBackend;

// md:impl FsBackend
impl FsBackend {
    // md:impl FsBackend > fn new
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let root: PathBuf = root.into();

        for dir in &[
            "notes",
            ".keeplin",
            ".keeplin/offsets",
            "logs",
            "notebooks",
            "tags",
            "note_tags",
        ] {
            tokio::fs::create_dir_all(root.join(dir)).await?;
        }

        let removed = Self::sweep_orphan_tmp_files(&root).await;
        if removed > 0 {
            tracing::info!(
                removed,
                "Removed orphaned .tmp files left by interrupted writes"
            );
        }

        let conflicts = Self::scan_sync_conflicts(&root).await;
        if !conflicts.is_empty() {
            for path in &conflicts {
                tracing::error!(path = %path.display(), "Syncthing conflict copy detected");
            }
            tracing::error!(
                count = conflicts.len(),
                "Syncthing '*.sync-conflict-*' files exist in this store. Every keeplin \
                 file has a single writer, so conflict copies mean two devices are \
                 fighting over the same files — almost always because `.keeplin/` (this \
                 device's identity) was replicated instead of excluded via .stignore. \
                 Fix the Syncthing ignore rules (see README, 'Multi-device setup with \
                 Syncthing'), then reconcile each conflict copy manually before trusting \
                 further writes."
            );
        }

        let (device_id, fresh) = Self::read_or_create_device_id(&root).await?;
        let backend = Self {
            root,
            device_id,
            note_write_lock: Arc::new(Mutex::new(())),
            global_log_lock: Arc::new(Mutex::new(())),
            note_index: Arc::new(RwLock::new(None)),
        };
        backend.ensure_format_version(fresh).await?;
        Ok(backend)
    }

    // md:impl FsBackend > fn sweep_orphan_tmp_files
    pub(super) async fn sweep_orphan_tmp_files(root: &Path) -> usize {
        let mut removed = 0usize;
        for flat in ["notebooks", "tags", "logs", ".keeplin", ".keeplin/offsets"] {
            removed += Self::sweep_tmp_in_dir(&root.join(flat)).await;
        }
        if let Ok(mut rd) = tokio::fs::read_dir(root.join("notes")).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    removed += Self::sweep_tmp_in_dir(&entry.path()).await;
                    removed += Self::sweep_tmp_in_dir(&entry.path().join("resources")).await;
                }
            }
        }
        if let Ok(mut rd) = tokio::fs::read_dir(root.join("note_tags")).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    removed += Self::sweep_tmp_in_dir(&entry.path()).await;
                }
            }
        }
        removed
    }

    // md:impl FsBackend > fn scan_sync_conflicts
    pub(super) async fn scan_sync_conflicts(root: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut dirs: Vec<PathBuf> = [
            "",
            "notebooks",
            "tags",
            "logs",
            ".keeplin",
            ".keeplin/offsets",
        ]
        .iter()
        .map(|d| root.join(d))
        .collect();
        if let Ok(mut rd) = tokio::fs::read_dir(root.join("notes")).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    dirs.push(entry.path());
                    dirs.push(entry.path().join("resources"));
                }
            }
        }
        if let Ok(mut rd) = tokio::fs::read_dir(root.join("note_tags")).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    dirs.push(entry.path());
                }
            }
        }
        for dir in dirs {
            let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
                continue;
            };
            while let Ok(Some(entry)) = rd.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.contains(".sync-conflict-")
                    && entry
                        .file_type()
                        .await
                        .map(|t| t.is_file())
                        .unwrap_or(false)
                {
                    found.push(entry.path());
                }
            }
        }
        found
    }

    // md:impl FsBackend > fn sweep_tmp_in_dir
    pub(super) async fn sweep_tmp_in_dir(dir: &Path) -> usize {
        let mut removed = 0usize;
        let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
            return 0;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".tmp") || name.starts_with(".syncthing.") {
                continue;
            }
            if !entry
                .file_type()
                .await
                .map(|t| t.is_file())
                .unwrap_or(false)
            {
                continue;
            }
            if tokio::fs::remove_file(entry.path()).await.is_ok() {
                tracing::debug!(path = %entry.path().display(), "Removed orphaned temp file");
                removed += 1;
            }
        }
        removed
    }

    pub(super) const FORMAT_VERSION: u32 = 8;

    pub(super) const NOTE_LOG_COMPACT_THRESHOLD: usize = 256;

    pub(super) const GLOBAL_LOG_COMPACT_THRESHOLD: usize = 512;

    pub(super) const GLOBAL_LOG_SOFT_BYTES: u64 = 64 * 1024;

    // md:impl FsBackend > fn format_version_path
    pub(super) fn format_version_path(&self) -> PathBuf {
        self.root.join(".keeplin").join("format_version")
    }

    // md:impl FsBackend > fn ensure_format_version
    pub(super) async fn ensure_format_version(&self, fresh: bool) -> Result<(), StorageError> {
        let path = self.format_version_path();

        if fresh {
            tokio::fs::write(&path, Self::FORMAT_VERSION.to_string()).await?;
            return Ok(());
        }

        let current = if path.exists() {
            tokio::fs::read_to_string(&path)
                .await?
                .trim()
                .parse::<u32>()
                .unwrap_or(1)
        } else {
            1
        };

        if current > Self::FORMAT_VERSION {
            return Err(StorageError::InvalidState(format!(
                "on-disk format version {current} is newer than this build supports \
                 (max {}); upgrade keeplin to open this store",
                Self::FORMAT_VERSION
            )));
        }

        for version in (current + 1)..=Self::FORMAT_VERSION {
            self.apply_format_migration(version).await?;
            tokio::fs::write(&path, version.to_string()).await?;
            tracing::info!(version, "Applied filesystem format migration");
        }

        tokio::fs::write(&path, Self::FORMAT_VERSION.to_string()).await?;
        Ok(())
    }

    // md:impl FsBackend > fn apply_format_migration
    pub(super) async fn apply_format_migration(&self, version: u32) -> Result<(), StorageError> {
        match version {
            2..=8 => Ok(()),
            other => Err(StorageError::InvalidState(format!(
                "no filesystem migration defined for format version {other}"
            ))),
        }
    }

    // md:impl FsBackend > fn read_or_create_device_id
    pub(super) async fn read_or_create_device_id(
        root: &Path,
    ) -> Result<(String, bool), StorageError> {
        let path = root.join(".keeplin").join("device_id");
        if path.exists() {
            let id = tokio::fs::read_to_string(&path).await?;
            Ok((id.trim().to_string(), false))
        } else {
            let id = new_id().to_string();
            tokio::fs::write(&path, &id).await?;
            Ok((id, true))
        }
    }
}
