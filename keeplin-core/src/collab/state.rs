//! Client-side line state for one collaborative note, plus the body↔lines
//! translation: materialising the flat markdown body frontends see, applying
//! server ops, and diffing a locally edited body into [`LineOp`]s.

use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use super::protocol::{LineOp, LineSnapshot, NoteLinesSnapshot};
use crate::storage::note_log::VersionVector;

/// In-memory mirror of a note's server-side line entities. Rebuilt from the
/// `Welcome` snapshot on every (re)connect — nothing here is durable.
#[derive(Debug, Clone, Default)]
pub struct NoteLines {
    pub order: Vec<Uuid>,
    pub lines: HashMap<Uuid, LineSnapshot>,
    pub vv: VersionVector,
}

impl NoteLines {
    pub fn from_snapshot(snapshot: NoteLinesSnapshot) -> Self {
        Self {
            order: snapshot.order,
            lines: snapshot.lines.into_iter().map(|l| (l.id, l)).collect(),
            vv: snapshot.vv,
        }
    }

    /// The flat body frontends see: live lines, in order, joined with '\n'.
    pub fn materialize(&self) -> String {
        self.order
            .iter()
            .filter_map(|id| self.lines.get(id))
            .filter(|l| l.deleted_at.is_none())
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Live line ids in order (the rows a body edit diffs against).
    fn live(&self) -> Vec<Uuid> {
        self.order
            .iter()
            .filter(|id| self.lines.get(id).is_some_and(|l| l.deleted_at.is_none()))
            .copied()
            .collect()
    }

    /// Apply one already-resolved server op to the mirror. The server is the
    /// source of truth: ops arrive validated and in `server_seq` order, so
    /// they are applied directly without re-resolving.
    pub fn apply(&mut self, op: &LineOp) {
        match op {
            LineOp::Insert {
                after_line_id,
                line_id,
                content,
                vv,
                last_writer,
                updated_at,
            } => {
                let pos = match after_line_id {
                    None => 0,
                    Some(after) => self
                        .order
                        .iter()
                        .position(|id| id == after)
                        .map(|i| i + 1)
                        .unwrap_or(self.order.len()),
                };
                self.order.insert(pos, *line_id);
                self.lines.insert(
                    *line_id,
                    LineSnapshot {
                        id: *line_id,
                        content: content.clone(),
                        created_at: *updated_at,
                        updated_at: *updated_at,
                        deleted_at: None,
                        vv: vv.clone(),
                        last_writer: last_writer.clone(),
                    },
                );
                merge_into(&mut self.vv, vv);
            }
            LineOp::Update {
                line_id,
                content,
                vv,
                last_writer,
                updated_at,
            } => {
                if let Some(line) = self.lines.get_mut(line_id) {
                    line.content = content.clone();
                    line.updated_at = *updated_at;
                    line.deleted_at = None;
                    merge_into(&mut line.vv, vv);
                    line.last_writer = last_writer.clone();
                }
            }
            LineOp::Delete {
                line_id,
                deleted_at,
                vv,
                last_writer,
                updated_at,
            } => {
                if let Some(line) = self.lines.get_mut(line_id) {
                    line.deleted_at = Some(*deleted_at);
                    line.updated_at = *updated_at;
                    merge_into(&mut line.vv, vv);
                    line.last_writer = last_writer.clone();
                }
            }
            LineOp::Move {
                line_ids,
                after_line_id,
                vv,
                ..
            } => {
                self.order.retain(|id| !line_ids.contains(id));
                let pos = match after_line_id {
                    None => 0,
                    Some(after) => self
                        .order
                        .iter()
                        .position(|id| id == after)
                        .map(|i| i + 1)
                        .unwrap_or(self.order.len()),
                };
                self.order.splice(pos..pos, line_ids.iter().copied());
                merge_into(&mut self.vv, vv);
            }
        }
    }

    /// Diff the current live lines against a newly edited flat `body` and
    /// return the ops that turn one into the other, applying them to the
    /// mirror as they are generated (optimistic local echo). Uses a common
    /// prefix/suffix trim; the middle is paired positionally (`Update`), with
    /// surplus old lines deleted and surplus new lines inserted.
    pub fn diff_body(&mut self, body: &str, device: &str) -> Vec<LineOp> {
        let now = Utc::now();
        let new_lines: Vec<&str> = if body.is_empty() {
            Vec::new()
        } else {
            body.split('\n').collect()
        };
        let old_ids = self.live();
        let old_contents: Vec<String> = old_ids
            .iter()
            .map(|id| self.lines[id].content.clone())
            .collect();

        let mut prefix = 0;
        while prefix < old_ids.len()
            && prefix < new_lines.len()
            && old_contents[prefix] == new_lines[prefix]
        {
            prefix += 1;
        }
        let mut suffix = 0;
        while suffix < old_ids.len() - prefix
            && suffix < new_lines.len() - prefix
            && old_contents[old_ids.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
        {
            suffix += 1;
        }

        let old_mid = &old_ids[prefix..old_ids.len() - suffix];
        let new_mid: Vec<String> = new_lines[prefix..new_lines.len() - suffix]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let mut ops = Vec::new();
        let paired = old_mid.len().min(new_mid.len());

        // Positionally paired lines become updates.
        for i in 0..paired {
            let id = old_mid[i];
            if self.lines[&id].content != new_mid[i] {
                let mut vv = self.lines[&id].vv.clone();
                bump(&mut vv, device);
                ops.push(LineOp::Update {
                    line_id: id,
                    content: new_mid[i].clone(),
                    vv,
                    last_writer: device.to_string(),
                    updated_at: now,
                });
            }
        }
        // Surplus old lines are tombstoned.
        for id in &old_mid[paired..] {
            let mut vv = self.lines[id].vv.clone();
            bump(&mut vv, device);
            ops.push(LineOp::Delete {
                line_id: *id,
                deleted_at: now,
                vv,
                last_writer: device.to_string(),
                updated_at: now,
            });
        }
        // Surplus new lines are inserted after the last paired/prefix line.
        // Inserts resolve against the ORDER entity, so each one must advance
        // the device's component past the previous — a single edit that adds
        // several lines carries strictly increasing vectors (the server would
        // drop the second insert as a replay otherwise).
        let mut anchor = if paired > 0 {
            Some(old_mid[paired - 1])
        } else if prefix > 0 {
            Some(old_ids[prefix - 1])
        } else {
            None
        };
        let mut order_vv = self.vv.clone();
        for content in &new_mid[paired..] {
            bump(&mut order_vv, device);
            let vv = order_vv.clone();
            let line_id = Uuid::new_v4();
            ops.push(LineOp::Insert {
                after_line_id: anchor,
                line_id,
                content: content.clone(),
                vv,
                last_writer: device.to_string(),
                updated_at: now,
            });
            anchor = Some(line_id);
        }

        for op in &ops {
            self.apply(op);
        }
        ops
    }
}

fn merge_into(target: &mut VersionVector, other: &VersionVector) {
    for (k, v) in other {
        let entry = target.entry(k.clone()).or_insert(0);
        if *v > *entry {
            *entry = *v;
        }
    }
}

fn bump(vv: &mut VersionVector, device: &str) {
    *vv.entry(device.to_string()).or_insert(0) += 1;
}
