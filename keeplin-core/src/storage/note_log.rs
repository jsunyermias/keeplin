//! Version-vector conflict resolution for the filesystem note model.
//!
//! Each note in [`crate::storage::fs::FsBackend`] keeps **one append-only operation log
//! per device** (`notes/{id}/log.{device_id}.msgpack`). Because every log file has a
//! single writer, an external file-synchronisation tool such as Syncthing replicates them
//! without ever producing conflict copies. The current state of a note is then the
//! *merge* of all per-device logs, decided by comparing **version vectors** — exactly the
//! "resolve conflicts by comparing each note's logs" model requested.
//!
//! This module is intentionally pure (no I/O): it defines the on-disk log types and the
//! [`merge`] function, so the conflict-resolution logic can be unit-tested in isolation.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::Note;

/// A version vector: a per-device monotonic counter map (`device_id -> counter`).
///
/// A missing key is treated as `0`. One vector *dominates* another when it is at least as
/// large in every component — meaning it causally descends from (has seen) the other.
pub type VersionVector = BTreeMap<String, u64>;

/// Increments `device`'s component of `vv` by one (creating it at `1` if absent).
pub fn increment(vv: &mut VersionVector, device: &str) {
    *vv.entry(device.to_string()).or_insert(0) += 1;
}

/// Returns `true` when `a` dominates `b`: `a[k] >= b[k]` for every key `k` of `b`.
///
/// Domination is reflexive (a vector dominates an equal one). Two vectors are
/// *concurrent* when neither dominates the other.
pub fn dominates(a: &VersionVector, b: &VersionVector) -> bool {
    b.iter()
        .all(|(k, &bv)| a.get(k).copied().unwrap_or(0) >= bv)
}

/// Returns the element-wise maximum (least upper bound) of two version vectors.
pub fn join(a: &VersionVector, b: &VersionVector) -> VersionVector {
    let mut out = a.clone();
    for (k, &bv) in b {
        let slot = out.entry(k.clone()).or_insert(0);
        *slot = (*slot).max(bv);
    }
    out
}

/// The operation a log entry records: an upsert carrying the full note content, or a
/// tombstone carrying the deletion time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NoteOp {
    /// Create-or-update with the complete note (body included).
    Upsert(Note),
    /// Soft delete; `deleted_at` is the deletion timestamp.
    Tombstone { deleted_at: DateTime<Utc> },
}

/// One entry in a per-device note log.
///
/// `vv` is the writer's known version vector *after* incrementing its own component, so
/// comparing the latest entry of each device's log reconstructs the causal relationships
/// between edits made on different devices.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteLogEntry {
    /// The version vector at the time this entry was written (own component already bumped).
    pub vv: VersionVector,
    /// Wall-clock time of the edit; used only to break ties between truly concurrent edits.
    pub timestamp: DateTime<Utc>,
    /// The device that wrote this entry.
    pub device_id: String,
    /// What happened.
    pub op: NoteOp,
}

/// The outcome of merging every per-device log of a single note.
#[derive(Debug, Clone)]
pub struct Merged {
    /// The winning note. `deleted_at` is `Some` when the winning op is a tombstone.
    /// `None` means the note has no entries at all (its directory should be ignored).
    pub note: Option<Note>,
    /// The merged version vector (join of every device's latest entry), to be stored as
    /// the new frontier and used as the base for the next local edit.
    pub vv: VersionVector,
    /// `true` when the merge had to break a real concurrent-edit conflict by timestamp.
    pub conflict: bool,
}

/// Merge all per-device logs of one note into its current state.
///
/// `logs` is one `Vec<NoteLogEntry>` per device (order between devices does not matter).
/// The algorithm:
/// 1. Take each device's **latest** entry (its log is append-only, so the last element).
/// 2. Compute the **frontier**: heads not dominated by any *other* head. A single
///    frontier element means one edit causally descends from all the others — the clean
///    case. Several frontier elements means a true concurrent conflict.
/// 3. The winner is the sole frontier element, or — on a conflict — the frontier element
///    with the greatest `(timestamp, device_id)` (a deterministic, last-write-wins
///    tiebreak that every device computes identically).
/// 4. The merged version vector is the join of every head.
///
/// For a `Tombstone` winner the returned note carries the most recent known content
/// fields with `deleted_at`/`updated_at` set to the tombstone time, so callers can both
/// hide it from listings and still resolve a later concurrent edit against it.
pub fn merge(logs: &[Vec<NoteLogEntry>]) -> Merged {
    // Heads: the latest entry of each non-empty device log.
    let heads: Vec<&NoteLogEntry> = logs.iter().filter_map(|l| l.last()).collect();
    if heads.is_empty() {
        return Merged {
            note: None,
            vv: VersionVector::new(),
            conflict: false,
        };
    }

    // Merged frontier vector = join of all heads.
    let mut merged_vv = VersionVector::new();
    for h in &heads {
        merged_vv = join(&merged_vv, &h.vv);
    }

    // Frontier: heads not strictly dominated by a different head.
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

    // Winner: the single frontier head, or the max (timestamp, device_id) on a conflict.
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
            // Recover the note's last known fields from the most recent Upsert anywhere,
            // then stamp it deleted at the tombstone time.
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
        conflict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Note;

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

    fn note(body: &str) -> Note {
        Note::new("t", body)
    }

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

    #[test]
    fn causal_update_wins_without_conflict() {
        // B edited after seeing A's edit (vv {A:1,B:1}) → B dominates A → B wins, no conflict.
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

    #[test]
    fn concurrent_edits_conflict_and_break_by_timestamp() {
        // Neither head dominates → conflict → later timestamp wins.
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

    #[test]
    fn tombstone_wins_over_concurrent_older_edit() {
        // A deletes (vv {A:2}, later), B edits concurrently (vv {B:1}, earlier) → delete wins.
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

    #[test]
    fn causal_edit_after_delete_resurrects() {
        // B's edit causally follows A's delete (knows {A:2}) → B dominates → resurrection.
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
