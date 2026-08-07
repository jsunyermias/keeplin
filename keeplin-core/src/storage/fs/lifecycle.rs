// md:Overview
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::error::StorageError;
use crate::models::new_id;

use super::FsBackend;

// md:impl FsBackend
impl FsBackend {
    // md:impl FsBackend > fn new
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let root: PathBuf = root.into();
        let has_format_version = root.join(".keeplin").join("format_version").exists();
        if !has_format_version {
            if let Some(path) = Self::unexpected_fresh_store_entry(&root).await? {
                let entry = path.strip_prefix(&root).unwrap_or(&path);
                return Err(StorageError::InvalidState(format!(
                    "on-disk format stamp is missing; unexpected entry {} prevents treating this \
                     directory as a fresh store; expected version {}. Retain the untouched \
                     directory for manual recovery, choose an empty directory for a new store, \
                     or restore a backup already in the expected format",
                    entry.display(),
                    Self::FORMAT_VERSION
                )));
            }
        }
        let fresh = !has_format_version;
        Self::ensure_format_version(&root, fresh).await?;

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

        let device_id = Self::read_or_create_device_id(&root).await?;
        let backend = Self {
            root,
            device_id,
            note_write_lock: Arc::new(Mutex::new(())),
            global_log_lock: Arc::new(Mutex::new(())),
            note_index: Arc::new(RwLock::new(None)),
        };
        if fresh {
            tokio::fs::write(
                backend.format_version_path(),
                Self::FORMAT_VERSION.to_string(),
            )
            .await?;
        }
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

    // md:impl FsBackend > fn unexpected_fresh_store_entry
    pub(super) async fn unexpected_fresh_store_entry(
        root: &Path,
    ) -> Result<Option<PathBuf>, StorageError> {
        let allowed_root = [
            "notes",
            ".keeplin",
            "logs",
            "notebooks",
            "tags",
            "note_tags",
        ];
        let mut root_entries = match tokio::fs::read_dir(root).await {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        while let Some(entry) = root_entries.next_entry().await? {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !entry.file_type().await?.is_dir() || !allowed_root.contains(&name.as_ref()) {
                return Ok(Some(entry.path()));
            }
            if name == ".keeplin" {
                let mut metadata_entries = tokio::fs::read_dir(entry.path()).await?;
                while let Some(metadata_entry) = metadata_entries.next_entry().await? {
                    let metadata_name = metadata_entry.file_name();
                    let metadata_type = metadata_entry.file_type().await?;
                    if metadata_name == "device_id" && metadata_type.is_file() {
                        continue;
                    }
                    if metadata_name != "offsets" || !metadata_type.is_dir() {
                        return Ok(Some(metadata_entry.path()));
                    }
                    let mut offset_entries = tokio::fs::read_dir(metadata_entry.path()).await?;
                    if let Some(offset_entry) = offset_entries.next_entry().await? {
                        return Ok(Some(offset_entry.path()));
                    }
                }
            } else {
                let mut entries = tokio::fs::read_dir(entry.path()).await?;
                if let Some(unexpected) = entries.next_entry().await? {
                    return Ok(Some(unexpected.path()));
                }
            }
        }
        Ok(None)
    }

    // md:impl FsBackend > fn ensure_format_version
    pub(super) async fn ensure_format_version(
        root: &Path,
        fresh: bool,
    ) -> Result<(), StorageError> {
        let path = root.join(".keeplin").join("format_version");

        if fresh {
            return Ok(());
        }

        let current = if path.exists() {
            let stamp = tokio::fs::read_to_string(&path).await?;
            stamp.trim().parse::<u32>().map_err(|_| {
                StorageError::InvalidState(format!(
                    "on-disk format stamp is unparsable; expected version {}. Retain the \
                     untouched store for manual recovery, start a new store, or restore a \
                     backup already in the expected format",
                    Self::FORMAT_VERSION
                ))
            })?
        } else {
            return Err(StorageError::InvalidState(format!(
                "on-disk format stamp is missing (implied version 1); expected version {}. \
                 Retain the untouched store for manual recovery, start a new store, or restore \
                 a backup already in the expected format",
                Self::FORMAT_VERSION
            )));
        };

        if current > Self::FORMAT_VERSION {
            return Err(StorageError::InvalidState(format!(
                "on-disk format version {current} is newer than this build supports \
                 (max {}); upgrade keeplin to open this store",
                Self::FORMAT_VERSION
            )));
        }

        if current < Self::FORMAT_VERSION {
            return Err(StorageError::InvalidState(format!(
                "on-disk format version {current} is older than the expected version {}. Retain \
                 the untouched store for manual recovery, start a new store, or restore a \
                 backup already in the expected format",
                Self::FORMAT_VERSION
            )));
        }

        Ok(())
    }

    // md:impl FsBackend > fn read_or_create_device_id
    pub(super) async fn read_or_create_device_id(root: &Path) -> Result<String, StorageError> {
        let path = root.join(".keeplin").join("device_id");
        if path.exists() {
            let id = tokio::fs::read_to_string(&path).await?;
            Ok(id.trim().to_string())
        } else {
            let id = new_id().to_string();
            tokio::fs::write(&path, &id).await?;
            Ok(id)
        }
    }
}
