//! Wire types of the keeplin-srv collaborative channel (`GET /api/ws?token=`),
//! mirroring the server's `protocol.rs`. JSON messages tagged with `type`;
//! line operations tagged with `op`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::note_log::VersionVector;

pub type LineId = Uuid;

/// A caret position inside a note (presence).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub line_id: LineId,
    pub column: usize,
}

/// One line as carried in snapshots: the full versioned entity, tombstones
/// included.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineSnapshot {
    pub id: LineId,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub vv: VersionVector,
    pub last_writer: String,
}

/// Full note state sent in `Welcome`: the versioned order plus every line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteLinesSnapshot {
    pub note_id: Uuid,
    pub order: Vec<LineId>,
    pub updated_at: DateTime<Utc>,
    pub vv: VersionVector,
    pub last_writer: String,
    pub lines: Vec<LineSnapshot>,
}

/// One line-level operation. `last_writer` and the vv component that advances
/// are this **device**'s id (the concurrency actor in server mode).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "PascalCase")]
pub enum LineOp {
    Insert {
        after_line_id: Option<LineId>,
        line_id: LineId,
        content: String,
        vv: VersionVector,
        last_writer: String,
        updated_at: DateTime<Utc>,
    },
    Update {
        line_id: LineId,
        content: String,
        vv: VersionVector,
        last_writer: String,
        updated_at: DateTime<Utc>,
    },
    Delete {
        line_id: LineId,
        deleted_at: DateTime<Utc>,
        vv: VersionVector,
        last_writer: String,
        updated_at: DateTime<Utc>,
    },
    Move {
        line_ids: Vec<LineId>,
        after_line_id: Option<LineId>,
        vv: VersionVector,
        last_writer: String,
        updated_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceInfo {
    pub user_id: String,
    pub display_name: String,
    pub cursor: Option<Cursor>,
}

/// Client → server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum CollabClientMsg {
    Join { note_id: Uuid },
    Leave { note_id: Uuid },
    Op { note_id: Uuid, ops: Vec<LineOp> },
    Cursor { note_id: Uuid, cursor: Cursor },
    Ack { server_seq: u64 },
}

/// Server → client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum CollabServerMsg {
    Welcome {
        note_id: Uuid,
        snapshot: NoteLinesSnapshot,
    },
    Op {
        server_seq: u64,
        note_id: Uuid,
        user_id: String,
        ops: Vec<LineOp>,
    },
    Presence {
        note_id: Uuid,
        users: Vec<PresenceInfo>,
    },
    Error {
        code: String,
        message: String,
    },
}
