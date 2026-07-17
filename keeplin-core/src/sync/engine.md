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

The cycle lives in the free function `run_sync(backend, report)`; `SyncEngine::sync`
is a thin wrapper that calls it with a no-op `report` callback, and the daemon's
streaming `Sync` RPC calls it with a callback that emits a `SyncStage` progress update
before each step. This keeps the watermark/ordering logic in exactly one place.

The cycle executes six steps in sequence:

1. **Retrieve last-sync timestamp** — `get_last_sync_time()` returns the UTC timestamp
   of the most recent successful sync, or the Unix epoch if this is the first sync. The
   new watermark `sync_ts = now()` is captured **here, before** any changes are read.
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
6. **Record new sync timestamp** — `update_sync_time(sync_ts)` persists the watermark
   captured in step 1 (not a fresh `now()`). Using the start-of-cycle time guarantees
   that any change recorded *while* the cycle ran is still collected on the next cycle,
   rather than being silently skipped.

## Design notes

- `SyncEngine` is generic over `T: StorageBackend`. This means there is no dynamic
  dispatch and no `Box<dyn StorageBackend>` indirection. The compiler monomorphises a
  separate sync function for each concrete backend type.
- The `SyncEngine` does not itself resolve conflicts: it collects and hands each remote
  `Change` to `apply_change`, and every backend resolves it with **version vectors** (see
  `note_log::resolve`/`merge`). Because that decision is order-independent and deterministic —
  a strictly-dominating write wins, and a genuine concurrent conflict is broken by the shared
  `(timestamp, device_id)` tiebreak — the outcome across all devices converges to the same state
  regardless of the order changes arrive in.
- The `SyncEngine` does not retry on failure. The caller is responsible for scheduling
  and retrying sync cycles; a simple approach is to call `sync()` periodically or on
  reconnect.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `run_sync()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `.sync()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `SyncStage` — defined here (EXTRACTED; 1 cross-file edge(s))
- `SyncEngine` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.new()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `SyncEngine<T>` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs` — error types (EXTRACTED: references×2; e.g. `SyncError`)
- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: references×2; e.g. `Change`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: references×2; e.g. `T`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/tests/ws_sync.rs` — end-to-end WebSocket sync test (EXTRACTED: calls×1; e.g. `failed_send_keeps_watermark_and_changes_are_resent_after_recovery()`)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: calls×1; e.g. `sync()`)
- `keeplin-daemon/src/server.rs` — gRPC service implementation (EXTRACTED: calls×1, references×1; e.g. `stage_to_proto()`, `.sync()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- The engine only sequences push-then-pull and maintains the watermark; all real work is delegated to the backend.
- The last-sync watermark advances only after a fully successful cycle — a failed push/pull must leave it unchanged so changes are re-sent.

## Related files

- `keeplin-core/src/storage/backend.rs` — `StorageBackend` trait that `T` must satisfy
- `keeplin-core/src/sync/mod.rs` — re-exports `SyncEngine`
- `keeplin-daemon/src/server.rs` — the gRPC `Sync` RPC drives the same sequence directly
  against the backend (without going through `SyncEngine`)
