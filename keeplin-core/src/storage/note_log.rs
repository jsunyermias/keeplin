// md:Overview
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::Note;

// md:VersionVector
pub type VersionVector = BTreeMap<String, u64>;

// md:fn increment
pub fn increment(vv: &mut VersionVector, device: &str) {
    *vv.entry(device.to_string()).or_insert(0) += 1;
}

// md:fn dominates
pub fn dominates(a: &VersionVector, b: &VersionVector) -> bool {
    b.iter()
        .all(|(k, &bv)| a.get(k).copied().unwrap_or(0) >= bv)
}

// md:fn join
pub fn join(a: &VersionVector, b: &VersionVector) -> VersionVector {
    let mut out = a.clone();
    for (k, &bv) in b {
        let slot = out.entry(k.clone()).or_insert(0);
        *slot = (*slot).max(bv);
    }
    out
}

// md:NoteOp
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum NoteOp {
    Upsert(Note),
    Tombstone { deleted_at: DateTime<Utc> },
}

// md:NoteLogEntry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteLogEntry {
    pub vv: VersionVector,
    pub timestamp: DateTime<Utc>,
    pub device_id: String,
    pub op: NoteOp,
}

// md:Merged
#[derive(Debug, Clone)]
pub struct Merged {
    pub note: Option<Note>,
    pub vv: VersionVector,
    pub winner_vv: VersionVector,
    pub winner_device: String,
    pub conflict: bool,
}

// md:fn merge
pub fn merge(logs: &[Vec<NoteLogEntry>]) -> Merged {
    let heads: Vec<&NoteLogEntry> = logs.iter().filter_map(|l| l.last()).collect();
    if heads.is_empty() {
        return Merged {
            note: None,
            vv: VersionVector::new(),
            winner_vv: VersionVector::new(),
            winner_device: String::new(),
            conflict: false,
        };
    }

    let mut merged_vv = VersionVector::new();
    for h in &heads {
        merged_vv = join(&merged_vv, &h.vv);
    }

    let frontier: Vec<&NoteLogEntry> = heads
        .iter()
        .copied()
        .filter(|h| {
            !heads
                .iter()
                .any(|g| !std::ptr::eq(*g, *h) && dominates(&g.vv, &h.vv) && g.vv != h.vv)
        })
        .collect();

    let conflict = frontier.len() > 1;

    let winner = frontier
        .iter()
        .copied()
        .max_by(|a, b| {
            a.timestamp
                .cmp(&b.timestamp)
                .then_with(|| a.device_id.cmp(&b.device_id))
        })
        .expect("frontier is non-empty when heads is non-empty");

    let note = match &winner.op {
        NoteOp::Upsert(note) => Some(note.clone()),
        NoteOp::Tombstone { deleted_at } => {
            let latest_upsert = logs
                .iter()
                .flatten()
                .filter_map(|e| match &e.op {
                    NoteOp::Upsert(n) => Some((e.timestamp, n)),
                    NoteOp::Tombstone { .. } => None,
                })
                .max_by_key(|(ts, _)| *ts)
                .map(|(_, n)| n.clone());
            latest_upsert.map(|mut n| {
                n.deleted_at = Some(*deleted_at);
                n.updated_at = *deleted_at;
                n
            })
        }
    };

    Merged {
        note,
        vv: merged_vv,
        winner_vv: winner.vv.clone(),
        winner_device: winner.device_id.clone(),
        conflict,
    }
}

// md:fn compact_own_log
pub fn compact_own_log(log: &[NoteLogEntry]) -> Vec<NoteLogEntry> {
    if log.len() <= 1 {
        return log.to_vec();
    }
    let head = log.last().expect("len > 1");
    let newest_upsert = log
        .iter()
        .filter(|e| matches!(e.op, NoteOp::Upsert(_)))
        .max_by(|a, b| {
            a.timestamp
                .cmp(&b.timestamp)
                .then_with(|| a.device_id.cmp(&b.device_id))
        });
    match newest_upsert {
        None => vec![head.clone()],
        Some(u) if std::ptr::eq(u, head) => vec![head.clone()],
        Some(u) => vec![u.clone(), head.clone()],
    }
}

// md:Winner
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Winner {
    Local,
    Incoming,
}

// md:fn resolve
pub fn resolve(
    local_vv: &VersionVector,
    local_ts: DateTime<Utc>,
    local_device: &str,
    incoming_vv: &VersionVector,
    incoming_ts: DateTime<Utc>,
    incoming_device: &str,
) -> Winner {
    let incoming_dominates = dominates(incoming_vv, local_vv);
    let local_dominates = dominates(local_vv, incoming_vv);
    match (incoming_dominates, local_dominates) {
        (true, false) => Winner::Incoming,
        (_, true) => Winner::Local,
        (false, false) => {
            if (incoming_ts, incoming_device) > (local_ts, local_device) {
                Winner::Incoming
            } else {
                Winner::Local
            }
        }
    }
}

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Note;

    // md:mod tests > fn vv
    fn vv(pairs: &[(&str, u64)]) -> VersionVector {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    // md:mod tests > fn ts
    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(secs, 0).unwrap()
    }

    // md:mod tests > fn resolve_incoming_causally_newer_wins
    #[test]
    fn resolve_incoming_causally_newer_wins() {
        let w = resolve(
            &vv(&[("A", 1)]),
            ts(10),
            "A",
            &vv(&[("A", 1), ("B", 1)]),
            ts(5),
            "B",
        );
        assert_eq!(w, Winner::Incoming);
    }

    // md:mod tests > fn resolve_stale_incoming_loses
    #[test]
    fn resolve_stale_incoming_loses() {
        let w = resolve(
            &vv(&[("A", 1), ("B", 1)]),
            ts(5),
            "B",
            &vv(&[("A", 1)]),
            ts(10),
            "A",
        );
        assert_eq!(w, Winner::Local);
    }

    // md:mod tests > fn resolve_equal_vectors_is_noop
    #[test]
    fn resolve_equal_vectors_is_noop() {
        let w = resolve(&vv(&[("A", 2)]), ts(10), "A", &vv(&[("A", 2)]), ts(99), "A");
        assert_eq!(w, Winner::Local);
    }

    // md:mod tests > fn resolve_concurrent_equal_timestamp_converges_by_device
    #[test]
    fn resolve_concurrent_equal_timestamp_converges_by_device() {
        let local_a = vv(&[("A", 1)]);
        let incoming_b = vv(&[("B", 1)]);
        assert_eq!(
            resolve(&local_a, ts(10), "A", &incoming_b, ts(10), "B"),
            Winner::Incoming
        );
        assert_eq!(
            resolve(&incoming_b, ts(10), "B", &local_a, ts(10), "A"),
            Winner::Local
        );
    }

    // md:mod tests > fn resolve_concurrent_breaks_by_timestamp
    #[test]
    fn resolve_concurrent_breaks_by_timestamp() {
        let w = resolve(&vv(&[("A", 1)]), ts(10), "A", &vv(&[("B", 1)]), ts(30), "B");
        assert_eq!(w, Winner::Incoming);
    }

    // md:mod tests > fn entry
    fn entry(vv: &[(&str, u64)], dev: &str, secs: i64, op: NoteOp) -> NoteLogEntry {
        let vv = vv
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect::<VersionVector>();
        NoteLogEntry {
            vv,
            timestamp: DateTime::<Utc>::from_timestamp(secs, 0).unwrap(),
            device_id: dev.to_string(),
            op,
        }
    }

    // md:mod tests > fn note
    fn note(body: &str) -> Note {
        Note::new("t", body)
    }

    // md:mod tests > fn single_device_history_picks_latest
    #[test]
    fn single_device_history_picks_latest() {
        let logs = vec![vec![
            entry(&[("A", 1)], "A", 10, NoteOp::Upsert(note("v1"))),
            entry(&[("A", 2)], "A", 20, NoteOp::Upsert(note("v2"))),
        ]];
        let m = merge(&logs);
        assert!(!m.conflict);
        assert_eq!(m.note.unwrap().body, "v2");
        assert_eq!(m.vv.get("A"), Some(&2));
    }

    // md:mod tests > fn merge_exposes_winning_heads_own_vv_and_device
    #[test]
    fn merge_exposes_winning_heads_own_vv_and_device() {
        let logs = vec![
            vec![entry(&[("A", 1)], "A", 10, NoteOp::Upsert(note("from A")))],
            vec![
                entry(
                    &[("A", 1), ("B", 1)],
                    "B",
                    20,
                    NoteOp::Upsert(note("from B")),
                ),
                entry(
                    &[("A", 1), ("B", 2)],
                    "B",
                    30,
                    NoteOp::Tombstone {
                        deleted_at: DateTime::<Utc>::from_timestamp(30, 0).unwrap(),
                    },
                ),
            ],
        ];
        let m = merge(&logs);
        assert_eq!(m.winner_device, "B");
        assert_eq!(m.winner_vv, vv(&[("A", 1), ("B", 2)]));
        assert!(m.note.unwrap().deleted_at.is_some());
    }

    // md:mod tests > fn merge_empty_has_empty_winner_fields
    #[test]
    fn merge_empty_has_empty_winner_fields() {
        let m = merge(&[]);
        assert!(m.winner_vv.is_empty());
        assert!(m.winner_device.is_empty());
    }

    // md:mod tests > fn causal_update_wins_without_conflict
    #[test]
    fn causal_update_wins_without_conflict() {
        let logs = vec![
            vec![entry(&[("A", 1)], "A", 10, NoteOp::Upsert(note("from A")))],
            vec![entry(
                &[("A", 1), ("B", 1)],
                "B",
                20,
                NoteOp::Upsert(note("from B")),
            )],
        ];
        let m = merge(&logs);
        assert!(!m.conflict);
        assert_eq!(m.note.unwrap().body, "from B");
        assert_eq!(m.vv.get("A"), Some(&1));
        assert_eq!(m.vv.get("B"), Some(&1));
    }

    // md:mod tests > fn concurrent_edits_conflict_and_break_by_timestamp
    #[test]
    fn concurrent_edits_conflict_and_break_by_timestamp() {
        let logs = vec![
            vec![entry(&[("A", 1)], "A", 10, NoteOp::Upsert(note("from A")))],
            vec![entry(&[("B", 1)], "B", 30, NoteOp::Upsert(note("from B")))],
        ];
        let m = merge(&logs);
        assert!(m.conflict);
        assert_eq!(m.note.unwrap().body, "from B");
        assert_eq!(m.vv.get("A"), Some(&1));
        assert_eq!(m.vv.get("B"), Some(&1));
    }

    // md:mod tests > fn tombstone_wins_over_concurrent_older_edit
    #[test]
    fn tombstone_wins_over_concurrent_older_edit() {
        let logs = vec![
            vec![
                entry(&[("A", 1)], "A", 10, NoteOp::Upsert(note("orig"))),
                entry(
                    &[("A", 2)],
                    "A",
                    40,
                    NoteOp::Tombstone {
                        deleted_at: DateTime::<Utc>::from_timestamp(40, 0).unwrap(),
                    },
                ),
            ],
            vec![entry(
                &[("B", 1)],
                "B",
                20,
                NoteOp::Upsert(note("concurrent")),
            )],
        ];
        let m = merge(&logs);
        assert!(m.conflict, "delete vs concurrent edit is a real conflict");
        let n = m.note.unwrap();
        assert!(n.deleted_at.is_some(), "tombstone wins by later timestamp");
    }

    // md:mod tests > fn compact_own_log_preserves_merge
    #[test]
    fn compact_own_log_preserves_merge() {
        let mut long = Vec::new();
        for i in 1..=10u64 {
            long.push(entry(
                &[("A", i)],
                "A",
                i as i64 * 10,
                NoteOp::Upsert(note(&format!("v{i}"))),
            ));
        }
        let c = compact_own_log(&long);
        assert_eq!(c.len(), 1, "upsert-headed history compacts to the head");
        assert_eq!(
            merge(&[c]).note.unwrap().body,
            merge(&[long]).note.unwrap().body
        );

        let del_ts = DateTime::<Utc>::from_timestamp(200, 0).unwrap();
        let mut with_delete = Vec::new();
        for i in 1..=5u64 {
            with_delete.push(entry(
                &[("A", i)],
                "A",
                i as i64 * 10,
                NoteOp::Upsert(note(&format!("body{i}"))),
            ));
        }
        with_delete.push(entry(
            &[("A", 6)],
            "A",
            200,
            NoteOp::Tombstone { deleted_at: del_ts },
        ));
        let c = compact_own_log(&with_delete);
        assert_eq!(
            c.len(),
            2,
            "tombstone-headed history keeps upsert + tombstone"
        );
        let m_orig = merge(std::slice::from_ref(&with_delete));
        let m_comp = merge(std::slice::from_ref(&c));
        let n_orig = m_orig.note.unwrap();
        let n_comp = m_comp.note.unwrap();
        assert_eq!(n_comp.body, n_orig.body, "recovered content is unchanged");
        assert!(n_comp.deleted_at.is_some());
        assert_eq!(m_comp.vv, m_orig.vv, "merged vector is unchanged");

        let peer = vec![entry(&[("B", 1)], "B", 15, NoteOp::Upsert(note("peer")))];
        let full = merge(&[with_delete, peer.clone()]);
        let comp = merge(&[c, peer]);
        assert_eq!(comp.vv, full.vv);
        assert_eq!(
            comp.note.map(|n| (n.body, n.deleted_at.is_some())),
            full.note.map(|n| (n.body, n.deleted_at.is_some())),
        );
    }

    // md:mod tests > fn causal_edit_after_delete_resurrects
    #[test]
    fn causal_edit_after_delete_resurrects() {
        let logs = vec![
            vec![
                entry(&[("A", 1)], "A", 10, NoteOp::Upsert(note("orig"))),
                entry(
                    &[("A", 2)],
                    "A",
                    20,
                    NoteOp::Tombstone {
                        deleted_at: DateTime::<Utc>::from_timestamp(20, 0).unwrap(),
                    },
                ),
            ],
            vec![entry(
                &[("A", 2), ("B", 1)],
                "B",
                30,
                NoteOp::Upsert(note("revived")),
            )],
        ];
        let m = merge(&logs);
        assert!(!m.conflict);
        let n = m.note.unwrap();
        assert!(n.deleted_at.is_none());
        assert_eq!(n.body, "revived");
    }
}
