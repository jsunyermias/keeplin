// md:Overview
use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{Note, Notebook};
use crate::storage::backend::DEFAULT_HISTORY_LIMIT;
use crate::storage::note_log::NoteOp;
use crate::storage::{EntityVersion, HistoryRepository};

use super::convert::{parse_epoch_header, LogEntry};
use super::FsBackend;

// md:impl FsBackend (global history)
impl FsBackend {
    // md:impl FsBackend (global history) > fn read_all_global_entries
    pub(super) async fn read_all_global_entries(
        &self,
    ) -> Result<Vec<(String, LogEntry)>, StorageError> {
        let dir = self.root.join("logs");
        let mut out = Vec::new();
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(device) = name.strip_suffix(".log") else {
                continue;
            };
            let bytes = tokio::fs::read(entry.path()).await?;
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines() {
                if line.trim().is_empty() || parse_epoch_header(line).is_some() {
                    continue;
                }
                if let Ok(le) = serde_json::from_str::<LogEntry>(line) {
                    out.push((device.to_string(), le));
                }
            }
        }
        Ok(out)
    }
}

// md:impl HistoryRepository for FsBackend
#[async_trait]
impl HistoryRepository for FsBackend {
    // md:impl HistoryRepository for FsBackend > fn note_history
    async fn note_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Note>>, StorageError> {
        let logs = self.read_note_logs(id).await?;
        let mut versions: Vec<EntityVersion<Note>> = logs
            .into_iter()
            .flatten()
            .map(|e| EntityVersion {
                timestamp: e.timestamp,
                device_id: e.device_id,
                entity: match e.op {
                    NoteOp::Upsert(note) => Some(note),
                    NoteOp::Tombstone { .. } => None,
                },
            })
            .collect();
        sort_and_cap(&mut versions, limit);
        Ok(versions)
    }

    // md:impl HistoryRepository for FsBackend > fn notebook_history
    async fn notebook_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Notebook>>, StorageError> {
        let entries = self.read_all_global_entries().await?;
        let mut versions: Vec<EntityVersion<Notebook>> = entries
            .into_iter()
            .filter(|(_, le)| le.entity_type == "notebook" && le.entity_id == id)
            .filter_map(|(device, le)| {
                let entity = match le.operation.as_str() {
                    "create" | "update" => Some(serde_json::from_value::<Notebook>(le.data).ok()?),
                    "delete" => None,
                    _ => return None,
                };
                Some(EntityVersion {
                    timestamp: le.timestamp,
                    device_id: device,
                    entity,
                })
            })
            .collect();
        sort_and_cap(&mut versions, limit);
        Ok(versions)
    }
}

// md:fn sort_and_cap
fn sort_and_cap<T>(versions: &mut Vec<EntityVersion<T>>, limit: u32) {
    versions.sort_by(|a, b| {
        b.timestamp
            .cmp(&a.timestamp)
            .then_with(|| b.device_id.cmp(&a.device_id))
    });
    let cap = if limit == 0 {
        DEFAULT_HISTORY_LIMIT
    } else {
        limit
    } as usize;
    versions.truncate(cap);
}
