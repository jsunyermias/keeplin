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
