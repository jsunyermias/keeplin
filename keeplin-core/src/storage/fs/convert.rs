// md:Overview
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::Change;
use crate::storage::note_log::VersionVector;

// md:LogEntry
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct LogEntry {
    pub(super) timestamp: DateTime<Utc>,
    #[serde(default = "default_entity_type")]
    pub(super) entity_type: String,
    #[serde(alias = "note_id")]
    pub(super) entity_id: Uuid,
    pub(super) operation: String,
    pub(super) data: serde_json::Value,
}

// md:fn default_entity_type
pub(super) fn default_entity_type() -> String {
    "note".to_string()
}

// md:EpochHeader
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct EpochHeader {
    #[serde(rename = "__keeplin_epoch__")]
    pub(super) epoch: u64,
}

// md:fn parse_epoch_header
pub(super) fn parse_epoch_header(line: &str) -> Option<u64> {
    serde_json::from_str::<EpochHeader>(line)
        .ok()
        .map(|h| h.epoch)
}

// md:fn fs_tombstone_value
pub(super) fn fs_tombstone_value(
    deleted_at: DateTime<Utc>,
    vv: &VersionVector,
    last_writer: &str,
) -> serde_json::Value {
    serde_json::json!({
        "deleted_at": deleted_at,
        "vv": vv,
        "last_writer": last_writer,
    })
}

// md:fn fs_assoc_value
pub(super) fn fs_assoc_value(
    tag_id: Uuid,
    updated_at: DateTime<Utc>,
    vv: &VersionVector,
    last_writer: &str,
) -> serde_json::Value {
    serde_json::json!({
        "tag_id": tag_id,
        "updated_at": updated_at,
        "vv": vv,
        "last_writer": last_writer,
    })
}

// md:fn snapshot_entry_from_sidecar
pub(super) fn snapshot_entry_from_sidecar<T: serde::Serialize + serde::de::DeserializeOwned>(
    bytes: &[u8],
    kind: &str,
    id: Uuid,
    ts: DateTime<Utc>,
) -> Option<LogEntry> {
    let concrete: T = serde_json::from_slice(bytes).ok()?;
    let value = serde_json::to_value(&concrete).ok()?;
    Some(snapshot_entry_from_value(kind, id, ts, value))
}

// md:fn snapshot_entry_from_value
pub(super) fn snapshot_entry_from_value(
    kind: &str,
    id: Uuid,
    ts: DateTime<Utc>,
    value: serde_json::Value,
) -> LogEntry {
    let deleted_at = value
        .get("deleted_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok());
    match deleted_at {
        Some(del) => {
            let vv = value
                .get("vv")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let last_writer = value
                .get("last_writer")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            LogEntry {
                timestamp: ts,
                entity_type: kind.to_string(),
                entity_id: id,
                operation: "delete".to_string(),
                data: fs_tombstone_value(del, &vv, &last_writer),
            }
        }
        None => LogEntry {
            timestamp: ts,
            entity_type: kind.to_string(),
            entity_id: id,
            operation: "create".to_string(),
            data: value,
        },
    }
}

// md:fn fs_assoc_from_data
pub(super) fn fs_assoc_from_data(
    data: &serde_json::Value,
    fallback_ts: DateTime<Utc>,
) -> (DateTime<Utc>, VersionVector, String) {
    let updated_at = data
        .get("updated_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(fallback_ts);
    let vv = data
        .get("vv")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let last_writer = data
        .get("last_writer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (updated_at, vv, last_writer)
}

// md:fn fs_tombstone_from_data
pub(super) fn fs_tombstone_from_data(
    data: &serde_json::Value,
    fallback_ts: DateTime<Utc>,
) -> (DateTime<Utc>, VersionVector, String) {
    let deleted_at = data
        .get("deleted_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(fallback_ts);
    let vv = data
        .get("vv")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let last_writer = data
        .get("last_writer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (deleted_at, vv, last_writer)
}

// md:fn log_entry_to_change
pub(super) fn log_entry_to_change(entry: LogEntry) -> Option<Change> {
    let id = entry.entity_id;
    let ts = entry.timestamp;
    match (entry.entity_type.as_str(), entry.operation.as_str()) {
        ("note", "create") | ("note", "note_create") => serde_json::from_value(entry.data)
            .ok()
            .map(|note| Change::NoteCreate { note }),
        ("note", "update") | ("note", "note_update") => serde_json::from_value(entry.data)
            .ok()
            .map(|note| Change::NoteUpdate { note }),
        ("note", "delete") | ("note", "note_delete") => {
            let (deleted_at, vv, last_writer) = fs_tombstone_from_data(&entry.data, ts);
            Some(Change::NoteDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            })
        }
        ("notebook", "create") => serde_json::from_value(entry.data)
            .ok()
            .map(|notebook| Change::NotebookCreate { notebook }),
        ("notebook", "update") => serde_json::from_value(entry.data)
            .ok()
            .map(|notebook| Change::NotebookUpdate { notebook }),
        ("notebook", "delete") => {
            let (deleted_at, vv, last_writer) = fs_tombstone_from_data(&entry.data, ts);
            Some(Change::NotebookDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            })
        }
        ("tag", "create") => serde_json::from_value(entry.data)
            .ok()
            .map(|tag| Change::TagCreate { tag }),
        ("tag", "update") => serde_json::from_value(entry.data)
            .ok()
            .map(|tag| Change::TagUpdate { tag }),
        ("tag", "delete") => {
            let (deleted_at, vv, last_writer) = fs_tombstone_from_data(&entry.data, ts);
            Some(Change::TagDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            })
        }
        ("note_tag", "add") => {
            let tag_id: Uuid = entry.data["tag_id"].as_str()?.parse().ok()?;
            let (updated_at, vv, last_writer) = fs_assoc_from_data(&entry.data, ts);
            Some(Change::NoteTagAdd {
                note_id: id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            })
        }
        ("note_tag", "remove") => {
            let tag_id: Uuid = entry.data["tag_id"].as_str()?.parse().ok()?;
            let (updated_at, vv, last_writer) = fs_assoc_from_data(&entry.data, ts);
            Some(Change::NoteTagRemove {
                note_id: id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            })
        }
        ("resource", "create") => {
            serde_json::from_value(entry.data)
                .ok()
                .map(|resource| Change::ResourceCreate {
                    resource,
                    data: None,
                })
        }
        ("resource", "delete") => {
            let (deleted_at, vv, last_writer) = fs_tombstone_from_data(&entry.data, ts);
            Some(Change::ResourceDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            })
        }
        _ => None,
    }
}
