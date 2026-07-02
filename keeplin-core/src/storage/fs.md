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
├── logs/{device_id}.log                    ← this device's global NDJSON journal (optional epoch header line)
├── .keeplin/device_id                      ← stable identifier for this installation
├── .keeplin/format_version                 ← storage format version (currently 5)
├── .keeplin/offsets/{device_id}            ← "{epoch}:{offset}" cursor consumed in each foreign global log
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
`logs/{device_id}.log`. `receive_changes` reads new foreign entries via the `{epoch}:{offset}`
cursors in `.keeplin/offsets/`, so each entry is processed exactly once (an epoch change means the
foreign log was compacted, so its snapshot is re-read from the start — see the compaction design
note). Each line is decoded by `log_entry_to_change`; unrecognised `(entity_type, operation)`
pairs are skipped (forward/backward compatibility), as is the optional `EpochHeader` first line. A
`delete` line carries the tombstone's own `(deleted_at, vv, last_writer)`, so replayed deletes
compete through `note_log::resolve` on the receiver.

### Backward compatibility

- `entity_type` defaults to `"note"` (v1 logs had no such field).
- `entity_id` accepts the old `"note_id"` field name via a serde alias.
- Both old (`"create"`) and new (`"note_create"`) operation strings are accepted.

### Format migrations (`ensure_format_version`, `FORMAT_VERSION = 5`)

`FsBackend::new` calls `ensure_format_version(fresh)`, a versioned ladder mirroring
`DbBackend`'s `PRAGMA user_version` runner:

- A **brand-new** store (`fresh` — the device-id file did not exist, so `.keeplin/device_id`
  was just created) is stamped directly at `FORMAT_VERSION` and runs no migration step; there
  is no prior data to transform, which matters once a future step does real work.
- An existing store runs each outstanding step (`apply_format_migration`) in order, stamping
  `.keeplin/format_version` **after each one**, so a crash mid-ladder resumes from the last
  completed step. An existing store with no stamp file is treated as format `1`.
- A stamp **newer** than this build is refused (`StorageError::InvalidState`) rather than
  opened, matching `DbBackend`'s downgrade guard.

Every historical step (v1→v5) is a no-op that only advances the stamp: the format changes so
far — the `LogEntry` serde aliases/defaults, versioned associations, resource tombstones, and
the optional global-log epoch header — are all parse-compatible with older files. A future
breaking change gets a real body in `apply_format_migration`, guaranteed to run exactly once,
in order, on the stores that need it.

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
- The **global NDJSON journal is compacted too**, by generation-epoch snapshots. Peers track
  their read position by byte offset, so entries cannot simply be dropped. Instead, once this
  device's own log passes `GLOBAL_LOG_COMPACT_THRESHOLD` entries, `append_log` rewrites it as a
  current-state snapshot — one entry per notebook/tag/resource/association (a `create`/`add`, or a
  `delete`/`remove` tombstone when soft-deleted) — behind a bumped `EpochHeader` first line. A
  peer whose cursor is from an older epoch re-reads the snapshot from the start (`read_new_entries`
  compares the header epoch to the stored `{epoch}:{offset}` cursor); because every entry is
  version-vector resolved and idempotent, replay converges rather than duplicating or resurrecting
  state. This bounds the log by entity count, not mutation count. `prune_change_journal` stays a
  no-op — compaction, not time-based deletion, does the bounding.
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
- `SECURITY.md` — "Conflict resolution is unified on version vectors".
