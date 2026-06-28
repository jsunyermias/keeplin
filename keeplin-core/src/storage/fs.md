# `storage/fs.rs` — FsBackend (filesystem storage)

## Purpose

`FsBackend` stores all Keeplin data as JSON files on the local filesystem and records
every mutation in a per-device Newline-Delimited JSON (NDJSON) change log. Synchronisation
across devices is handled by an external file-replication tool such as Syncthing: when
Syncthing replicates the `logs/` directory, `get_changes_since` reads the log files
written by other devices to discover what changed remotely.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `FsBackend` | struct | Filesystem-backed `StorageBackend` implementation |
| `LogEntry` | struct (private) | One line in a device's NDJSON change log |
| `SyncState` | struct (private) | Wrapper holding the `last_sync` timestamp, persisted to JSON |

## Directory layout

```
{root}/
├── notes/{uuid}/meta.json            ← Note metadata (JSON)
├── notebooks/{uuid}.json             ← Notebook metadata (JSON)
├── tags/{uuid}.json                  ← Tag metadata (JSON)
├── note_tags/{note_uuid}/{tag_uuid}  ← Marker files (empty); existence = linked
├── resources/{uuid}/meta.json        ← Resource metadata (JSON)
├── resources/{uuid}/data             ← Resource binary payload
├── logs/{device_id}.log              ← This device's NDJSON change log
├── .keeplin/device_id                ← Stable identifier for this installation
├── .keeplin/format_version           ← Storage format version number (currently "2")
├── .keeplin/offsets/{device_id}      ← Byte offset into the corresponding log file
└── .keeplin/sync_state.json          ← Last successful sync timestamp
```

## Log entry format

Each NDJSON line is a `LogEntry`:

```json
{"timestamp":"2024-01-01T12:00:00Z","entity_type":"note","entity_id":"<uuid>","operation":"create","data":{...}}
```

### Backward compatibility with v1 logs

V1 log entries used `"note_id"` instead of `"entity_id"` and had no `"entity_type"`
field. The struct handles these via:
- `#[serde(default = "default_entity_type")]` — absent `entity_type` defaults to `"note"`
- `#[serde(alias = "note_id")]` — `"note_id"` is accepted as an alias for `"entity_id"`

## Format versioning

On startup, `ensure_format_version()` reads `.keeplin/format_version`:
- **Absent** — treated as version 1 (old installation)
- **Version 1** — logged, then updated to the current version; no data migration is
  required because v1 → v2 only adds new entity types and the log format is
  backward-compatible via the serde aliases above
- **Current version** — no action taken

The version file is always (re-)written on startup so that new installations are stamped
immediately.

## Public API

### `FsBackend::new(root: impl Into<PathBuf>) -> Result<Self, StorageError>`
**What it does:** Creates the required directory tree under `root`, reads or generates a
stable device ID, and checks/migrates the storage format version.  
**Parameters:** `root` — path to the root data directory.  
**Returns:** A ready-to-use backend.  
**Errors:** `StorageError::Io` if any directory cannot be created.

All other methods implement `StorageBackend` — see `storage/backend.md` for the full
contract.

## Atomic write pattern

All writes use a temp-file-then-rename pattern to avoid partial writes:

```rust
let tmp = final_path.with_extension("tmp");
tokio::fs::write(&tmp, bytes).await?;
tokio::fs::rename(&tmp, &final_path).await?;
```

This guarantees that a reader always sees either the old file or the new file, never a
partially-written state.

## Sync — `get_changes_since`

`get_changes_since(since)` does two things:

1. **Read changes from other devices** — scans all `.log` files in `logs/` except the
   current device's own log (because local changes are already known). For each file,
   it uses a byte-offset file in `.keeplin/offsets/` to avoid re-reading lines already
   processed. The offset is updated atomically after each batch.
2. **Read this device's own new changes** — reads new lines from the current device's
   log using the same offset mechanism.

Each log line is parsed via `log_entry_to_change()`. Unrecognised combinations of
`entity_type` and `operation` are skipped silently (future compatibility).

## Sync — `apply_change`

`apply_change` handles all 13 `Change` variants:
- Notes, notebooks, tags: write or overwrite the JSON metadata file
- NoteTagAdd: create the marker file `note_tags/{note_id}/{tag_id}`
- NoteTagRemove: remove the marker file (ignores `NotFound`)
- ResourceCreate: write `resources/{id}/meta.json`; if `data: Some(bytes)`, also write
  `resources/{id}/data` (useful when receiving from `DbBackend`; Syncthing handles the
  normal case)
- ResourceDelete: remove the entire `resources/{id}/` directory

## Design notes

- `send_changes` and `receive_changes` are no-ops in `FsBackend` because replication is
  fully handled by the external Syncthing daemon. The sync cycle in `SyncEngine` still
  calls them; they simply return immediately.
- NoteTag associations are stored as empty marker files rather than in a list file,
  so adding and removing a tag from a note are independent atomic operations.
- The `FsBackend` is not suitable for high-concurrency workloads: concurrent writes from
  two processes to the same entity directory can race. It is designed for single-user,
  single-process desktop use.

## Related files

- `keeplin-core/src/storage/backend.rs` — trait that `FsBackend` implements
- `keeplin-core/src/models.rs` — all types stored by this backend
- `.cargo/config.toml` — workspace release profile; relevant for binary used alongside
  Syncthing deployments
- `scripts/build.sh` — cross-compilation script for packaging the daemon with `FsBackend`
