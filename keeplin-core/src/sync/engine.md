# `sync/engine.rs` — SyncEngine

## Purpose

This module implements `SyncEngine<T>`, which orchestrates a complete synchronisation
cycle for any backend that implements `StorageBackend`. It is deliberately thin: all the
real work (collecting changes, sending, receiving, applying) is delegated to the backend.
`SyncEngine` only sequences these operations and handles the sync timestamp bookkeeping.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `SyncEngine<T>` | struct | Holds a backend and exposes a `sync()` method |

## Public API

### `SyncEngine::new(backend: T) -> Self`
**What it does:** Constructs a new engine wrapping the given backend.  
**Parameters:** `backend` — any value implementing `StorageBackend + Send + Sync + 'static`.  
**Returns:** A ready-to-use engine. The `backend` field is `pub` for direct access.

### `async fn sync(&self) -> Result<Vec<Change>, SyncError>`
**What it does:** Runs one complete push/pull synchronisation cycle and returns the list
of remote changes that were applied locally during this cycle.  
**Returns:** `Ok(remote_changes)` — the changes received from the remote peer and applied.  
**Errors:** `SyncError::Storage` if any storage operation fails; `SyncError::Conflict` if
the backend detects a write conflict.

## Data flow

The sync cycle executes six steps in sequence:

1. **Retrieve last-sync timestamp** — `get_last_sync_time()` returns the UTC timestamp
   of the most recent successful sync, or the Unix epoch if this is the first sync.
2. **Collect local changes** — `get_changes_since(last_sync)` returns all `Change`
   events recorded on this device since the previous sync.
3. **Push local changes** — `send_changes(local_changes)` transmits the changes to the
   remote peer (WebSocket for `DbBackend`, no-op for `FsBackend` which relies on
   Syncthing to replicate its log files).
4. **Pull remote changes** — `receive_changes()` retrieves changes that other devices
   sent to the remote peer since the last pull.
5. **Apply remote changes locally** — each `Change` from the remote is applied in
   order via `apply_change(change)`. All `apply_change` implementations are idempotent,
   so re-running a cycle after a partial failure is safe.
6. **Record new sync timestamp** — `update_sync_time(now())` persists the current UTC
   time as the new last-sync point, so the next cycle only collects changes made after
   this moment.

## Design notes

- `SyncEngine` is generic over `T: StorageBackend`. This means there is no dynamic
  dispatch and no `Box<dyn StorageBackend>` indirection. The compiler monomorphises a
  separate sync function for each concrete backend type.
- The sync cycle uses a last-write-wins conflict resolution strategy: whichever change
  is applied last to the local store wins. Because all changes are applied in
  chronological order of their remote timestamps, the outcome across all devices
  eventually converges to the same state.
- The `SyncEngine` does not retry on failure. The caller is responsible for scheduling
  and retrying sync cycles; a simple approach is to call `sync()` periodically or on
  reconnect.

## Related files

- `keeplin-core/src/storage/backend.rs` — `StorageBackend` trait that `T` must satisfy
- `keeplin-core/src/sync/mod.rs` — re-exports `SyncEngine`
- `keeplin-daemon/src/server.rs` — the gRPC `Sync` RPC drives the same sequence directly
  against the backend (without going through `SyncEngine`)
