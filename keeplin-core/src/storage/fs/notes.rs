// md:Overview
use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{now, Change, Note};
use crate::storage::note_log::{self, NoteLogEntry, NoteOp, VersionVector};
use crate::storage::{NoteRepository, NotebookSortProfile, SortableRfc3339};

use super::convert::parse_note_log;
use super::io::atomic_write;
use super::pagination::PageCollector;
use super::FsBackend;

// md:NoteMeta
#[derive(Debug, Serialize, Deserialize)]
struct NoteMeta {
    note: Note,
    vv: VersionVector,
}

// md:NoteMetaEntry
#[derive(Debug, Clone)]
struct NoteMetaEntry {
    notebook_id: Uuid,
    created_at: DateTime<Utc>,
    effective_sort_key: u32,
    is_starred: bool,
}

// md:impl NoteMetaEntry
impl NoteMetaEntry {
    // md:impl NoteMetaEntry > fn from_note
    fn from_note(note: &Note) -> Self {
        Self {
            notebook_id: note.notebook_id,
            created_at: note.created_at,
            effective_sort_key: note.effective_sort_key(),
            is_starred: note.is_starred,
        }
    }
}

// md:NoteMetaIndex
#[derive(Debug, Default)]
pub(super) struct NoteMetaIndex {
    entries: std::collections::HashMap<Uuid, NoteMetaEntry>,
}

// md:impl NoteMetaIndex
impl NoteMetaIndex {
    // md:impl NoteMetaIndex > fn apply
    fn apply(&mut self, note: &Note) {
        if note.deleted_at.is_some() {
            self.entries.remove(&note.id);
        } else {
            self.entries.insert(note.id, NoteMetaEntry::from_note(note));
        }
    }
}

// md:fn parse_note_log
fn parse_note_log(bytes: &[u8]) -> Result<Vec<NoteLogEntry>, StorageError> {
    let mut out = Vec::new();
    for line in bytes.split(|b| *b == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let entry: NoteLogEntry = serde_json::from_slice(line)
            .map_err(|e| StorageError::CorruptedData(format!("ndjson decode: {e}")))?;
        out.push(entry);
    }
    Ok(out)
}

// md:impl FsBackend
impl FsBackend {
    // md:impl FsBackend > fn note_dir
    pub(super) fn note_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("notes").join(id.to_string())
    }

    // md:impl FsBackend > fn note_md_path
    pub(super) fn note_md_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("note.md")
    }

    // md:impl FsBackend > fn note_meta_path
    pub(super) fn note_meta_path(&self, id: Uuid) -> PathBuf {
        self.note_dir(id).join("meta.ndjson")
    }

    // md:impl FsBackend > fn note_log_path
    pub(super) fn note_log_path(&self, id: Uuid, device_id: &str) -> PathBuf {
        self.note_dir(id).join(format!("log.{device_id}.ndjson"))
    }

    // md:impl FsBackend > fn write_note_log
    pub(super) async fn write_note_log(
        &self,
        path: &Path,
        entries: &[NoteLogEntry],
    ) -> Result<(), StorageError> {
        let mut bytes = Vec::new();
        for entry in entries {
            let line = serde_json::to_vec(entry)
                .map_err(|e| StorageError::InvalidState(format!("ndjson encode: {e}")))?;
            bytes.extend_from_slice(&line);
            bytes.push(b'\n');
        }
        atomic_write(path, &bytes).await
    }

    // md:impl FsBackend > fn note_vv
    pub(super) async fn note_vv(&self, id: Uuid) -> Result<VersionVector, StorageError> {
        match self
            .read_sidecar::<NoteMeta>(&self.note_meta_path(id), id)
            .await
        {
            Ok(meta) => Ok(meta.vv),
            Err(StorageError::NotFound(_)) => Ok(VersionVector::new()),
            Err(e) => Err(e),
        }
    }

    // md:impl FsBackend > fn read_note_logs
    pub(super) async fn read_note_logs(
        &self,
        id: Uuid,
    ) -> Result<Vec<Vec<NoteLogEntry>>, StorageError> {
        let dir = self.note_dir(id);
        let mut logs = Vec::new();
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(logs),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("log.") && name.ends_with(".ndjson") {
                let bytes = tokio::fs::read(entry.path()).await?;
                match parse_note_log(&bytes) {
                    Ok(v) => logs.push(v),
                    Err(e) => tracing::error!(
                        note_id = %id,
                        path = %entry.path().display(),
                        "Unreadable note log excluded from merge — this device's history \
                         for the note is not being applied; restore the file from a backup \
                         or another device to recover it: {e}"
                    ),
                }
            }
        }
        Ok(logs)
    }

    // md:impl FsBackend > fn merge_note
    pub(super) async fn merge_note(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
        let logs = self.read_note_logs(id).await?;
        Ok(note_log::merge(&logs).note)
    }

    // md:impl FsBackend > fn materialize
    pub(super) async fn materialize(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
        let logs = self.read_note_logs(id).await?;
        let merged = note_log::merge(&logs);
        match merged.note {
            None => Ok(None),
            Some(note) => {
                if merged.conflict {
                    tracing::warn!(%id, "Concurrent note edit resolved by the (timestamp, device-id) tiebreak");
                }
                self.persist_note_projection(&note, &merged.vv).await?;
                Ok(Some(note))
            }
        }
    }

    // md:impl FsBackend > fn persist_note_projection
    pub(super) async fn persist_note_projection(
        &self,
        note: &Note,
        vv: &VersionVector,
    ) -> Result<(), StorageError> {
        tokio::fs::create_dir_all(self.note_dir(note.id)).await?;
        atomic_write(&self.note_md_path(note.id), note.body.as_bytes()).await?;
        let mut meta_note = note.clone();
        meta_note.body = String::new();
        self.write_sidecar(
            &self.note_meta_path(note.id),
            &NoteMeta {
                note: meta_note,
                vv: vv.clone(),
            },
        )
        .await?;
        if let Some(idx) = self.note_index.write().await.as_mut() {
            idx.apply(note);
        }
        Ok(())
    }

    // md:impl FsBackend > fn read_note_projection
    pub(super) async fn read_note_projection(
        &self,
        id: Uuid,
    ) -> Result<Option<Note>, StorageError> {
        let meta: NoteMeta = match self.read_sidecar(&self.note_meta_path(id), id).await {
            Ok(m) => m,
            Err(StorageError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(Some(meta.note))
    }

    // md:impl FsBackend > fn with_note_index
    pub(super) async fn with_note_index<R>(
        &self,
        f: impl FnOnce(&NoteMetaIndex) -> R,
    ) -> Result<R, StorageError> {
        {
            let guard = self.note_index.read().await;
            if let Some(idx) = guard.as_ref() {
                return Ok(f(idx));
            }
        }
        let mut guard = self.note_index.write().await;
        if guard.is_none() {
            *guard = Some(self.build_note_index().await?);
        }
        Ok(f(guard.as_ref().expect("index was just built")))
    }

    // md:impl FsBackend > fn build_note_index
    pub(super) async fn build_note_index(&self) -> Result<NoteMetaIndex, StorageError> {
        let mut idx = NoteMetaIndex::default();
        let mut dir = match tokio::fs::read_dir(self.root.join("notes")).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(idx),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = dir.next_entry().await? {
            let Ok(id) = Uuid::parse_str(&entry.file_name().to_string_lossy()) else {
                continue;
            };
            let note = match self.read_note_projection(id).await {
                Ok(Some(note)) => Some(note),
                Ok(None) => self.merge_note(id).await?,
                Err(e) => {
                    tracing::warn!("Rebuilding note {id} from logs (projection unreadable): {e}");
                    self.merge_note(id).await?
                }
            };
            if let Some(note) = note {
                if note.deleted_at.is_none() {
                    idx.apply(&note);
                }
            }
        }
        Ok(idx)
    }

    // md:impl FsBackend > fn materialize_page
    pub(super) async fn materialize_page(&self, ids: Vec<Uuid>) -> Result<Vec<Note>, StorageError> {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            match self.merge_note(id).await {
                Ok(Some(n)) if n.deleted_at.is_none() => out.push(n),
                Ok(_) => {}
                Err(e) => tracing::warn!("Could not merge note {id} for listing: {e}"),
            }
        }
        Ok(out)
    }

    // md:impl FsBackend > fn append_note_op
    pub(super) async fn append_note_op(&self, id: Uuid, op: NoteOp) -> Result<Note, StorageError> {
        let _write_guard = self.note_write_lock.lock().await;
        tokio::fs::create_dir_all(self.note_dir(id)).await?;
        let mut vv = note_log::merge(&self.read_note_logs(id).await?).vv;
        note_log::increment(&mut vv, &self.device_id);
        let log_path = self.note_log_path(id, &self.device_id);
        let mut log: Vec<NoteLogEntry> = if log_path.exists() {
            let bytes = tokio::fs::read(&log_path).await?;
            parse_note_log(&bytes)?
        } else {
            Vec::new()
        };
        log.push(NoteLogEntry {
            vv,
            timestamp: now(),
            device_id: self.device_id.clone(),
            op,
        });
        if log.len() > Self::NOTE_LOG_COMPACT_THRESHOLD {
            log = note_log::compact_own_log(&log);
        }
        self.write_note_log(&log_path, &log).await?;
        self.materialize(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))
    }

    // md:impl FsBackend > fn collect_advanced_notes
    pub(super) async fn collect_advanced_notes(&self) -> Result<Vec<Change>, StorageError> {
        let mut changes = Vec::new();
        let notes_dir = self.root.join("notes");
        let mut rd = match tokio::fs::read_dir(&notes_dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(changes),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            let id = match Uuid::parse_str(&entry.file_name().to_string_lossy()) {
                Ok(id) => id,
                Err(_) => continue,
            };
            let old_vv = self.note_vv(id).await?;
            let logs = self.read_note_logs(id).await?;
            let merged = note_log::merge(&logs);
            if merged.vv != old_vv {
                if let Some(note) = merged.note {
                    self.persist_note_projection(&note, &merged.vv).await?;
                    match note.deleted_at {
                        Some(deleted_at) => changes.push(Change::NoteDelete {
                            id,
                            deleted_at,
                            vv: merged.winner_vv.clone(),
                            last_writer: merged.winner_device.clone(),
                        }),
                        None => changes.push(Change::NoteUpdate { note }),
                    }
                }
            }
        }
        Ok(changes)
    }
}

// md:impl NoteRepository for FsBackend
#[async_trait]
impl NoteRepository for FsBackend {
    // md:impl NoteRepository for FsBackend > fn create_note
    async fn create_note(&self, note: Note) -> Result<Note, StorageError> {
        let merged = self.append_note_op(note.id, NoteOp::Upsert(note)).await?;
        tracing::info!(id = %merged.id, "Note created");
        Ok(merged)
    }

    // md:impl NoteRepository for FsBackend > fn read_note
    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        self.merge_note(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))
    }

    // md:impl NoteRepository for FsBackend > fn update_note
    async fn update_note(&self, note: Note) -> Result<Note, StorageError> {
        if self.read_note_logs(note.id).await?.is_empty() {
            return Err(StorageError::NotFound(note.id.to_string()));
        }
        let id = note.id;
        let prior_deleted = self.merge_note(id).await?.and_then(|n| n.deleted_at);
        let merged = self.append_note_op(id, NoteOp::Upsert(note)).await?;
        if merged.deleted_at.is_none() {
            if let Some(old_ts) = prior_deleted {
                self.cascade_unstamp_resources(id, old_ts).await?;
            }
        }
        tracing::info!(id = %merged.id, "Note updated");
        Ok(merged)
    }

    // md:impl NoteRepository for FsBackend > fn delete_note
    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        if self.read_note_logs(id).await?.is_empty() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let ts = now();
        self.append_note_op(id, NoteOp::Tombstone { deleted_at: ts })
            .await?;
        self.cascade_stamp_resources(id, ts).await?;
        tracing::info!(%id, "Note deleted");
        Ok(())
    }

    // md:impl NoteRepository for FsBackend > fn list_notes
    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let (ids, next) = self
            .with_note_index(|idx| {
                let mut collector = PageCollector::new(limit, page_token.as_deref());
                for (id, entry) in &idx.entries {
                    collector.push((entry.created_at.to_sortable_rfc3339(), *id), *id);
                }
                collector.into_page()
            })
            .await?;
        Ok((self.materialize_page(ids).await?, next))
    }

    // md:impl NoteRepository for FsBackend > fn list_notes_in_notebook
    async fn list_notes_in_notebook(
        &self,
        notebook_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let (ids, next) = self
            .with_note_index(|idx| {
                let mut collector = PageCollector::new(limit, page_token.as_deref());
                for (id, entry) in &idx.entries {
                    if entry.notebook_id == notebook_id {
                        collector.push((format!("{:010}", entry.effective_sort_key), *id), *id);
                    }
                }
                collector.into_page()
            })
            .await?;
        Ok((self.materialize_page(ids).await?, next))
    }

    // md:impl NoteRepository for FsBackend > fn list_starred_notes
    async fn list_starred_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let limit = super::effective_page_size(page_size) as usize;
        let (ids, next) = self
            .with_note_index(|idx| {
                let mut collector = PageCollector::new(limit, page_token.as_deref());
                for (id, entry) in &idx.entries {
                    if entry.is_starred {
                        collector.push((entry.created_at.to_sortable_rfc3339(), *id), *id);
                    }
                }
                collector.into_page()
            })
            .await?;
        Ok((self.materialize_page(ids).await?, next))
    }

    // md:impl NoteRepository for FsBackend > fn notebook_sort_profile
    async fn notebook_sort_profile(
        &self,
        notebook_id: Uuid,
    ) -> Result<NotebookSortProfile, StorageError> {
        let keys = self
            .with_note_index(|idx| {
                idx.entries
                    .values()
                    .filter(|e| e.notebook_id == notebook_id)
                    .map(|e| e.effective_sort_key)
                    .collect::<Vec<u32>>()
            })
            .await?;
        Ok(NotebookSortProfile::from_effective_keys(keys))
    }
}
