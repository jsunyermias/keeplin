// md:Overview
use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use super::protocol::{LineOp, LineSnapshot, NoteLinesSnapshot};
use crate::format::{self, LimitViolation};
use crate::storage::note_log::VersionVector;

// md:NoteLines
#[derive(Debug, Clone, Default)]
pub struct NoteLines {
    pub order: Vec<Uuid>,
    pub lines: HashMap<Uuid, LineSnapshot>,
    pub vv: VersionVector,
}

// md:impl NoteLines
impl NoteLines {
    // md:impl NoteLines > fn from_snapshot
    pub fn from_snapshot(snapshot: NoteLinesSnapshot) -> Self {
        Self {
            order: snapshot.order,
            lines: snapshot.lines.into_iter().map(|l| (l.id, l)).collect(),
            vv: snapshot.vv,
        }
    }

    // md:impl NoteLines > fn materialize
    pub fn materialize(&self) -> String {
        self.order
            .iter()
            .filter_map(|id| self.lines.get(id))
            .filter(|l| l.deleted_at.is_none())
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    // md:impl NoteLines > fn live
    fn live(&self) -> Vec<Uuid> {
        self.order
            .iter()
            .filter(|id| self.lines.get(id).is_some_and(|l| l.deleted_at.is_none()))
            .copied()
            .collect()
    }

    // md:impl NoteLines > fn apply
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

    // md:impl NoteLines > fn diff_body
    pub fn diff_body(&mut self, body: &str, device: &str) -> Result<Vec<LineOp>, LimitViolation> {
        format::check_body(body)?;
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
        Ok(ops)
    }
}

// md:fn merge_into
fn merge_into(target: &mut VersionVector, other: &VersionVector) {
    for (k, v) in other {
        let entry = target.entry(k.clone()).or_insert(0);
        if *v > *entry {
            *entry = *v;
        }
    }
}

// md:fn bump
fn bump(vv: &mut VersionVector, device: &str) {
    *vv.entry(device.to_string()).or_insert(0) += 1;
}

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;

    // md:mod tests > fn diff_body_accepts_a_line_at_the_byte_limit_and_rejects_one_byte_more
    #[test]
    fn diff_body_accepts_a_line_at_the_byte_limit_and_rejects_one_byte_more() {
        let mut lines = NoteLines::default();
        let at_limit = "a".repeat(format::MAX_LINE_BYTES);
        assert_eq!(lines.diff_body(&at_limit, "dev").unwrap().len(), 1);
        assert_eq!(lines.materialize(), at_limit);

        let over_limit = "a".repeat(format::MAX_LINE_BYTES + 1);
        let violation = lines.diff_body(&over_limit, "dev").unwrap_err();
        assert_eq!(violation.code(), format::CODE_LINE_TOO_LONG);
        assert_eq!(
            lines.materialize(),
            at_limit,
            "a rejected edit leaves the note untouched"
        );
    }

    // md:mod tests > fn diff_body_accepts_the_line_count_limit_and_rejects_one_line_more
    #[test]
    fn diff_body_accepts_the_line_count_limit_and_rejects_one_line_more() {
        let mut lines = NoteLines::default();
        let at_limit = vec!["x"; format::MAX_LINES_PER_NOTE].join("\n");
        assert_eq!(
            lines.diff_body(&at_limit, "dev").unwrap().len(),
            format::MAX_LINES_PER_NOTE
        );

        let over_limit = vec!["x"; format::MAX_LINES_PER_NOTE + 1].join("\n");
        let violation = lines.diff_body(&over_limit, "dev").unwrap_err();
        assert_eq!(violation.code(), format::CODE_TOO_MANY_LINES);
        assert_eq!(
            lines.order.len(),
            format::MAX_LINES_PER_NOTE,
            "a rejected edit emits no Insert"
        );
    }
}
