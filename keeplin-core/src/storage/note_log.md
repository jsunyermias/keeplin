# `note_log.rs`

## Purpose

Pure (I/O-free) conflict-resolution logic for the filesystem note model. Each note in
`FsBackend` keeps one append-only operation log per device
(`notes/{id}/log.{device_id}.msgpack`); because every log has a single writer, Syncthing
replicates them without ever producing conflict copies. A note's current state is the
**merge** of all its per-device logs, decided by comparing **version vectors**. Keeping
this logic here — separate from the filesystem I/O in `fs.rs` — lets the merge be
unit-tested in isolation.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `VersionVector` | type alias | `BTreeMap<String, u64>`: per-device counter map (`device_id -> counter`); a missing key is `0` |
| `NoteOp` | enum | `Upsert(Note)` (full content) or `Tombstone { deleted_at }` |
| `NoteLogEntry` | struct | One log entry: `{ vv, timestamp, device_id, op }` |
| `Merged` | struct | Result of a merge: `{ note: Option<Note>, vv, conflict: bool }` |

## Public API

### `fn increment(vv: &mut VersionVector, device: &str)`
Increments `device`'s component by one (creating it at `1` if absent). Called before each
local edit so the entry records a strictly newer vector.

### `fn dominates(a: &VersionVector, b: &VersionVector) -> bool`
`true` when `a[k] >= b[k]` for every key of `b` — i.e. `a` causally descends from (has
seen) `b`. Two vectors are *concurrent* when neither dominates the other.

### `fn join(a, b) -> VersionVector`
Element-wise maximum (least upper bound), used to compute the merged frontier.

### `fn merge(logs: &[Vec<NoteLogEntry>]) -> Merged`
Merge all of a note's per-device logs:
1. Take each device's **latest** entry (the log is append-only, so its last element).
2. Compute the **frontier**: heads not dominated by any other head. One frontier element
   means one edit causally follows all the others (clean case); several means a true
   concurrent conflict.
3. The winner is the sole frontier element, or — on a conflict — the frontier element with
   the greatest `(timestamp, device_id)`. This tiebreak is deterministic, so every device
   computes the same winner and the note **converges**.
4. The merged `vv` is the join of all heads.

For a `Tombstone` winner, the returned note carries the most recent known content fields
with `deleted_at`/`updated_at` set to the tombstone time, so the note is both hidden from
listings and still comparable against a later concurrent edit.

### `fn compact_own_log(log: &[NoteLogEntry]) -> Vec<NoteLogEntry>`

Bounds a device's **own** per-device log without changing what `merge` returns. Within one
single-writer log every entry's version vector dominates all earlier ones (each local write
increments this device's component over everything seen so far), so the last entry is the log's
frontier and alone determines this device's contribution to `merge`'s heads and merged vector.
The only other entry `merge` can consult from a log is the newest `Upsert` (used to recover a
`Tombstone` winner's content fields), so compaction keeps **at most two** entries: the head, plus
the highest-`(timestamp, device_id)` `Upsert` when that is not already the head. `FsBackend`
calls this from `append_note_op` once a log passes `NOTE_LOG_COMPACT_THRESHOLD` entries. It is
sound **only** for a device's own log — a foreign or multi-writer log is not totally ordered by
domination, so compacting it could drop entries `merge` still needs.

### `fn resolve(local: (vv, ts, device), incoming: (vv, ts, device)) -> Winner`

The **state-based** (current-value) analogue of `merge`, for backends that keep only the
current state instead of a full op log — used by `DbBackend::apply_change` (via `incoming_wins`)
for every entity type, and by `FsBackend::apply_change` (via `sidecar_incoming_wins`) for
notebooks and tags. Returns `Winner::Incoming` iff the incoming vector strictly dominates
local's; `Winner::Local` iff local dominates (including equal vectors, so re-applying is a
no-op); otherwise (concurrent) the greater `(timestamp, device_id)` wins — the **same** frontier
tiebreak `merge` uses. So `FsBackend`'s log merge and `DbBackend`'s state resolve compute the
identical winner, and the two backends converge deterministically. This replaced `DbBackend`'s
old bare-`updated_at` last-write-wins, which diverged permanently on equal timestamps.

## Design notes

- **Convergence** is the central CRDT property: independent of message order or which
  device runs the merge, all devices reach the same state.
- A tombstone is a versioned op, so a stale edit cannot resurrect a delete, while a causal
  edit made *after* seeing the delete legitimately revives the note.
- The module is intentionally free of filesystem and async code; `fs.rs` reads/writes the
  log files and calls `merge`.

## Related files

- `keeplin-core/src/storage/fs.rs` — reads the per-device logs, calls `merge`, and writes
  the `note.md` / `meta.msgpack` projection.
- `keeplin-core/tests/fs_backend.rs` — two-device causal and concurrent-convergence tests.
- `SECURITY.md` — "Conflict resolution differs by backend".
