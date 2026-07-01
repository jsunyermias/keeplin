# `storage/fs.rs` — FsBackend (filesystem storage)

## Purpose

`FsBackend` stores all Keeplin data as files on the local filesystem and lets an external
file-replication tool (typically **Syncthing**) carry those files between devices. There is
no sync server: replication is "copy the files"; conflict resolution happens locally when a
device reads the merged state. Every log file has a **single writer**, so Syncthing never
produces conflict copies.

There are two storage models under one root:

1. **Notes** — per-device append-only operation logs, merged by **version vector**
   (a small CRDT). This is the model that makes concurrent offline edits converge.
2. **Notebooks, tags, resources** — one MessagePack sidecar per entity, with every mutation
   also appended to a global per-device NDJSON journal used by sync.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `FsBackend` | struct | Filesystem-backed `StorageBackend` implementation |
| `NoteMeta` | struct (private) | The `meta.msgpack` projection: the merged note (body blanked) + merged version vector |
| `LogEntry` | struct (private) | One line in the global NDJSON journal (notebooks/tags/resources/note-tags) |
| `SyncState` | struct (private) | Holds the `last_sync` timestamp, persisted to `.keeplin/sync_state.json` |

Note-log types (`NoteOp`, `NoteLogEntry`, `VersionVector`, `merge`) live in the sibling
`note_log.rs` — see `note_log.md`.

## Directory layout

```
{root}/
├── notes/{uuid}/note.md                    ← materialized body (ciphertext when encrypted)
├── notes/{uuid}/meta.msgpack               ← materialized metadata + merged VV (a cache)
├── notes/{uuid}/log.{device_id}.msgpack    ← that device's append-only note-op log (truth)
├── notebooks/{uuid}.msgpack                ← notebook sidecar
├── tags/{uuid}.msgpack                     ← tag sidecar
├── note_tags/{note_uuid}/{tag_uuid}        ← versioned association state (msgpack: vv + deleted_at)
├── resources/{uuid}/meta.msgpack           ← resource metadata
├── resources/{uuid}/data                   ← resource binary payload
├── logs/{device_id}.log                    ← this device's global NDJSON journal
├── .keeplin/device_id                      ← stable identifier for this installation
├── .keeplin/format_version                 ← storage format version (currently 4)
├── .keeplin/offsets/{device_id}            ← byte offset consumed in each foreign global log
└── .keeplin/sync_state.json                ← last successful sync timestamp
```

## The note model — per-device logs + version-vector merge

Each note is a **directory** of logs, one per device that has ever edited it. A write:

1. takes `note_write_lock` (see Concurrency), then reads **all** of the note's on-disk logs;
2. `merge`s them (via `note_log::merge`) to learn the current merged version vector;
3. `increment`s this device's component and appends a new `NoteOp::Upsert`/`Tombstone`
   entry to **this device's** log only;
4. re-materializes `note.md` + `meta.msgpack` from the merged result as a local projection.

A **read** (`read_note` / `list_notes`) re-merges the logs live and returns the winner; it is
**non-mutating** — it does not rewrite `note.md`/`meta.msgpack` (an earlier version did, which
made reads look like writes to sync-change detection). The `.md`/`.msgpack` files are therefore
a cache, never the source of truth.

Convergence (all devices agree regardless of replication order) comes from `note_log::merge`:
a causal edit applies cleanly; a genuinely concurrent edit is resolved by a deterministic
last-write-wins tiebreak. See `note_log.md` for the merge rules.

## Global journal & format — `LogEntry`, `SyncState`

Notebook/tag/resource/note-tag mutations append a `LogEntry` (NDJSON) to
`logs/{device_id}.log`. `get_changes_since` reads new foreign entries via the byte-offset
cursors in `.keeplin/offsets/`, so each entry is processed exactly once, and also picks up
this device's own new lines. Each line is decoded by `log_entry_to_change`; unrecognised
`(entity_type, operation)` pairs are skipped (forward/backward compatibility). A `delete`
line's own timestamp becomes the tombstone time, so replayed deletes compete in
last-write-wins on the receiver.

### Backward compatibility

- `entity_type` defaults to `"note"` (v1 logs had no such field).
- `entity_id` accepts the old `"note_id"` field name via a serde alias.
- Both old (`"create"`) and new (`"note_create"`) operation strings are accepted.

`FORMAT_VERSION = 4`. `ensure_format_version()` reads `.keeplin/format_version` on startup;
older stamps are logged and re-stamped. Migrations to date need no data transformation (serde
aliases/defaults handle old files at parse time); the stamp is always (re)written so brand-new
and un-stamped stores are marked immediately.

## Concurrency — `note_write_lock`

`FsBackend` holds one `Arc<Mutex<()>>` (`note_write_lock`) taken across the whole
read-logs → append → materialize sequence of a note write. Without it, two concurrent writes
to the same note read the same log and the second atomic rename overwrites the first, silently
dropping an entry — which the single-writer-per-log version-vector model forbids. One **global**
mutex (not per-note) keeps it simple; note writes are infrequent in offline FS use, so the
reduced write parallelism is fine. **Reads take no lock** — the atomic temp-then-rename gives
them a consistent view of every log file.

## Atomic write pattern

All file writes use temp-then-rename so a reader always sees the old or the new file, never a
half-written one:

```rust
tokio::fs::write(&tmp, bytes).await?;
tokio::fs::rename(&tmp, &final_path).await?;
```

## `apply_change`

Applies all 13 `Change` variants: notes go through the version-vector log (an incoming
`NoteCreate`/`NoteUpdate` is merged, not blindly overwritten); **notebooks/tags are
version-vector resolved** — `sidecar_incoming_wins` runs `note_log::resolve` over the stored
sidecar's `(vv, updated_at, last_writer)` and the incoming write, so concurrent edits converge
deterministically (a delete carries its own `vv` in the global-log entry via
`fs_tombstone_value`, so it competes like an edit); **`NoteTagAdd`/`Remove` are also
version-vector resolved** — the association file at `note_tags/{note}/{tag}` now holds a
versioned `NoteTagState` (msgpack: `updated_at`/`deleted_at`/`vv`/`last_writer`), so a
concurrent add-vs-remove converges (an add is the present state, a remove a tombstone kept so
it can beat a concurrent add); `ResourceCreate` writes the metadata sidecar and, if
`data: Some(bytes)`, the payload too (used when receiving from `DbBackend`; Syncthing handles
the normal case); **`ResourceDelete` is version-vector resolved too** — `resource_incoming_wins`
runs `note_log::resolve` over the resource sidecar's `(vv, effective_ts, last_writer)`, and a
winning delete soft-deletes the metadata (`deleted_at` set, blob retained) rather than removing
the resource dir, so a concurrent delete-vs-recreate converges.

Local notebook/tag/resource writes stamp the sidecar's `vv`/`last_writer` (`next_sidecar_vv` /
`next_resource_vv` load the current vector and increment this device's component), and note↔tag
adds/removes stamp the association file's `vv`/`last_writer` the same way — matching notes and
`DbBackend`. An old empty marker file (pre-version) is read as a present association with an empty
vector, so existing stores keep working. `list_resources` skips soft-deleted sidecars and
`read_resource` returns `NotFound` for one.

## Design notes

- `send_changes` / `receive_changes` push/pull is a no-op: replication is entirely
  Syncthing's job. The sync engine still calls them; they return immediately.
- **Per-note logs are compacted automatically.** Each `log.{device}.msgpack` has a single
  writer, so its last entry dominates all earlier ones; once a log passes
  `NOTE_LOG_COMPACT_THRESHOLD` entries, `append_note_op` collapses it to its frontier (the head
  plus the newest `Upsert` needed to recover a tombstone winner's content) via
  `note_log::compact_own_log`. This is lossless — `merge` yields the same result — and bounds
  each per-note per-device log regardless of how many times the note is edited.
- The **global NDJSON journal** is still append-only and **not yet pruned** by the backend:
  peers track their read position by byte offset, so blindly dropping entries would corrupt that
  cursor. It grows with entity *churn* (not entity count); snapshot-based pruning of these logs
  is a scheduled follow-up. `prune_change_journal` remains a no-op until then.
- FsBackend targets single-user, low-concurrency desktop use; cross-*process* writes to the
  same store are still unsupported (the lock is per-process).
- FsBackend targets single-user, low-concurrency desktop use; cross-*process* writes to the
  same store are still unsupported (the lock is per-process).

## Related files

- `keeplin-core/src/storage/note_log.rs` — the pure version-vector merge this backend calls.
- `keeplin-core/src/storage/backend.rs` — the trait (and default `note_backlinks` scan) it implements.
- `keeplin-core/src/models.rs` — all types stored by this backend.
- `keeplin-core/tests/fs_backend.rs` — including the concurrent-write regression and
  two-device convergence tests.
- `SECURITY.md` — "Conflict resolution differs by backend".
