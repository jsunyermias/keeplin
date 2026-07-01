# `tests/sync.rs` — cross-device change-propagation tests

## What is tested

These integration tests model **two independent devices**, each with its own backend, and
verify that a `Change` recorded on one device can be collected with
`SyncBackend::get_changes_since` and replayed on the other with `SyncBackend::apply_change`
until both converge. They also pin down the **last-write-wins** conflict semantics and the
**tombstone** rules that stop a stale edit from resurrecting a delete.

Most tests use two `DbBackend` instances (last-write-wins for all entities); one test uses
`FsBackend` to confirm its tombstones resolve the same way by timestamp.

## Test cases

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `create_propagates_between_devices` | Device A creates a note; collect + apply on B | B reads the same note back |
| `stale_remote_update_does_not_clobber_newer_local` | Apply a remote update older than the local `updated_at` | The newer local note is kept (LWW) |
| `db_stale_delete_does_not_override_newer_edit` | Apply a delete older than a later local edit | The note stays alive (the delete loses) |
| `db_stale_update_does_not_resurrect_tombstone` | Apply an update older than a local tombstone | The note stays deleted (the tombstone wins) |
| `fs_tombstones_resolve_by_timestamp` | Same delete-vs-edit race on `FsBackend` | Resolved deterministically by timestamp |

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `device()` | Builds a fresh `DbBackend` on a temp `.db` file in offline mode (no server URL) |
| `tempfile::tempdir()` | Unique temp dir removed when the guard drops |

## Related files

- `keeplin-core/src/storage/db.rs` — `apply_change` + `should_apply` (the LWW guard under test)
- `keeplin-core/src/storage/fs.rs` — the filesystem backend used by the last case
- `keeplin-core/tests/ws_sync.rs` — the same propagation over a real WebSocket relay
- `SECURITY.md` — "Conflict resolution differs by backend"
