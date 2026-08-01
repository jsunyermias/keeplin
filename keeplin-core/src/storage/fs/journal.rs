// md:Overview
use std::io::SeekFrom;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{now, Notebook, Resource, Tag};

use super::convert::{
    fs_assoc_value, parse_epoch_header, snapshot_entry_from_sidecar, EpochHeader, LogEntry,
};
use super::io::atomic_write;
use super::FsBackend;

// md:impl FsBackend
impl FsBackend {
    // md:impl FsBackend > fn device_log_path
    pub(super) fn device_log_path(&self) -> PathBuf {
        self.root
            .join("logs")
            .join(format!("{}.log", self.device_id))
    }

    // md:impl FsBackend > fn append_log
    pub(super) async fn append_log(
        &self,
        entity_type: &str,
        entity_id: Uuid,
        operation: &str,
        data: serde_json::Value,
    ) -> Result<(), StorageError> {
        let _guard = self.global_log_lock.lock().await;
        let entry = LogEntry {
            timestamp: now(),
            entity_type: entity_type.to_string(),
            entity_id,
            operation: operation.to_string(),
            data,
        };
        let line = serde_json::to_string(&entry)? + "\n";
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.device_log_path())
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;
        drop(file);
        self.maybe_compact_global_log_locked().await
    }

    // md:impl FsBackend > fn maybe_compact_global_log_locked
    pub(super) async fn maybe_compact_global_log_locked(&self) -> Result<(), StorageError> {
        let path = self.device_log_path();
        let size = match tokio::fs::metadata(&path).await {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        if size < Self::GLOBAL_LOG_SOFT_BYTES {
            return Ok(());
        }
        if self.own_log_entry_count().await? <= Self::GLOBAL_LOG_COMPACT_THRESHOLD {
            return Ok(());
        }
        self.compact_global_log_locked().await
    }

    // md:impl FsBackend > fn own_log_entry_count
    pub(super) async fn own_log_entry_count(&self) -> Result<usize, StorageError> {
        let file = match tokio::fs::File::open(self.device_log_path()).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut count = 0usize;
        loop {
            line.clear();
            if reader.read_line(&mut line).await? == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() || parse_epoch_header(trimmed).is_some() {
                continue;
            }
            count += 1;
        }
        Ok(count)
    }

    // md:impl FsBackend > fn read_own_epoch
    pub(super) async fn read_own_epoch(&self) -> Result<u64, StorageError> {
        let (epoch, _len) = self.read_log_header(&self.device_log_path()).await?;
        Ok(epoch)
    }

    // md:impl FsBackend > fn compact_global_log_locked
    pub(super) async fn compact_global_log_locked(&self) -> Result<(), StorageError> {
        let new_epoch = self.read_own_epoch().await?.saturating_add(1);
        let (entries, unreadable) = self.build_global_snapshot().await?;
        if unreadable > 0 {
            tracing::error!(
                unreadable,
                "Global-log compaction skipped: unreadable sidecar file(s) would be \
                 silently dropped from the snapshot (see the errors above for paths). \
                 The journal keeps appending until they are repaired or restored from \
                 a backup or another device."
            );
            return Ok(());
        }

        let mut buf = serde_json::to_string(&EpochHeader { epoch: new_epoch })?;
        buf.push('\n');
        for entry in &entries {
            buf.push_str(&serde_json::to_string(entry)?);
            buf.push('\n');
        }

        atomic_write(&self.device_log_path(), buf.as_bytes()).await?;
        tracing::info!(
            epoch = new_epoch,
            entries = entries.len(),
            "Compacted global change log to a snapshot"
        );
        Ok(())
    }

    // md:impl FsBackend > fn build_global_snapshot
    pub(super) async fn build_global_snapshot(
        &self,
    ) -> Result<(Vec<LogEntry>, usize), StorageError> {
        let ts = now();
        let mut out = Vec::new();
        let mut unreadable = 0usize;

        for (dir, kind) in [("notebooks", "notebook"), ("tags", "tag")] {
            let mut rd = match tokio::fs::read_dir(self.root.join(dir)).await {
                Ok(rd) => rd,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            while let Some(e) = rd.next_entry().await? {
                let fname = e.file_name().to_string_lossy().into_owned();
                let Some(stem) = fname.strip_suffix(".ndjson") else {
                    continue;
                };
                let Ok(id) = Uuid::parse_str(stem) else {
                    continue;
                };
                let bytes = tokio::fs::read(e.path()).await?;
                let entry = match kind {
                    "notebook" => snapshot_entry_from_sidecar::<Notebook>(&bytes, kind, id, ts),
                    _ => snapshot_entry_from_sidecar::<Tag>(&bytes, kind, id, ts),
                };
                match entry {
                    Some(e) => out.push(e),
                    None => {
                        unreadable += 1;
                        tracing::error!(
                            path = %e.path().display(),
                            "Unreadable {kind} sidecar; restore it from a backup or \
                             another device (global-log compaction is paused until then)"
                        );
                    }
                }
            }
        }

        for note_id in self.all_note_ids().await? {
            for id in self.note_resource_ids(note_id).await? {
                let meta_path = self.resource_meta_path(note_id, id);
                let bytes = match tokio::fs::read(&meta_path).await {
                    Ok(bytes) => bytes,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e.into()),
                };
                match snapshot_entry_from_sidecar::<Resource>(&bytes, "resource", id, ts) {
                    Some(e) => out.push(e),
                    None => {
                        unreadable += 1;
                        tracing::error!(
                            path = %meta_path.display(),
                            "Unreadable resource sidecar; restore it from a backup or \
                             another device (global-log compaction is paused until then)"
                        );
                    }
                }
            }
        }

        if let Ok(mut notes_rd) = tokio::fs::read_dir(self.root.join("note_tags")).await {
            while let Some(note_entry) = notes_rd.next_entry().await? {
                let Ok(note_id) = Uuid::parse_str(&note_entry.file_name().to_string_lossy()) else {
                    continue;
                };
                let mut tags_rd = tokio::fs::read_dir(note_entry.path()).await?;
                while let Some(tag_entry) = tags_rd.next_entry().await? {
                    let Ok(tag_id) = Uuid::parse_str(&tag_entry.file_name().to_string_lossy())
                    else {
                        continue;
                    };
                    let Some(state) = self.read_assoc_state(&tag_entry.path()).await? else {
                        continue;
                    };
                    let op = if state.deleted_at.is_some() {
                        "remove"
                    } else {
                        "add"
                    };
                    out.push(LogEntry {
                        timestamp: ts,
                        entity_type: "note_tag".to_string(),
                        entity_id: note_id,
                        operation: op.to_string(),
                        data: fs_assoc_value(
                            tag_id,
                            state.updated_at,
                            &state.vv,
                            &state.last_writer,
                        ),
                    });
                }
            }
        }

        Ok((out, unreadable))
    }

    // md:impl FsBackend > fn log_offset_path
    pub(super) fn log_offset_path(&self, device_id: &str) -> PathBuf {
        self.root.join(".keeplin").join("offsets").join(device_id)
    }

    // md:impl FsBackend > fn read_log_offset
    pub(super) async fn read_log_offset(&self, device_id: &str) -> (u64, u64) {
        let raw = match tokio::fs::read_to_string(self.log_offset_path(device_id)).await {
            Ok(s) => s,
            Err(_) => return (0, 0),
        };
        let raw = raw.trim();
        match raw.split_once(':') {
            Some((epoch, offset)) => (
                epoch.trim().parse().unwrap_or(0),
                offset.trim().parse().unwrap_or(0),
            ),
            None => (0, raw.parse().unwrap_or(0)),
        }
    }

    // md:impl FsBackend > fn write_log_offset
    pub(super) async fn write_log_offset(
        &self,
        device_id: &str,
        epoch: u64,
        offset: u64,
    ) -> Result<(), StorageError> {
        atomic_write(
            &self.log_offset_path(device_id),
            format!("{epoch}:{offset}").as_bytes(),
        )
        .await
    }

    // md:impl FsBackend > fn read_log_header
    pub(super) async fn read_log_header(&self, path: &Path) -> Result<(u64, u64), StorageError> {
        let file = match tokio::fs::File::open(path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((0, 0)),
            Err(e) => return Err(e.into()),
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        match parse_epoch_header(line.trim()) {
            Some(epoch) => Ok((epoch, n as u64)),
            None => Ok((0, 0)),
        }
    }

    // md:impl FsBackend > fn read_other_logs_since
    pub(super) async fn read_other_logs_since(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<LogEntry>, StorageError> {
        let mut entries = Vec::new();
        let logs_dir = self.root.join("logs");
        let mut dir = match tokio::fs::read_dir(&logs_dir).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
            Err(e) => return Err(e.into()),
        };
        while let Some(dir_entry) = dir.next_entry().await? {
            let fname = dir_entry.file_name().to_string_lossy().into_owned();
            if fname == format!("{}.log", self.device_id) {
                continue;
            }
            if !fname.ends_with(".log") {
                continue;
            }
            let file = tokio::fs::File::open(dir_entry.path()).await?;
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    break;
                }
                let trimmed = line.trim();
                if trimmed.is_empty() || parse_epoch_header(trimmed).is_some() {
                    continue;
                }
                match serde_json::from_str::<LogEntry>(trimmed) {
                    Ok(e) if e.timestamp > since => entries.push(e),
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!("Skipping malformed log line: {err}");
                    }
                }
            }
        }
        Ok(entries)
    }

    // md:impl FsBackend > fn read_new_entries
    pub(super) async fn read_new_entries(&self) -> Result<Vec<LogEntry>, StorageError> {
        let mut entries = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.join("logs")).await?;
        while let Some(dir_entry) = dir.next_entry().await? {
            let fname = dir_entry.file_name().to_string_lossy().into_owned();
            if fname == format!("{}.log", self.device_id) {
                continue;
            }
            if !fname.ends_with(".log") {
                continue;
            }
            let device_id = fname.trim_end_matches(".log").to_owned();
            let (writer_epoch, header_len) = self.read_log_header(&dir_entry.path()).await?;
            let (stored_epoch, stored_offset) = self.read_log_offset(&device_id).await;
            let start = if writer_epoch != stored_epoch {
                header_len
            } else {
                stored_offset
            };

            let mut file = tokio::fs::File::open(dir_entry.path()).await?;
            file.seek(SeekFrom::Start(start)).await?;

            let mut reader = BufReader::new(file);
            let mut line = String::new();
            let mut new_offset = start;
            loop {
                line.clear();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    break;
                }
                new_offset += n as u64;
                let trimmed = line.trim();
                if trimmed.is_empty() || parse_epoch_header(trimmed).is_some() {
                    continue;
                }
                match serde_json::from_str::<LogEntry>(trimmed) {
                    Ok(e) => entries.push(e),
                    Err(err) => {
                        tracing::warn!("Skipping malformed log line: {err}");
                    }
                }
            }

            if writer_epoch != stored_epoch || new_offset > stored_offset {
                if let Err(e) = self
                    .write_log_offset(&device_id, writer_epoch, new_offset)
                    .await
                {
                    tracing::warn!("Could not save log offset for {device_id}: {e}");
                }
            }
        }
        Ok(entries)
    }
}
