# `storage/note_log.rs` — version-vector conflict resolution

Self-contained companion for `keeplin-core/src/storage/note_log.rs`. It documents
**every code block of the source file, in source order** — a reader with only this
file must be able to understand it without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::Note;
```

**What it does** — Version-vector conflict resolution for the filesystem note
model — and the **single resolution procedure** the whole project shares. Each
note in `FsBackend` keeps one append-only operation log **per device**
(`notes/{id}/log.{device_id}.msgpack`); because every log file has a single
writer, Syncthing replicates them without conflict copies, and a note's current
state is the *merge* of all per-device logs decided by comparing version vectors.
The module is intentionally **pure** (no I/O): the on-disk log types plus `merge`
(log-based), `resolve` (the state-based analogue for `DbBackend`), and
`compact_own_log`, all unit-tested in isolation.

**Dependencies** — `std::collections::BTreeMap`, `chrono`, `serde`,
`crate::models::Note`.

**Used by** — `storage/fs.rs` (log merge + append), `storage/db.rs` (`resolve`
on every applied change), `models.rs` (every entity carries a `VersionVector`),
`collab/protocol.rs`/`state.rs` (line versioning), keeplin-srv (pins this crate
and calls `resolve` for its rows), backend integration tests.

**Repeated context** — Determinism is the contract: every device given the same
inputs must pick the same winner. `merge`, `resolve`, and keeplin-srv's line
model must never fork their semantics — the domination test and the
`(timestamp, device_id)` last-write-wins tiebreak are shared verbatim.

---

## VersionVector

**Identification** — `pub type VersionVector = BTreeMap<String, u64>;` marker
`// md:VersionVector`.

**Code** — complete and verbatim:

```rust
// md:VersionVector
pub type VersionVector = BTreeMap<String, u64>;
```

**What it does** — A version vector: per-device monotonic counters
(`device_id → counter`). A missing key is `0`. One vector *dominates* another
when it is at least as large in every component — it causally descends from
(has seen) the other. `BTreeMap` keeps serialisation deterministic (sorted
keys).

**Dependencies** — `BTreeMap`.

**Used by** — everywhere: entity fields (`models.rs`), collab lines, both
backends, keeplin-srv.

**Repeated context** — the vv actor is the **device** id, never the user id.

---

## fn increment

**Identification** — `pub fn increment(vv: &mut VersionVector, device: &str)`;
marker `// md:fn increment`.

**Code** — complete and verbatim:

```rust
// md:fn increment
pub fn increment(vv: &mut VersionVector, device: &str) {
    *vv.entry(device.to_string()).or_insert(0) += 1;
}
```

**What it does** — Bumps `device`'s component by one (creating it at 1) — the
"I am about to write" step that makes a local edit dominate the state it was
based on.

**Dependencies** — none.

**Used by** — `FsBackend::append_note_op`, `DbBackend` write paths;
`collab/state.rs` has its own equivalent (`bump`).

**Repeated context** — none.

---

## fn dominates

**Identification** — `pub fn dominates(a: &VersionVector, b: &VersionVector) -> bool`;
marker `// md:fn dominates`.

**Code** — complete and verbatim:

```rust
// md:fn dominates
pub fn dominates(a: &VersionVector, b: &VersionVector) -> bool {
    b.iter()
        .all(|(k, &bv)| a.get(k).copied().unwrap_or(0) >= bv)
}
```

**What it does** — `true` when `a[k] >= b[k]` for every key of `b`. Reflexive
(equal vectors dominate each other); two vectors where neither dominates are
*concurrent*.

**Dependencies** — none.

**Used by** — `merge` (frontier computation), `resolve`.

**Repeated context** — none.

---

## fn join

**Identification** — `pub fn join(a: &VersionVector, b: &VersionVector) -> VersionVector`;
marker `// md:fn join`.

**Code** — complete and verbatim:

```rust
// md:fn join
pub fn join(a: &VersionVector, b: &VersionVector) -> VersionVector {
    let mut out = a.clone();
    for (k, &bv) in b {
        let slot = out.entry(k.clone()).or_insert(0);
        *slot = (*slot).max(bv);
    }
    out
}
```

**What it does** — Element-wise maximum (least upper bound) of two vectors —
"the union of everything both sides have seen".

**Dependencies** — none.

**Used by** — `merge` (the merged frontier vector); backends when recording a
new frontier.

**Repeated context** — none.

---

## NoteOp

**Identification** — enum deriving `Debug, Clone, Serialize, Deserialize,
PartialEq, Eq` with `#[allow(clippy::large_enum_variant)]`; marker
`// md:NoteOp`.

**Code** — complete and verbatim:

```rust
// md:NoteOp
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum NoteOp {
    Upsert(Note),
    Tombstone { deleted_at: DateTime<Utc> },
}
```

**What it does** — What a log entry records: `Upsert(Note)` (create-or-update
with the complete note, body included) or `Tombstone { deleted_at }` (soft
delete). `Upsert` intentionally carries the full `Note` inline — it *is* the
serialised op-log payload — so the size disparity with `Tombstone` is by
design; boxing would add a per-entry heap allocation for no benefit, hence the
clippy allow.

**Dependencies** — `Note`, `chrono`, `serde`.

**Used by** — `NoteLogEntry.op`; `FsBackend` log writes; `merge`;
`compact_own_log`.

**Repeated context** — full-snapshot ops are what make journal-derived history
and tombstone-content recovery possible.

---

## NoteLogEntry

**Identification** — struct deriving `Debug, Clone, Serialize, Deserialize,
PartialEq, Eq`; marker `// md:NoteLogEntry`.

**Code** — complete and verbatim:

```rust
// md:NoteLogEntry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteLogEntry {
    pub vv: VersionVector,
    pub timestamp: DateTime<Utc>,
    pub device_id: String,
    pub op: NoteOp,
}
```

**What it does** — One entry in a per-device note log: `vv` (the writer's known
vector *after* incrementing its own component — comparing each device's latest
entry reconstructs causal relationships), `timestamp` (wall clock; used
**only** to break ties between truly concurrent edits), `device_id` (the
writer), `op`.

**Dependencies** — `VersionVector`, `NoteOp`, `chrono`.

**Used by** — `FsBackend` (MessagePack-encoded log records), `merge`,
`compact_own_log`.

**Repeated context** — none.

---

## Merged

**Identification** — struct deriving `Debug, Clone`; marker `// md:Merged`.

**Code** — complete and verbatim:

```rust
// md:Merged
#[derive(Debug, Clone)]
pub struct Merged {
    pub note: Option<Note>,
    pub vv: VersionVector,
    pub winner_vv: VersionVector,
    pub winner_device: String,
    pub conflict: bool,
}
```

**What it does** — The outcome of merging every per-device log of one note:
`note: Option<Note>` (the winner; `deleted_at: Some` when a tombstone won;
`None` when there are no entries at all — ignore the directory), `vv` (join of
every device's latest entry — the new frontier and the base for the next local
edit), `winner_vv` (the **winning head's own** vector — what a state-based
backend must carry on the emitted `Change` so its `resolve` sees the same
causal position this merge did, *not* the join which folds in concurrent
heads), `winner_device` (the `last_writer` for the emitted `Change`, so a
peer's tiebreak breaks identically; empty with no entries), `conflict` (`true`
when a real concurrent conflict was broken by timestamp).

**Dependencies** — `Note`, `VersionVector`.

**Used by** — `FsBackend::read_note_logs`/materialisation and its
emitted-change plumbing.

**Repeated context** — the `winner_vv` ≠ joined `vv` distinction matters most
for delete winners: emitting the join would make the delete look causally ahead
of edits it never saw.

---

## fn merge

**Identification** — `pub fn merge(logs: &[Vec<NoteLogEntry>]) -> Merged`;
marker `// md:fn merge`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Merges all per-device logs of one note into its current
state. `logs` is one `Vec<NoteLogEntry>` per device (inter-device order
irrelevant). Algorithm:

1. **Heads** — each device's latest entry (append-only log ⇒ the last element);
   no heads → empty `Merged`.
2. **Frontier** — heads not strictly dominated by any *other* head. One element
   = the clean causal case; several = a true concurrent conflict.
3. **Winner** — the sole frontier element, or on a conflict the frontier
   element with the greatest `(timestamp, device_id)` — the deterministic LWW
   tiebreak every device computes identically.
4. **Merged vector** — the join of every head.

For a `Tombstone` winner, the returned note recovers the most recent known
content fields (the highest-timestamp `Upsert` anywhere in the logs) with
`deleted_at`/`updated_at` stamped to the tombstone time, so callers can hide it
from listings *and* still resolve a later concurrent edit against it.

**Dependencies** — `dominates`, `join`, `NoteLogEntry`, `Merged`.

**Used by** — `FsBackend` on every note read/materialisation; the unit tests
here; indirectly the whole FS sync model.

**Repeated context** — equal-vector heads both stay in the frontier (exclusion
requires strict domination), so identical replicated entries do not eliminate
each other.

---

## fn compact_own_log

**Identification** — `pub fn compact_own_log(log: &[NoteLogEntry]) -> Vec<NoteLogEntry>`;
marker `// md:fn compact_own_log`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Compacts one device's **own** append-only log without
changing `merge`'s result. Within a single device's log every entry's vector
dominates all earlier ones (each local write bases itself on everything seen so
far, then increments its own component — see `FsBackend::append_note_op`), so
the last entry alone determines this device's head; the only other entry
`merge` can consult is the newest `Upsert` (tombstone-content recovery). Hence
compaction keeps at most two entries: the head, plus the
highest-`(timestamp, device_id)` `Upsert` when that isn't already the head.
**Sound only for a device's own single-writer log**
(`log.{own_device}.msgpack`) — a foreign or multi-writer log's entries are not
totally ordered by domination, and compacting one would drop entries `merge`
still needs.

**Dependencies** — `NoteOp`, `NoteLogEntry`.

**Used by** — `FsBackend` when a note's own log passes the compaction
threshold (256 entries).

**Repeated context** — compaction is why FS note history is bounded (~256
versions per note before collapse to the frontier).

---

## Winner

**Identification** — enum deriving `Debug, Clone, Copy, PartialEq, Eq`; marker
`// md:Winner`.

**Code** — complete and verbatim:

```rust
// md:Winner
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Winner {
    Local,
    Incoming,
}
```

**What it does** — Outcome of a pairwise comparison: `Local` (keep the local
value — incoming is stale, equal, or loses the tiebreak) or `Incoming`
(replace — causally newer or wins the concurrent tiebreak).

**Dependencies** — none.

**Used by** — returned by `resolve`; matched in `storage/db.rs`,
`storage/fs.rs` sidecar handling, and keeplin-srv.

**Repeated context** — none.

---

## fn resolve

**Identification** —
`pub fn resolve(local_vv, local_ts, local_device, incoming_vv, incoming_ts, incoming_device) -> Winner`;
marker `// md:fn resolve`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Decides whether an `incoming` versioned write replaces the
`local` one for a single entity — the **state-based analogue of `merge`** for
backends keeping only current state (`DbBackend`, keeplin-srv). Rules (matching
`merge`'s frontier + tiebreak exactly, so every backend converges regardless of
arrival order):

- `Incoming` iff its vector **strictly dominates** local's (it causally saw
  the local write and moved past it);
- `Local` iff its vector dominates incoming's — **including equal vectors**,
  so re-applying a change is an idempotent no-op;
- otherwise the writes are **concurrent**: the greater
  `(timestamp, device_id)` wins — deterministic LWW that avoids the permanent
  divergence a bare `updated_at` comparison suffers when two edits share a
  timestamp.

**Dependencies** — `dominates`, `Winner`.

**Used by** — `storage/db.rs::apply_change` for every entity kind;
`storage/fs.rs` sidecar entities; keeplin-srv's store (same function via the
pinned crate); backend integration tests.

**Repeated context** — this is why `apply_change` idempotency holds everywhere:
equal vectors → `Local` → no-op.

---

## mod tests

**Identification** — `#[cfg(test)]` unit-test module; marker `// md:mod tests`.
Four helpers + twelve tests, all pure.

**Code** — container: members documented as sub-blocks below: fn vv, fn ts, fn resolve_incoming_causally_newer_wins, fn resolve_stale_incoming_loses, fn resolve_equal_vectors_is_noop, fn resolve_concurrent_equal_timestamp_converges_by_device, fn resolve_concurrent_breaks_by_timestamp, fn entry, fn note, fn single_device_history_picks_latest, fn merge_exposes_winning_heads_own_vv_and_device, fn merge_empty_has_empty_winner_fields, fn causal_update_wins_without_conflict, fn concurrent_edits_conflict_and_break_by_timestamp, fn tombstone_wins_over_concurrent_older_edit, fn compact_own_log_preserves_merge, fn causal_edit_after_delete_resurrects.

**What it does** — Pins the resolution semantics: the `resolve` regimes,
`merge`'s clean/conflict/tombstone/resurrection behaviours, winner-field
exposure, and `compact_own_log`'s equivalence.

**Dependencies** — `super::*`, `models::Note`.

**Used by** — CI.

**Repeated context** — these tests are the executable contract shared with
keeplin-srv; changing them means changing distributed convergence semantics.

The explicit `imports` leaf below preserves the test-module dependency preamble
verbatim.

### imports

**Identification** — test-module dependencies; marker `// md:mod tests > imports`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > imports
    use super::*;
    use crate::models::Note;
```

**What it does** — Brings the parent module API and test-only dependencies into
scope.

### fn vv

**Identification** — helper `fn vv(pairs: &[(&str, u64)]) -> VersionVector`;
marker `// md:mod tests > fn vv`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn vv
    fn vv(pairs: &[(&str, u64)]) -> VersionVector {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }
```

**What it does** — Builds a vector from literal pairs.

### fn ts

**Identification** — helper `fn ts(secs: i64) -> DateTime<Utc>`; marker
`// md:mod tests > fn ts`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn ts
    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(secs, 0).unwrap()
    }
```

**What it does** — Second-resolution timestamp constructor.

### fn resolve_incoming_causally_newer_wins

**Identification** — unit test; marker
`// md:mod tests > fn resolve_incoming_causally_newer_wins`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — incoming `{A:1,B:1}` dominates local `{A:1}` → `Incoming`,
even with an older timestamp (causality beats wall clock).

### fn resolve_stale_incoming_loses

**Identification** — unit test; marker
`// md:mod tests > fn resolve_stale_incoming_loses`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — the mirror case: a dominated incoming loses despite a newer
timestamp.

### fn resolve_equal_vectors_is_noop

**Identification** — unit test; marker
`// md:mod tests > fn resolve_equal_vectors_is_noop`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn resolve_equal_vectors_is_noop
    #[test]
    fn resolve_equal_vectors_is_noop() {
        let w = resolve(&vv(&[("A", 2)]), ts(10), "A", &vv(&[("A", 2)]), ts(99), "A");
        assert_eq!(w, Winner::Local);
    }
```

**What it does** — equal vectors → `Local` (idempotent re-apply), regardless of
timestamps.

### fn resolve_concurrent_equal_timestamp_converges_by_device

**Identification** — unit test; marker
`// md:mod tests > fn resolve_concurrent_equal_timestamp_converges_by_device`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — the case bare-`updated_at` LWW gets wrong: two concurrent
edits with identical timestamps. Runs `resolve` from both devices'
perspectives and asserts both pick the **same** winner (the greater device id).

### fn resolve_concurrent_breaks_by_timestamp

**Identification** — unit test; marker
`// md:mod tests > fn resolve_concurrent_breaks_by_timestamp`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn resolve_concurrent_breaks_by_timestamp
    #[test]
    fn resolve_concurrent_breaks_by_timestamp() {
        let w = resolve(&vv(&[("A", 1)]), ts(10), "A", &vv(&[("B", 1)]), ts(30), "B");
        assert_eq!(w, Winner::Incoming);
    }
```

**What it does** — concurrent vectors, different timestamps → the later one
wins.

### fn entry

**Identification** — helper building a `NoteLogEntry`; marker
`// md:mod tests > fn entry`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Constructs an entry from vv pairs, device, seconds, and op.

### fn note

**Identification** — helper `fn note(body: &str) -> Note`; marker
`// md:mod tests > fn note`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn note
    fn note(body: &str) -> Note {
        Note::new("t", body)
    }
```

**What it does** — `Note::new("t", body)`.

### fn single_device_history_picks_latest

**Identification** — unit test; marker
`// md:mod tests > fn single_device_history_picks_latest`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — one device, two upserts → latest body wins, no conflict,
merged vv `{A:2}`.

### fn merge_exposes_winning_heads_own_vv_and_device

**Identification** — unit test; marker
`// md:mod tests > fn merge_exposes_winning_heads_own_vv_and_device`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — a causal chain ending in B's tombstone: `winner_device` is
`B`, `winner_vv` is the delete head's own `{A:1,B:2}` (not a join), and the
merged note is deleted.

### fn merge_empty_has_empty_winner_fields

**Identification** — unit test; marker
`// md:mod tests > fn merge_empty_has_empty_winner_fields`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn merge_empty_has_empty_winner_fields
    #[test]
    fn merge_empty_has_empty_winner_fields() {
        let m = merge(&[]);
        assert!(m.winner_vv.is_empty());
        assert!(m.winner_device.is_empty());
    }
```

**What it does** — `merge(&[])` yields empty `winner_vv`/`winner_device`.

### fn causal_update_wins_without_conflict

**Identification** — unit test; marker
`// md:mod tests > fn causal_update_wins_without_conflict`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — B edited after seeing A's edit (`{A:1,B:1}`) → B wins
cleanly, joined vv `{A:1,B:1}`.

### fn concurrent_edits_conflict_and_break_by_timestamp

**Identification** — unit test; marker
`// md:mod tests > fn concurrent_edits_conflict_and_break_by_timestamp`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `{A:1}` vs `{B:1}` (neither dominates) → `conflict = true`,
later timestamp wins, vv joins both.

### fn tombstone_wins_over_concurrent_older_edit

**Identification** — unit test; marker
`// md:mod tests > fn tombstone_wins_over_concurrent_older_edit`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A's later delete vs B's earlier concurrent edit → the
delete wins the tiebreak and the merged note is deleted (content recovered
from the newest upsert).

### fn compact_own_log_preserves_merge

**Identification** — unit test; marker
`// md:mod tests > fn compact_own_log_preserves_merge`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Three cases: a 10-entry upsert history compacts to just the
head with an identical merge; a history ending in a tombstone keeps exactly
`[newest upsert, tombstone head]` and still recovers content; the compacted
log merged against a concurrent peer log yields the same note and vector as
the full log.

### fn causal_edit_after_delete_resurrects

**Identification** — unit test; marker
`// md:mod tests > fn causal_edit_after_delete_resurrects`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — B's edit causally follows A's delete (knows `{A:2}`) → B
dominates → the note revives with `deleted_at: None`, no conflict.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `NoteOp` — defined here (EXTRACTED; 2 cross-file edge(s))
- `vv()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `NoteLogEntry` — defined here (EXTRACTED; 1 cross-file edge(s))
- `Merged` — defined here (EXTRACTED; 1 cross-file edge(s))
- `increment()` — defined here (EXTRACTED; file-local)
- `dominates()` — defined here (EXTRACTED; file-local)
- `join()` — defined here (EXTRACTED; file-local)
- `merge()` — defined here (EXTRACTED; file-local)
- `compact_own_log()` — defined here (EXTRACTED; file-local)
- `Winner` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: imports_from×1, references×2; e.g. `Note`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: references×2; e.g. `.read_note_logs()`, `.append_note_op()`)
- `keeplin-core/tests/db_backend.rs` — DbBackend integration tests (EXTRACTED: calls×1; e.g. `delete_for_unknown_entity_leaves_a_tombstone_blocking_a_stale_create()`)
- `keeplin-core/tests/fs_backend.rs` — FsBackend integration tests (EXTRACTED: calls×1; e.g. `delete_for_unknown_sidecar_entity_leaves_a_tombstone_blocking_a_stale_create()`)
- `keeplin-core/src/storage/db.rs`, `keeplin-core/src/collab/*`, keeplin-srv — via `resolve`/`VersionVector` (INFERRED: fully-qualified paths the AST pass does not link)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `VersionVector` | `// md:VersionVector` |
| 3 | `fn increment` | `// md:fn increment` |
| 4 | `fn dominates` | `// md:fn dominates` |
| 5 | `fn join` | `// md:fn join` |
| 6 | `NoteOp` | `// md:NoteOp` |
| 7 | `NoteLogEntry` | `// md:NoteLogEntry` |
| 8 | `Merged` | `// md:Merged` |
| 9 | `fn merge` | `// md:fn merge` |
| 10 | `fn compact_own_log` | `// md:fn compact_own_log` |
| 11 | `Winner` | `// md:Winner` |
| 12 | `fn resolve` | `// md:fn resolve` |
| 13 | `mod tests` (container) | `// md:mod tests` |
| 14 | `imports` | `// md:mod tests > imports` |
| 15 | `fn vv` | `// md:mod tests > fn vv` |
| 16 | `fn ts` | `// md:mod tests > fn ts` |
| 17 | `fn resolve_incoming_causally_newer_wins` | `// md:mod tests > fn resolve_incoming_causally_newer_wins` |
| 18 | `fn resolve_stale_incoming_loses` | `// md:mod tests > fn resolve_stale_incoming_loses` |
| 19 | `fn resolve_equal_vectors_is_noop` | `// md:mod tests > fn resolve_equal_vectors_is_noop` |
| 20 | `fn resolve_concurrent_equal_timestamp_converges_by_device` | `// md:mod tests > fn resolve_concurrent_equal_timestamp_converges_by_device` |
| 21 | `fn resolve_concurrent_breaks_by_timestamp` | `// md:mod tests > fn resolve_concurrent_breaks_by_timestamp` |
| 22 | `fn entry` | `// md:mod tests > fn entry` |
| 23 | `fn note` | `// md:mod tests > fn note` |
| 24 | `fn single_device_history_picks_latest` | `// md:mod tests > fn single_device_history_picks_latest` |
| 25 | `fn merge_exposes_winning_heads_own_vv_and_device` | `// md:mod tests > fn merge_exposes_winning_heads_own_vv_and_device` |
| 26 | `fn merge_empty_has_empty_winner_fields` | `// md:mod tests > fn merge_empty_has_empty_winner_fields` |
| 27 | `fn causal_update_wins_without_conflict` | `// md:mod tests > fn causal_update_wins_without_conflict` |
| 28 | `fn concurrent_edits_conflict_and_break_by_timestamp` | `// md:mod tests > fn concurrent_edits_conflict_and_break_by_timestamp` |
| 29 | `fn tombstone_wins_over_concurrent_older_edit` | `// md:mod tests > fn tombstone_wins_over_concurrent_older_edit` |
| 30 | `fn compact_own_log_preserves_merge` | `// md:mod tests > fn compact_own_log_preserves_merge` |
| 31 | `fn causal_edit_after_delete_resurrects` | `// md:mod tests > fn causal_edit_after_delete_resurrects` |
