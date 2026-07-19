# `storage/fs.rs` — FsBackend (filesystem storage)

Self-contained companion for `keeplin-core/src/storage/fs.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must
be able to understand it without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

```rust
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;
// … async_trait, chrono, serde, tokio io/sync, uuid; crate error/model types;
// super::{backend::DEFAULT_HISTORY_LIMIT, note_log::{resolve, NoteLogEntry, NoteOp,
//         VersionVector, Winner}, EntityVersion, the repository traits,
//         NotebookSortProfile, SortableRfc3339}
```

**What it does** — The filesystem `StorageBackend`: files under a user-chosen root
that Syncthing (or any equivalent) replicates between devices. Two storage models:

- **Notes — per-device logs with version-vector merge.** Each note is a directory
  `notes/{id}/` holding `note.md` (materialised body; ciphertext under
  encryption), `meta.msgpack` (materialised metadata + merged vector — a cache),
  and `log.{device_id}.msgpack` — an append-only op log written **only** by that
  device. Single-writer logs never conflict under Syncthing; a note's true state
  is the merge of all its logs (`note_log::merge`). Projections are regenerated on
  every write and sync; **reads materialise live from the logs** and never write.
- **Notebooks, tags, resources — sidecars + global change log.** One MessagePack
  sidecar per entity, every mutation appended as an NDJSON line to
  `logs/{device_id}.log`; `receive_changes` reads new foreign entries via a
  byte-offset cursor.

Log growth is bounded: per-note logs compact to their frontier past
`NOTE_LOG_COMPACT_THRESHOLD` (256) entries (`note_log::compact_own_log`, lossless
for `merge`); the global journal compacts to a **current-state snapshot** behind a
bumped generation-epoch header once this device's own log passes
`GLOBAL_LOG_COMPACT_THRESHOLD` (512) entries — a peer notices the epoch change and
re-reads the snapshot from the start; every entry is version-vector resolved and
idempotent, so replaying converges. `prune_change_journal` stays a no-op —
compaction, not time-based deletion, does the bounding.

**Dependencies** — `tokio::fs`/io, `rmp_serde` (MessagePack), `serde_json`
(NDJSON), `note_log`, the trait family, `SortableRfc3339`.

**Used by** — `keeplin-daemon/src/main.rs` (`storage = "filesystem"` mode, the
default), `migrate.rs`, in-crate tests of many modules (the cheapest real
backend), `tests/fs_backend.rs`.

**Repeated context** — soft-delete-always, idempotent `apply_change`, and the
`(timestamp, device_id)` tiebreak, exactly as in `DbBackend` — same decisions,
different storage shape.

---

## NoteMeta

**Identification** — private serde struct; marker `// md:NoteMeta`.

**What it does** — The materialised projection written to
`notes/{id}/meta.msgpack`: the merged note (body blanked — it lives in `note.md`)
plus the merged version vector. A local cache regenerated from the logs; never
the source of truth for resolution.

**Used by** — `persist_note_projection`, `read_note_projection`, `note_vv`.

---

## NoteMetaEntry

**Identification** — private struct; marker `// md:NoteMetaEntry`.

**What it does** — One **live** note's listing/ordering metadata for the
in-memory index: `notebook_id`, `created_at`, `effective_sort_key` (the `0`
sentinel already mapped), `is_starred`. Deliberately tiny — no title or body —
so the index is bounded by note count, not corpus size.

**Used by** — `NoteMetaIndex`.

---

## impl NoteMetaEntry

**Identification** — inherent impl; marker `// md:impl NoteMetaEntry`. One
method.

### fn from_note

**Identification** — marker `// md:impl NoteMetaEntry > fn from_note`.

**What it does** — Projects a `Note` to its entry.

---

## NoteMetaIndex

**Identification** — private `#[derive(Debug, Default)]` struct; marker
`// md:NoteMetaIndex`.

**What it does** — In-memory `note_id → NoteMetaEntry` map of every live note,
so `list_notes` / `list_notes_in_notebook` / `list_starred_notes` /
`notebook_sort_profile` select, order and paginate without re-merging every
note's logs per call. Built lazily on the first listing (from the cheap
projections, full merge only for notes with none), then maintained
incrementally through `persist_note_projection`. **Freshness**: listings
reflect the last *materialised* state — updated on every local write and every
sync cycle; a Syncthing-replicated peer edit appears after the next cycle,
matching `DbBackend` (whose rows also change only on `apply_change`).
Single-note `read_note` stays a live merge, so reading a specific note is
always current.

**Used by** — the listing methods via `with_note_index`.

---

## impl NoteMetaIndex

**Identification** — inherent impl; marker `// md:impl NoteMetaIndex`. One
method.

### fn apply

**Identification** — marker `// md:impl NoteMetaIndex > fn apply`.

**What it does** — Reflects a note's current state: live → (re-)insert;
tombstoned → drop (listings exclude soft-deleted notes).

---

## NoteTagState

**Identification** — private serde struct; marker `// md:NoteTagState`.

**What it does** — The versioned state of one note↔tag association, stored as
the MessagePack contents of `note_tags/{note}/{tag}` (previously an empty
marker file): `updated_at`, `deleted_at` (`None` = attached, `Some` = tombstone
kept so a remove can beat a concurrent add), `vv`, `last_writer` — all
`serde(default)` so old records parse.

**Used by** — the association helpers and `add/remove_note_tag`,
`apply_change`.

---

## LogEntry

**Identification** — private serde struct; marker `// md:LogEntry`.

**What it does** — One line of a per-device NDJSON global log: `timestamp`,
`entity_type` (defaults to `"note"` — v1 logs had no field), `entity_id`
(alias `"note_id"` for v1), `operation`, `data`. Plain-text lines that Syncthing
replicates.

**Used by** — `append_log`, the readers, `log_entry_to_change`, the snapshot
builders.

---

## fn default_entity_type

**Identification** — marker `// md:fn default_entity_type`.

**What it does** — `"note"` — the v1 serde default.

---

## EpochHeader

**Identification** — private serde struct; marker `// md:EpochHeader`.

**What it does** — The first line of a compacted global log: a
`{"__keeplin_epoch__": n}` generation marker. The epoch increments on every
compaction so a byte-offset reader can notice the rewrite and restart.

**Used by** — `compact_global_log_locked`, `parse_epoch_header`,
`read_log_header`.

---

## fn parse_epoch_header

**Identification** — marker `// md:fn parse_epoch_header`.

**What it does** — Parses a line as an `EpochHeader`, `None` for a normal
`LogEntry` line (which lacks the field).

**Used by** — every log reader (to skip/detect headers).

---

## fn fs_tombstone_value

**Identification** — marker `// md:fn fs_tombstone_value`.

**What it does** — The global-log `data` payload for a delete:
`{deleted_at, vv, last_writer}` so `log_entry_to_change` reconstructs a delete
`Change` carrying everything `resolve` needs on the receiving device.

**Used by** — the delete paths and `snapshot_entry_from_value`.

---

## fn fs_assoc_value

**Identification** — marker `// md:fn fs_assoc_value`.

**What it does** — The `data` payload for a note↔tag add/remove:
`{tag_id, updated_at, vv, last_writer}`.

**Used by** — `add/remove_note_tag`, the snapshot builder.

---

## fn snapshot_entry_from_sidecar

**Identification** — generic fn; marker `// md:fn snapshot_entry_from_sidecar`.

**What it does** — Builds a snapshot `LogEntry` for a notebook/tag/resource by
decoding its MessagePack sidecar into the concrete type and re-serialising
through `serde_json` — the same encoding `append_log` uses, so the entry
round-trips through `log_entry_to_change` identically. `None` when the sidecar
cannot be decoded.

**Used by** — `build_global_snapshot`.

---

## fn snapshot_entry_from_value

**Identification** — marker `// md:fn snapshot_entry_from_value`.

**What it does** — A snapshot entry from an entity's JSON value: a live entity
becomes a `create` carrying the full record; a soft-deleted one becomes a
`delete` tombstone carrying `(deleted_at, vv, last_writer)` — exactly the
shapes `log_entry_to_change` reconstructs.

**Used by** — `snapshot_entry_from_sidecar`.

---

## fn fs_assoc_from_data

**Identification** — marker `// md:fn fs_assoc_from_data`.

**What it does** — Reconstructs `(updated_at, vv, last_writer)` from a
global-log `data` value, falling back to the entry timestamp and empty
vector/writer for pre-version records.

**Used by** — `log_entry_to_change`.

---

## fn fs_tombstone_from_data

**Identification** — marker `// md:fn fs_tombstone_from_data`.

**What it does** — Reconstructs `(deleted_at, vv, last_writer)`, same
fallbacks (v1 records stored `{ "id": … }`).

**Used by** — `log_entry_to_change`.

---

## fn log_entry_to_change

**Identification** — `fn log_entry_to_change(entry: LogEntry) -> Option<Change>`;
marker `// md:fn log_entry_to_change`.

**What it does** — Converts one log line into a typed `Change`. `None` for
unrecognised `(entity_type, operation)` pairs (corruption, or a newer build's
rows) — callers log and skip. v1 compatibility: `"note"` accepts both
`"create"` and `"note_create"` style operations. Note deletes parse their
tombstone's vv/writer from the data when present (v1 records fall back to an
empty vector + entry timestamp) so a replayed delete keeps its causal metadata
instead of an empty vector a peer would treat as stale (issue #70). Resource
entries carry metadata only, `data: None` — Syncthing replicates
`resources/{id}/data` independently.

**Used by** — `get_changes_since`, `receive_changes`.

---

## fn atomic_write

**Identification** — `async fn atomic_write(path: &Path, bytes: &[u8])`; marker
`// md:fn atomic_write`.

**What it does** — Write a sibling `{path}.tmp`, **fsync**, then rename over
the destination: a reader never observes a half-written file; a failed write
leaves the previous contents intact; the fsync closes the power-loss window in
which the rename persists but the data does not. On failure the temp file is
best-effort removed (a crash can still orphan one — see
`sweep_orphan_tmp_files`).

**Used by** — every sidecar/projection/cursor write.

---

## SyncState

**Identification** — private serde struct; marker `// md:SyncState`.

**What it does** — The contents of `.keeplin/sync_state.msgpack`: `last_sync`,
the watermark `get_changes_since` filters against.

**Used by** — `get_last_sync_time`, `update_sync_time`.

---

## FsBackend

**Identification** — `pub struct FsBackend`; marker `// md:FsBackend`.

**What it does** — The backend's state and the on-disk tree:

```text
{root}/
  notes/{uuid}/note.md                    — materialized body
  notes/{uuid}/meta.msgpack               — metadata + merged vv (cache)
  notes/{uuid}/log.{device_id}.msgpack    — that device's op log (source of truth)
  notebooks/{uuid}.msgpack                — sidecar
  tags/{uuid}.msgpack                     — sidecar
  note_tags/{note}/{tag}                  — versioned association state
  resources/{uuid}/meta.msgpack + data    — metadata + raw payload
  logs/{device_id}.log                    — global NDJSON log (optional epoch header)
  .keeplin/device_id | format_version | sync_state.msgpack | offsets/{device_id}
```

Fields: `root`; `device_id` (from `.keeplin/device_id`);
`note_write_lock: Mutex<()>` — serialises this device's note-log mutations
(read-modify-write + atomic rename: without it two concurrent writes to the
same note read the same log and the second rename silently drops an entry; the
vv model assumes a single writer per device log). One global mutex keeps it
simple; reads need no lock (the atomic rename gives a consistent view);
`global_log_lock: Mutex<()>` — serialises append + compaction of the global
log; `note_index: RwLock<Option<NoteMetaIndex>>` — the lazy listing index.

**Used by** — everything below.

---

## impl FsBackend

**Identification** — the first inherent impl; marker `// md:impl FsBackend`.
Constructor, sweeps, format versioning, path helpers, log machinery,
sidecar/association/resource versioning, the note merge pipeline. Contains the
constants `FORMAT_VERSION = 5`, `NOTE_LOG_COMPACT_THRESHOLD = 256`,
`GLOBAL_LOG_COMPACT_THRESHOLD = 512`, `GLOBAL_LOG_SOFT_BYTES = 64 KiB`
(documented with the methods that use them).

### fn new

**Identification** — `pub async fn new(root) -> Result<Self, StorageError>`;
marker `// md:impl FsBackend > fn new`.

**What it does** — Creates the directory tree, sweeps orphaned `*.tmp` files,
scans for Syncthing `*.sync-conflict-*` copies (reported at **error** level —
in a single-writer-per-file store they are the signature of a replicated
`.keeplin/` directory, i.e. two devices sharing one identity; nothing is
deleted), loads or creates the device id, and runs `ensure_format_version`
(`fresh` = the id was just created).

**Used by** — `main.rs::build_storage` (default mode); tests everywhere.

### fn sweep_orphan_tmp_files

**Identification** — marker `// md:impl FsBackend > fn sweep_orphan_tmp_files`.

**What it does** — Best-effort startup removal of `*.tmp` files orphaned by
interrupted atomic writes, across the flat dirs and one level down inside
`notes/`/`note_tags/`/`resources/`. Syncthing's own `.syncthing.*.tmp`
in-flight temporaries are explicitly left alone. Errors ignored — hygiene,
never a startup blocker.

### fn scan_sync_conflicts

**Identification** — marker `// md:impl FsBackend > fn scan_sync_conflicts`.

**What it does** — Read-only collection of every `*.sync-conflict-*` file in
the managed directories (and root). Nothing is deleted — the copies may hold
the only good version; the caller logs the findings with remediation guidance
(fix `.stignore`, reconcile manually).

### fn sweep_tmp_in_dir

**Identification** — marker `// md:impl FsBackend > fn sweep_tmp_in_dir`.

**What it does** — Non-recursive removal of orphaned `*.tmp` regular files in
one directory, skipping Syncthing temporaries.

### fn format_version_path

**Identification** — marker `// md:impl FsBackend > fn format_version_path`.

**What it does** — `.keeplin/format_version`.

### fn ensure_format_version

**Identification** — marker `// md:impl FsBackend > fn ensure_format_version`.

**What it does** — Brings the store up to `FORMAT_VERSION` (5), stamping after
**each** step so a crash mid-ladder resumes from the last completed step. A
`fresh` store is stamped directly (no migration over empty data). A missing
stamp on an existing store means format `1`; a stamp **newer** than this build
is refused (`InvalidState`) so a downgrade cannot run against a layout it does
not understand. A final stamp write covers the already-current case.

### fn apply_format_migration

**Identification** — marker
`// md:impl FsBackend > fn apply_format_migration`.

**What it does** — The per-version step. Every bump so far is
parse-compatible, so v2–v5 are no-ops that advance the stamp: v2 = `LogEntry`
serde aliases; v3/v4 = versioned associations + resource tombstones via
`serde(default)`; v5 = optional `EpochHeader` + `epoch:offset` cursors (a
pre-v5 log is epoch 0, a bare-integer cursor is `(0, offset)`). A future
breaking change gets a real body here.

### fn note_dir

**Identification** — marker `// md:impl FsBackend > fn note_dir`.

**What it does** — `{root}/notes/{id}`.

### fn note_md_path

**Identification** — marker `// md:impl FsBackend > fn note_md_path`.

**What it does** — `…/note.md` (human-readable when unencrypted).

### fn note_meta_path

**Identification** — marker `// md:impl FsBackend > fn note_meta_path`.

**What it does** — `…/meta.msgpack` (cache, not source of truth).

### fn note_log_path

**Identification** — marker `// md:impl FsBackend > fn note_log_path`.

**What it does** — `…/log.{device_id}.msgpack` (single-writer op log).

### fn device_log_path

**Identification** — marker `// md:impl FsBackend > fn device_log_path`.

**What it does** — `{root}/logs/{device_id}.log` (this device's global log).

### fn notebook_path

**Identification** — marker `// md:impl FsBackend > fn notebook_path`.

**What it does** — `{root}/notebooks/{id}.msgpack`.

### fn tag_path

**Identification** — marker `// md:impl FsBackend > fn tag_path`.

**What it does** — `{root}/tags/{id}.msgpack`.

### fn note_tag_dir

**Identification** — marker `// md:impl FsBackend > fn note_tag_dir`.

**What it does** — `{root}/note_tags/{note_id}`.

### fn note_tag_path

**Identification** — marker `// md:impl FsBackend > fn note_tag_path`.

**What it does** — `…/{tag_id}` — the association's versioned state file.

### fn resource_dir

**Identification** — marker `// md:impl FsBackend > fn resource_dir`.

**What it does** — `{root}/resources/{id}`.

### fn resource_meta_path

**Identification** — marker `// md:impl FsBackend > fn resource_meta_path`.

**What it does** — `…/meta.msgpack`.

### fn resource_data_path

**Identification** — marker `// md:impl FsBackend > fn resource_data_path`.

**What it does** — `…/data` (raw payload; `nonce ‖ ciphertext` under
encryption).

### fn read_or_create_device_id

**Identification** — marker
`// md:impl FsBackend > fn read_or_create_device_id`.

**What it does** — Reads `.keeplin/device_id`, or generates + persists a UUID
v4. Returns `(id, fresh)` — the file is the first thing written on init, so
its absence reliably means "never initialised" (used to stamp new stores at
the current format). The id names this device's log file and is the Argon2id
salt for `EncryptedBackend`; it must stay stable.

### fn append_log

**Identification** — marker `// md:impl FsBackend > fn append_log`.

**What it does** — Appends one `LogEntry` (one JSON line) to this device's
global log under `global_log_lock`, then may compact
(`maybe_compact_global_log_locked`). The lock ensures a concurrent append is
never lost to a compaction rewriting the file.

**Used by** — every notebook/tag/resource/association mutation.

### fn maybe_compact_global_log_locked

**Identification** — marker
`// md:impl FsBackend > fn maybe_compact_global_log_locked`.

**What it does** — Compacts when past the threshold; a cheap `metadata` size
gate (`GLOBAL_LOG_SOFT_BYTES`, 64 KiB) skips the line count entirely for small
logs. Caller must hold `global_log_lock`.

### fn own_log_entry_count

**Identification** — marker `// md:impl FsBackend > fn own_log_entry_count`.

**What it does** — Counts change entries (excluding the epoch header and
blanks) in this device's log.

### fn read_own_epoch

**Identification** — marker `// md:impl FsBackend > fn read_own_epoch`.

**What it does** — This device's log's generation epoch (0 = never
compacted).

### fn compact_global_log_locked

**Identification** — marker
`// md:impl FsBackend > fn compact_global_log_locked`.

**What it does** — Rewrites the global log as a current-state snapshot behind
a bumped epoch header (atomic write): notebooks/tags/resources/associations
each collapse to one entry (create/add or delete/remove tombstone), bounding
the log by entity count. Peers re-read the snapshot (epoch changed) and
converge because every entry is version-vector resolved and idempotent.
**Declines to run while any sidecar is unreadable** — the rewrite destroys
history, so an undecodable entity would silently vanish from the snapshot and
a lagging peer would never learn it existed; skipping is always safe (the
journal just keeps growing) and compaction resumes once the sidecar is
repaired.

### fn build_global_snapshot

**Identification** — marker `// md:impl FsBackend > fn build_global_snapshot`.

**What it does** — Builds the snapshot entries: notebooks + tags from their
sidecar directories, resources from their metadata sidecars (a missing meta is
a crashed-create orphan, skipped — not corruption), associations from their
state files. Notes are excluded — they sync through per-note logs. Returns
`(entries, unreadable)`; each unreadable sidecar is reported at error level
with its path and pauses compaction (see above).

### fn log_offset_path

**Identification** — marker `// md:impl FsBackend > fn log_offset_path`.

**What it does** — `.keeplin/offsets/{device_id}` — the `"{epoch}:{offset}"`
cursor for a foreign log (a bare integer is a pre-v5 cursor, read as epoch 0).

### fn read_log_offset

**Identification** — marker `// md:impl FsBackend > fn read_log_offset`.

**What it does** — Reads the cursor, `(0, 0)` when absent/unreadable.

### fn write_log_offset

**Identification** — marker `// md:impl FsBackend > fn write_log_offset`.

**What it does** — Atomic write of `"{epoch}:{offset}"`. A torn cursor reads
as `(0, 0)` → safe re-delivery (apply is idempotent), just wasteful.

### fn read_log_header

**Identification** — marker `// md:impl FsBackend > fn read_log_header`.

**What it does** — A log's `(epoch, header_byte_len)`; `(0, 0)` when there is
no header, so reading starts at byte 0. `header_byte_len` includes the newline
— exactly where the first change entry begins.

### fn read_other_logs_since

**Identification** — marker `// md:impl FsBackend > fn read_other_logs_since`.

**What it does** — Scans every **foreign** `.log` file from the beginning
(never advancing cursors) and returns entries with `timestamp > since`. Used
by `get_changes_since`, which needs a filtered view, not delivery tracking.
Own log skipped (local changes are already local state); non-`.log` files
skipped; malformed lines warned and skipped.

### fn read_new_entries

**Identification** — marker `// md:impl FsBackend > fn read_new_entries`.

**What it does** — Reads all new entries from each foreign log since the last
call and advances the byte-offset cursor (exactly-once delivery). Generation
epochs: when the foreign log's epoch differs from the cursor's, the log was
compacted — the stale offset is discarded and reading restarts just past the
new header, re-delivering the snapshot (idempotent + resolved ⇒ converges). A
failed cursor write is only a warning: the entries re-deliver next call,
safely.

**Used by** — `receive_changes`.

### fn write_sidecar

**Identification** — marker `// md:impl FsBackend > fn write_sidecar`.

**What it does** — MessagePack-encode + `atomic_write` (encode failure →
`InvalidState`).

### fn read_sidecar

**Identification** — marker `// md:impl FsBackend > fn read_sidecar`.

**What it does** — Read + decode; missing file → `NotFound(id)`, bad bytes →
`CorruptedData`.

### fn sidecar_vv

**Identification** — marker `// md:impl FsBackend > fn sidecar_vv`.

**What it does** — Deserialises only the `vv` field of a notebook/tag sidecar
(empty when the file is absent), to base a local write's incremented vector on
current state.

### fn next_sidecar_vv

**Identification** — marker `// md:impl FsBackend > fn next_sidecar_vv`.

**What it does** — `sidecar_vv` + increment this device's component.

### fn sidecar_incoming_wins

**Identification** — marker `// md:impl FsBackend > fn sidecar_incoming_wins`.

**What it does** — `resolve` over the stored vs incoming
`(vv, updated_at, last_writer)` of a notebook/tag sidecar; `true` when no
local sidecar exists.

**Used by** — `apply_change` (notebooks/tags).

### fn read_assoc_state

**Identification** — marker `// md:impl FsBackend > fn read_assoc_state`.

**What it does** — Reads an association's versioned state (`None` when
absent). Two degenerate shapes both fall back to a "present, minimum
priority" marker (epoch-0 timestamp, empty vector) for different reasons: an
**empty** file is the pre-versioning marker format (designed back-compat — any
versioned write dominates); a **non-empty unparseable** file is corruption,
and the same weakest-priority reading is the least-harm recovery — the
association stays visible locally instead of vanishing, and the next
versioned peer state supersedes it through `resolve`. The corrupt case is
reported at **error** level but stays non-fatal.

### fn next_assoc_vv

**Identification** — marker `// md:impl FsBackend > fn next_assoc_vv`.

**What it does** — Current association vector (empty if new) + increment.

### fn assoc_incoming_wins

**Identification** — marker `// md:impl FsBackend > fn assoc_incoming_wins`.

**What it does** — `resolve` for association writes; `true` with no local
file.

### fn write_assoc_state

**Identification** — marker `// md:impl FsBackend > fn write_assoc_state`.

**What it does** — Creates `note_tags/{note}` and writes the state sidecar.

### fn read_resource_meta

**Identification** — marker `// md:impl FsBackend > fn read_resource_meta`.

**What it does** — The resource metadata sidecar, `None` when absent.

### fn next_resource_vv

**Identification** — marker `// md:impl FsBackend > fn next_resource_vv`.

**What it does** — Current resource vector + increment.

### fn resource_incoming_wins

**Identification** — marker
`// md:impl FsBackend > fn resource_incoming_wins`.

**What it does** — `resolve` for resource changes; the tiebreak timestamp is
`deleted_at` when tombstoned else `created_at` (resources have no
`updated_at`); `true` with no local metadata.

### fn note_vv

**Identification** — marker `// md:impl FsBackend > fn note_vv`.

**What it does** — A note's merged vector from its meta projection (empty when
none) — the "what did we last materialise" reference for
`collect_advanced_notes`.

### fn read_note_logs

**Identification** — marker `// md:impl FsBackend > fn read_note_logs`.

**What it does** — Reads every `log.*.msgpack` for a note. A missing directory
yields empty; an unreadable individual log is **excluded from the merge and
reported at error level** (that device's entire history is missing — a
silent-data-loss risk, not routine). The file is left in place (it belongs to
another device; a local rename would replicate back to its writer), so a
restored copy re-enters the merge on the next read.

### fn merge_note

**Identification** — marker `// md:impl FsBackend > fn merge_note`.

**What it does** — Merge without touching disk. Reads use this so a read never
rewrites projections (no write amplification) and never consumes a peer change
the next sync should report. `None` when the note has no entries.

### fn materialize

**Identification** — marker `// md:impl FsBackend > fn materialize`.

**What it does** — Merge + refresh the `note.md`/`meta.msgpack` projection
(used by write and sync paths, never reads); a resolved concurrent conflict is
logged.

### fn persist_note_projection

**Identification** — marker
`// md:impl FsBackend > fn persist_note_projection`.

**What it does** — Writes the body to `note.md` and the blanked-body metadata
+ vector to `meta.msgpack` (both atomic). **The single choke point every note
write passes through**, so it also keeps the `NoteMetaIndex` current: on-disk
first, then the index entry (only if built — an unbuilt index misses nothing,
its eventual build reads the fresh projection). A crash between the two
leaves the index no staler than the projection.

### fn read_note_projection

**Identification** — marker
`// md:impl FsBackend > fn read_note_projection`.

**What it does** — The on-disk projection (metadata only; `None` when absent)
— used only to build the index cheaply.

### fn with_note_index

**Identification** — marker `// md:impl FsBackend > fn with_note_index`.

**What it does** — Runs `f` against the index, building it first when absent
(double-checked write lock → at most one concurrent build).

### fn build_note_index

**Identification** — marker `// md:impl FsBackend > fn build_note_index`.

**What it does** — Scans every note directory; metadata from projections, full
merge for notes with none (a peer note never materialised here) or an
unreadable one. Only live notes are indexed.

### fn materialize_page

**Identification** — marker `// md:impl FsBackend > fn materialize_page`.

**What it does** — Merges a page's ids into full notes, skipping any that no
longer merge live (a race with concurrent delete/move). Page-bounded — merge
cost is paid only for the returned page.

### fn append_note_op

**Identification** — marker `// md:impl FsBackend > fn append_note_op`.

**What it does** — **The single entry point for every local note mutation.**
Under `note_write_lock`: base the new entry's vector on the merge of every log
currently on disk (not the meta cache — so an edit causally follows all state
present at write time even though reads never refresh that cache) + increment;
read this device's log; append the entry; compact past
`NOTE_LOG_COMPACT_THRESHOLD` (single-writer log ⇒ `compact_own_log` is
lossless); atomic write; `materialize` and return the merged note (`NotFound`
if nothing merges).

### fn collect_advanced_notes

**Identification** — marker
`// md:impl FsBackend > fn collect_advanced_notes`.

**What it does** — Scans every note directory and re-materialises those whose
logs advanced beyond the stored projection (e.g. Syncthing just delivered a
peer's log). Comparison is by version vector, never file mtime — immune to
clock skew. Emits one `Change` per advanced note: `NoteUpdate` for live,
`NoteDelete` for tombstoned — carrying the **winning tombstone's own vv and
author**, not the joined frontier with an empty writer: a state-based peer
(`DbBackend`) resolves the delete by vector, and an empty vector would be
dominated by any local row, silently dropping the delete (issue #70).

**Used by** — `receive_changes`.

---

## KeyedItem

**Identification** — private `struct KeyedItem<T>`; marker `// md:KeyedItem`.

**What it does** — An item tagged with its `(created_at_rfc3339, id)`
pagination key, ordered by the key alone so `PageCollector`'s max-heap can
evict the largest candidate.

---

## impl PartialEq for KeyedItem

**Identification** — marker `// md:impl PartialEq for KeyedItem`.

**What it does** — Key equality.

---

## impl Eq for KeyedItem

**Identification** — marker `// md:impl Eq for KeyedItem`.

**What it does** — Marker impl.

---

## impl PartialOrd for KeyedItem

**Identification** — marker `// md:impl PartialOrd for KeyedItem`.

**What it does** — Delegates to `cmp`.

---

## impl Ord for KeyedItem

**Identification** — marker `// md:impl Ord for KeyedItem`.

**What it does** — Key ordering.

---

## PageCollector

**Identification** — private `struct PageCollector<T>`; marker
`// md:PageCollector`.

**What it does** — Streaming replacement for collect-everything-then-paginate:
retains only the `limit + 1` smallest keys past the cursor in a max-heap, so
building one page holds O(page) items instead of the whole store; the `+1`
overflow slot is how it learns whether a next page exists. Cursor semantics
and the produced token match `paginate` exactly.

**Used by** — the note listing methods.

---

## impl PageCollector

**Identification** — inherent impl; marker `// md:impl PageCollector`. Three
methods.

### fn new

**Identification** — marker `// md:impl PageCollector > fn new`.

**What it does** — Parses the `"<ts>|<uuid>"` cursor (`None`/empty/malformed
→ start at the beginning).

### fn push

**Identification** — marker `// md:impl PageCollector > fn push`.

**What it does** — Offers one candidate: keys at or before the cursor are
skipped (the same predicate as `paginate`'s partition point); the rest compete
for the retained slots (heap eviction of the largest).

### fn into_page

**Identification** — marker `// md:impl PageCollector > fn into_page`.

**What it does** — The retained items in ascending key order, trimmed to
`limit`, with a next-cursor when the overflow slot proved more exist.

---

## fn paginate

**Identification** —
`fn paginate<T, F>(items, limit, token, key_fn) -> (Vec<T>, Option<String>)`;
marker `// md:fn paginate`.

**What it does** — Cursor pagination over an already-sorted vec: partition
past the `"<ts>|<uuid>"` cursor (strictly after the cursor pair), take
`limit`, emit a next token from the page's last item when more remain.

**Used by** — the notebook/tag/resource listings (which sort small collected
vecs).

---

## impl NoteRepository for FsBackend

**Identification** — marker `// md:impl NoteRepository for FsBackend`;
per-method markers `> fn <name>`.

**What it does** — the note surface over the log pipeline.

### fn create_note

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn create_note`.

**What it does** — `append_note_op(Upsert(note))` — a create is just the first
op.

### fn read_note

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn read_note`.

**What it does** — Live merge (`merge_note`) — always current, even right
after Syncthing delivers a peer log — and never writes the projection back.
`NotFound` when nothing merges.

### fn update_note

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn update_note`.

**What it does** — `NotFound` when the note has no logs at all, else
`append_note_op(Upsert)`.

### fn delete_note

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn delete_note`.

**What it does** — `NotFound` without logs, else
`append_note_op(Tombstone { deleted_at: now })`.

### fn list_notes

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn list_notes`.

**What it does** — Select + paginate ids from the in-memory index with a
`PageCollector` keyed by `(created_at, id)`, then `materialize_page` only that
page.

### fn list_notes_in_notebook

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn list_notes_in_notebook`.

**What it does** — Same, filtered to the notebook, keyed by the effective sort
key **zero-padded to 10 digits** so lexicographic heap order is numeric.

### fn list_starred_notes

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn list_starred_notes`.

**What it does** — Same, filtered to `is_starred`, keyed by
`(created_at, id)`.

### fn notebook_sort_profile

**Identification** — marker
`// md:impl NoteRepository for FsBackend > fn notebook_sort_profile`.

**What it does** — The notebook's effective keys straight from the index into
`NotebookSortProfile::from_effective_keys`.

---

## impl NotebookRepository for FsBackend

**Identification** — marker `// md:impl NotebookRepository for FsBackend`;
per-method markers `> fn <name>`.

**What it does** — sidecar CRUD + global-log journaling.

### fn create_notebook

**Identification** — marker
`// md:impl NotebookRepository for FsBackend > fn create_notebook`.

**What it does** — Stamp vv/writer (`next_sidecar_vv`), write the sidecar,
append a `"create"` log entry with the full record.

### fn read_notebook

**Identification** — marker
`// md:impl NotebookRepository for FsBackend > fn read_notebook`.

**What it does** — Sidecar read (`NotFound`/`CorruptedData` semantics).

### fn update_notebook

**Identification** — marker
`// md:impl NotebookRepository for FsBackend > fn update_notebook`.

**What it does** — `NotFound` when the sidecar doesn't exist, else stamp +
write + `"update"` entry.

### fn delete_notebook

**Identification** — marker
`// md:impl NotebookRepository for FsBackend > fn delete_notebook`.

**What it does** — Soft delete: read, set `deleted_at`/`updated_at`, bump vv,
write back, `"delete"` entry with `fs_tombstone_value`.

### fn list_notebooks

**Identification** — marker
`// md:impl NotebookRepository for FsBackend > fn list_notebooks`.

**What it does** — Scan the sidecar directory, keep live decodable notebooks
(failures warned and skipped), sort `(created_at, id)`, `paginate`.

---

## impl TagRepository for FsBackend

**Identification** — marker `// md:impl TagRepository for FsBackend`;
per-method markers `> fn <name>`.

**What it does** — mirrors the notebook pattern, plus versioned associations.

### fn create_tag

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn create_tag`.

**What it does** — Stamp + sidecar + `"create"` entry.

### fn read_tag

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn read_tag`.

**What it does** — Sidecar read.

### fn update_tag

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn update_tag`.

**What it does** — Existence check + stamp + sidecar + `"update"` entry.

### fn delete_tag

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn delete_tag`.

**What it does** — Soft delete + tombstone entry.

### fn list_tags

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn list_tags`.

**What it does** — Scan, filter live, sort, `paginate`.

### fn add_note_tag

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn add_note_tag`.

**What it does** — Both ends must exist and be live (merged note; live tag
sidecar) — `NotFound` otherwise; the API must not create dangling
associations (`apply_change` deliberately skips the check: sync delivery
order is not guaranteed). Then write the association's **present** state
(`deleted_at: None`, fresh vv) and an `"add"` log entry. Idempotent.

### fn remove_note_tag

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn remove_note_tag`.

**What it does** — Write the **tombstone** state (kept so it can beat a
concurrent add) and a `"remove"` entry. Idempotent.

### fn list_note_tags

**Identification** — marker
`// md:impl TagRepository for FsBackend > fn list_note_tags`.

**What it does** — Walk `note_tags/{note}`, skip tombstoned associations and
deleted/unreadable tags, sort, `paginate`.

---

## impl ResourceRepository for FsBackend

**Identification** — marker `// md:impl ResourceRepository for FsBackend`;
per-method markers `> fn <name>`.

**What it does** — resources as `meta.msgpack` + `data`.

### fn create_resource

**Identification** — marker
`// md:impl ResourceRepository for FsBackend > fn create_resource`.

**What it does** — Stamp vv/writer, write the **data file first, metadata
last**: `read_resource` treats `meta.msgpack` as proof of existence, so the
metadata write is the commit marker — a crash between the two leaves an
orphan data file (harmless, overwritten on retry) rather than metadata
pointing at a missing payload. Then a `"create"` log entry (metadata only).

### fn read_resource

**Identification** — marker
`// md:impl ResourceRepository for FsBackend > fn read_resource`.

**What it does** — `NotFound` without metadata or when tombstoned (the
tombstone is kept for sync); else metadata + data bytes.

### fn delete_resource

**Identification** — marker
`// md:impl ResourceRepository for FsBackend > fn delete_resource`.

**What it does** — Soft delete: tombstone + bumped vv in the metadata; the
payload is retained; `"delete"` entry.

### fn list_resources

**Identification** — marker
`// md:impl ResourceRepository for FsBackend > fn list_resources`.

**What it does** — Scan resource dirs, keep live decodable metadata, sort,
`paginate` (no payloads).

### fn purge_deleted_resources

**Identification** — marker
`// md:impl ResourceRepository for FsBackend > fn purge_deleted_resources`.

**What it does** — Removes the `data` file of every resource whose readable
tombstone is older than the cutoff (live resources, crashed-create orphans,
and unreadable metas conservatively keep their payloads). The removal
replicates as a deletion through Syncthing — safe: every peer converges on
the same tombstone, and a late concurrent revive rewrites the file. Tombstone
metadata always survives.

---

## impl SyncBackend for FsBackend

**Identification** — marker `// md:impl SyncBackend for FsBackend`; per-method
markers `> fn <name>`.

**What it does** — the passive-replication sync surface.

### fn get_changes_since

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn get_changes_since`.

**What it does** — Foreign-log entries after `since`
(`read_other_logs_since`) mapped through `log_entry_to_change`; unrecognised
entries warned and skipped. **Note**: only the global journal — notes are not
emitted here (they travel per-note logs), which is why `migrate.rs` uses
typed copies instead of a raw change bridge.

### fn apply_change

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn apply_change`.

**What it does** — Per variant:

- **Notes (create/update/delete)** — just `materialize(id)`: conflict
  resolution lives entirely in the per-device logs Syncthing already
  replicated, so applying is a re-materialisation — it never appends to this
  device's log (no spurious local edit) and is idempotent.
- **Notebook/tag create/update** — `sidecar_incoming_wins` gate, then write
  the sidecar (stale changes logged and skipped).
- **Notebook/tag delete** — gate, then tombstone the existing sidecar or —
  unknown locally — write a **minimal tombstone** so a later stale
  create/update loses in `resolve` instead of resurrecting it (issue #71).
- **NoteTagAdd/Remove** — `assoc_incoming_wins` gate, then write the
  present/tombstone state.
- **ResourceCreate** — gate; write metadata, and the payload only when the
  change carries one (`data = Some` from a DbBackend peer; `None` from an
  FsBackend peer whose data file Syncthing replicates independently).
- **ResourceDelete** — gate; tombstone existing metadata or write a minimal
  tombstone (issue #71); the blob is retained.

### fn get_last_sync_time

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn get_last_sync_time`.

**What it does** — `.keeplin/sync_state.msgpack`, epoch when absent.

### fn update_sync_time

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn update_sync_time`.

**What it does** — Atomic sidecar write of the watermark.

### fn send_changes

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn send_changes`.

**What it does** — A no-op: Syncthing replicates `logs/` passively. Exists
(and succeeds) because `SyncEngine` calls it in the standard cycle.

### fn receive_changes

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn receive_changes`.

**What it does** — Cursor-advanced foreign-log entries
(notebooks/tags/resources via `read_new_entries` + `log_entry_to_change`)
**plus** `collect_advanced_notes` (notes whose per-note logs advanced — e.g. a
peer's log just arrived via Syncthing — re-materialised and reported as
changes). This is the call that *materialises* replicated peer notes, which is
why `LinkingBackend` invalidates its alias index on any note/notebook change
reported here.

### fn get_device_id

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn get_device_id`.

**What it does** — The cached id.

### fn prune_change_journal

**Identification** — marker
`// md:impl SyncBackend for FsBackend > fn prune_change_journal`.

**What it does** — Always `Ok(0)`: peers track entries by byte offset, so
time-based deletion could permanently lose changes for a lagging peer;
epoch-snapshot compaction does the bounding instead.

---

## impl FsBackend (global history)

**Identification** — the second inherent impl; marker
`// md:impl FsBackend (global history)`. One method.

### fn read_all_global_entries

**Identification** — marker
`// md:impl FsBackend (global history) > fn read_all_global_entries`.

**What it does** — Reads every global log (`logs/*.log`) and returns each
change entry paired with the writing device (the file stem). Headers, blanks,
and unparseable lines skipped. Used only by notebook history.

---

## impl HistoryRepository for FsBackend

**Identification** — marker `// md:impl HistoryRepository for FsBackend`;
per-method markers `> fn <name>`.

**What it does** — journal-derived history.

### fn note_history

**Identification** — marker
`// md:impl HistoryRepository for FsBackend > fn note_history`.

**What it does** — The per-note op logs already *are* the history: every
entry becomes an `EntityVersion` (`Upsert` → snapshot, `Tombstone` → `None`),
sorted newest-first and capped by `sort_and_cap`. Depth is bounded by the
256-entry compaction.

### fn notebook_history

**Identification** — marker
`// md:impl HistoryRepository for FsBackend > fn notebook_history`.

**What it does** — Notebooks are state-based sidecars, so their history lives
only in the global journal — which compacts to current state, so this is
**best-effort**: whatever versions the journal still holds
(create/update → snapshot, delete → tombstone, per writing device).

---

## fn sort_and_cap

**Identification** — `fn sort_and_cap<T>(versions, limit)`; marker
`// md:fn sort_and_cap`.

**What it does** — Orders a history list newest-first
(`(timestamp, device_id)` descending — the same total order the merge
tiebreaks on) and truncates to `limit` (`0` → `DEFAULT_HISTORY_LIMIT`).

**Used by** — both history methods.

---

## mod tests

**Identification** — `#[cfg(test)] mod tests`; marker `// md:mod tests`.
Twelve tests.

**What it does** — Pins the concurrency, purity, pagination, hygiene,
corruption-recovery, compaction, purge, and format-version behaviours.

### fn concurrent_same_note_updates_keep_every_log_entry

**Identification** — multi-thread tokio test; marker
`// md:mod tests > fn concurrent_same_note_updates_keep_every_log_entry`.

**What it does** — Regression for the lost-update race: 20 concurrent updates
to one note all land in the single-device log (create + 20 entries, none
dropped by a racing rename) — the `note_write_lock` guarantee.

### fn read_does_not_rewrite_projection

**Identification** — tokio test; marker
`// md:mod tests > fn read_does_not_rewrite_projection`.

**What it does** — Delete the projection files; `read_note` still answers
from the logs and does **not** recreate `note.md`/`meta.msgpack` (reads are
pure).

### fn list_notes_pages_match_full_walk

**Identification** — tokio test; marker
`// md:mod tests > fn list_notes_pages_match_full_walk`.

**What it does** — 23 notes (one deleted) walked in pages of 7 reproduce the
full `(created_at, id)` order — the heap `PageCollector` paginates exactly
like sort-then-`paginate`.

### fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones

**Identification** — tokio test; marker
`// md:mod tests > fn startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones`.

**What it does** — Planted `*.tmp` files in every managed dir are swept on
startup; a `.syncthing.*.tmp` survives; the store still reads.

### fn failed_atomic_write_cleans_up_its_temp_file

**Identification** — tokio test; marker
`// md:mod tests > fn failed_atomic_write_cleans_up_its_temp_file`.

**What it does** — A rename-blocked `atomic_write` errors, removes its temp
file, and leaves the destination untouched.

### fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it

**Identification** — tokio test; marker
`// md:mod tests > fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it`.

**What it does** — A corrupted association file still lists as attached
(least harm), and a versioned peer remove supersedes the epoch-0 fallback
marker.

### fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair

**Identification** — tokio test; marker
`// md:mod tests > fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair`.

**What it does** — With a corrupted notebook sidecar the journal is not
rewritten and no epoch is produced; after repair, compaction produces epoch 1
containing the notebook.

### fn detects_syncthing_conflict_copies_without_removing_them

**Identification** — tokio test; marker
`// md:mod tests > fn detects_syncthing_conflict_copies_without_removing_them`.

**What it does** — Conflict copies in `.keeplin/`, `notebooks/`, and a note
dir are all detected, never deleted, and never block startup.

### fn purge_reclaims_old_tombstoned_payloads_only

**Identification** — tokio test; marker
`// md:mod tests > fn purge_reclaims_old_tombstoned_payloads_only`.

**What it does** — A pre-tombstone cutoff purges nothing; a later cutoff
frees exactly the dead payload while the tombstone metadata survives and live
resources are untouched; purge is idempotent and the id can be recreated.

### fn fresh_store_is_stamped_current_version

**Identification** — tokio test; marker
`// md:mod tests > fn fresh_store_is_stamped_current_version`.

**What it does** — A brand-new store starts stamped `FORMAT_VERSION`.

### fn migrates_a_legacy_stamp_and_preserves_data

**Identification** — tokio test; marker
`// md:mod tests > fn migrates_a_legacy_stamp_and_preserves_data`.

**What it does** — A store rolled back to stamp 1 reopens through the ladder
to the current stamp with data intact.

### fn refuses_to_open_a_newer_format

**Identification** — tokio test; marker
`// md:mod tests > fn refuses_to_open_a_newer_format`.

**What it does** — A stamp of `FORMAT_VERSION + 1` is refused with the
"newer than this build" `InvalidState`.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `FsBackend` — defined here (EXTRACTED; the filesystem backend root)
- the repository-trait implementations (implements×6) and the log/merge/pagination helpers (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs` — error types (EXTRACTED: references×75)
- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: calls×1, references×37)
- `keeplin-core/src/storage/backend.rs` — the trait family (EXTRACTED: implements×6, references×12)
- `keeplin-core/src/storage/note_log.rs` — `merge`/`resolve`/`compact_own_log`/`VersionVector` (EXTRACTED: references×2)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/main.rs` — `build_storage` default mode (INFERRED)
- `keeplin-core/src/{history,ordering,linking,interop}.rs` tests — the cheapest real backend (EXTRACTED)
- `keeplin-core/tests/fs_backend.rs`, `tests/migrate.rs`, `tests/encryption.rs` (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2–8 | `NoteMeta`, `NoteMetaEntry` (+impl), `NoteMetaIndex` (+impl), `NoteTagState`, `LogEntry` | struct/impl markers as listed above |
| 9–18 | `fn default_entity_type` … `fn log_entry_to_change`, `fn atomic_write`, `SyncState` | `// md:fn <name>` / struct markers |
| 19 | `struct FsBackend` | `// md:FsBackend` |
| 20 | first `impl FsBackend` (constructor + ~45 helpers, incl. the four consts) | `// md:impl FsBackend` (+ `> fn …` per method) |
| 21 | `KeyedItem` + its four trait impls | one marker each |
| 22 | `PageCollector` (+impl: `new`, `push`, `into_page`) | `// md:PageCollector` / `// md:impl PageCollector` (+ `> fn …`) |
| 23 | `fn paginate` | `// md:fn paginate` |
| 24–28 | the five repository trait impls | `// md:impl <Trait> for FsBackend` (+ `> fn …` per method) |
| 29 | second `impl FsBackend` (global history) | `// md:impl FsBackend (global history)` (+ `> fn read_all_global_entries`) |
| 30 | `impl HistoryRepository for FsBackend` | marker + `> fn …` |
| 31 | `fn sort_and_cap` | `// md:fn sort_and_cap` |
| 32 | `mod tests` (+ twelve tests) | `// md:mod tests` (+ `> fn …`) |

Note: the four consts inside the first impl (`FORMAT_VERSION`,
`NOTE_LOG_COMPACT_THRESHOLD`, `GLOBAL_LOG_COMPACT_THRESHOLD`,
`GLOBAL_LOG_SOFT_BYTES`) are covered by the sections of the methods that use
them and carry no separate markers.
