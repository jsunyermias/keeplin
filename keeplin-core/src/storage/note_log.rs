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
///
/// `Upsert` intentionally carries the complete [`Note`] inline — it *is* the op-log
/// payload that is serialised to disk and merged on read — so the size disparity with the
/// tiny `Tombstone` variant is by design. Boxing would add a heap allocation per entry for
/// no benefit, so `large_enum_variant` is allowed here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
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

/// Compact one device's **own** append-only log without changing the result of [`merge`].
///
/// Within a single device's log every entry's version vector dominates all earlier ones: each
/// local write bases its vector on the merge of all state seen so far and then increments this
/// device's own component (see `FsBackend::append_note_op`). The last entry is therefore the
/// log's frontier and alone determines this device's contribution to `merge`'s heads and merged
/// vector. The only other entry `merge` can consult from a log is the newest `Upsert` — used to
/// recover a tombstone winner's content fields (see the `Tombstone` arm of [`merge`]) — so
/// compaction keeps at most two entries: the head, plus the highest-`(timestamp, device_id)`
/// `Upsert` when that is not already the head. `merge` over the compacted log (in any device
/// combination) yields an identical note and merged vector.
///
/// This is sound **only** for a device's own single-writer log (`log.{own_device}.msgpack`);
/// applying it to a foreign or multi-writer log, whose entries are not totally ordered by
/// domination, would drop entries `merge` still needs.
pub fn compact_own_log(log: &[NoteLogEntry]) -> Vec<NoteLogEntry> {
    if log.len() <= 1 {
        return log.to_vec();
    }
    let head = log.last().expect("len > 1");
    // The highest-(timestamp, device_id) Upsert anywhere in the log — exactly what merge's
    // tombstone-content recovery selects with its `max_by_key(timestamp)` over all upserts.
    let newest_upsert = log
        .iter()
        .filter(|e| matches!(e.op, NoteOp::Upsert(_)))
        .max_by(|a, b| {
            a.timestamp
                .cmp(&b.timestamp)
                .then_with(|| a.device_id.cmp(&b.device_id))
        });
    match newest_upsert {
        // No upserts (a bare tombstone) or the head already is the newest upsert → head suffices.
        None => vec![head.clone()],
        Some(u) if std::ptr::eq(u, head) => vec![head.clone()],
        // Keep the newest upsert (for tombstone recovery) followed by the frontier head.
        Some(u) => vec![u.clone(), head.clone()],
    }
}

/// The outcome of a pairwise version-vector comparison: which side should win.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Winner {
    /// Keep the local value; the incoming write is stale, equal, or loses the tiebreak.
    Local,
    /// Replace with the incoming value; it is causally newer or wins the concurrent tiebreak.
    Incoming,
}

/// Decide whether an `incoming` versioned write should replace the `local` one for a single
/// entity — the state-based (current-value) analogue of [`merge`], for backends that keep only
/// the current state (e.g. `DbBackend`) rather than a full per-device op log.
///
/// The rules match `merge`'s frontier + tiebreak exactly, so every backend converges to the
/// same state regardless of the order changes arrive in:
/// - `incoming` wins iff its vector **strictly dominates** local's (it has causally seen the
///   local write and moved past it);
/// - `local` wins iff its vector dominates incoming's — including the case where the vectors
///   are **equal**, so re-applying a change is an idempotent no-op;
/// - otherwise the two writes are **concurrent**, and the winner is the one with the greater
///   `(timestamp, device_id)` — a deterministic last-write-wins tiebreak that avoids the
///   permanent divergence a bare `updated_at` comparison suffers when two edits share a
///   timestamp.
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
        // Incoming has seen strictly more than local → it is causally newer.
        (true, false) => Winner::Incoming,
        // Local dominates (incoming is stale) or the vectors are equal (idempotent no-op).
        (_, true) => Winner::Local,
        // Concurrent: neither dominates → deterministic (timestamp, device) tiebreak.
        (false, false) => {
            if (incoming_ts, incoming_device) > (local_ts, local_device) {
                Winner::Incoming
            } else {
                Winner::Local
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Note;

    fn vv(pairs: &[(&str, u64)]) -> VersionVector {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(secs, 0).unwrap()
    }

    #[test]
    fn resolve_incoming_causally_newer_wins() {
        // incoming {A:1,B:1} dominates local {A:1} → incoming has seen local and moved past.
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

    #[test]
    fn resolve_equal_vectors_is_noop() {
        // Same vector on both sides → idempotent re-apply → keep local.
        let w = resolve(&vv(&[("A", 2)]), ts(10), "A", &vv(&[("A", 2)]), ts(99), "A");
        assert_eq!(w, Winner::Local);
    }

    #[test]
    fn resolve_concurrent_equal_timestamp_converges_by_device() {
        // The case bare-`updated_at` LWW gets wrong: two concurrent edits, identical timestamp.
        // Both devices must pick the SAME winner. Device B's write has the greater device id.
        let local_a = vv(&[("A", 1)]);
        let incoming_b = vv(&[("B", 1)]);
        // On device A: local=A's edit, incoming=B's edit.
        assert_eq!(
            resolve(&local_a, ts(10), "A", &incoming_b, ts(10), "B"),
            Winner::Incoming
        );
        // On device B: local=B's edit, incoming=A's edit → keeps B. Same converged winner (B).
        assert_eq!(
            resolve(&incoming_b, ts(10), "B", &local_a, ts(10), "A"),
            Winner::Local
        );
    }

    #[test]
    fn resolve_concurrent_breaks_by_timestamp() {
        let w = resolve(&vv(&[("A", 1)]), ts(10), "A", &vv(&[("B", 1)]), ts(30), "B");
        assert_eq!(w, Winner::Incoming);
    }

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

    /// Compaction must not change what `merge` yields, for every shape of a device's own log:
    /// a long upsert history, and a history ending in a tombstone (whose content is recovered
    /// from the newest upsert). It must also bound the log to at most two entries.
    #[test]
    fn compact_own_log_preserves_merge() {
        // Case 1: a long single-device upsert history → compacts to just the head.
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

        // Case 2: history ending in a tombstone → keeps the newest upsert + the tombstone head,
        // so the tombstone winner still recovers its content fields.
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

        // Case 3: the compacted log still merges correctly against a *concurrent* peer log —
        // the newest upsert survives so a winning peer tombstone can recover content from it.
        let peer = vec![entry(&[("B", 1)], "B", 15, NoteOp::Upsert(note("peer")))];
        let full = merge(&[with_delete, peer.clone()]);
        let comp = merge(&[c, peer]);
        assert_eq!(comp.vv, full.vv);
        assert_eq!(
            comp.note.map(|n| (n.body, n.deleted_at.is_some())),
            full.note.map(|n| (n.body, n.deleted_at.is_some())),
        );
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
