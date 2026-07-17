# `error.rs` — error types

## Purpose

This module defines all error types used throughout `keeplin-core`. Centralising errors
here means every other module can return a consistent error type without introducing
circular dependencies. The module has no logic beyond error conversion (the `From` impls
below).

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `StorageError` | enum | Every error that can arise from a storage operation |
| `SyncError` | enum | Errors specific to the sync cycle (wraps `StorageError`) |

## `StorageError` variants

| Variant | Source type | When it arises |
|---------|-------------|----------------|
| `Io(std::io::Error)` | `#[from]` (auto via `?`) | Filesystem read/write failure in `FsBackend` |
| `Serialization(serde_json::Error)` | `#[from]` (auto via `?`) | `serde_json` parse/serialise failure (e.g. a malformed global-log `data` value; msgpack sidecars surface as `CorruptedData`) |
| `Database(String)` | hand-written `From<libsql::Error>` (auto via `?`; flattens the chain) | LibSQL or SQLite error (full chain included) |
| `WebSocket(String)` | hand-written `From<tungstenite::Error>` (auto via `?`) | `tokio-tungstenite` connection or protocol error |
| `NotFound(String)` | constructed at call site | Entity with the given ID does not exist |
| `Conflict(String)` | constructed at call site | Uniqueness violation the client should retry differently: a duplicate alias from `LinkingBackend`, or pinning past the 999-note limit (HTTP `409` / gRPC `ALREADY_EXISTS`). Concurrent *edits* never surface it — they resolve automatically by version vectors |
| `InvalidState(String)` | constructed at call site | Server-side unexpected internal state (e.g. key-derivation failure) — HTTP `500` / gRPC `INTERNAL` |
| `InvalidInput(String)` | constructed at call site | The caller broke a domain rule: pinning an Inbox note, an out-of-band `sort_key`, deleting the Inbox (HTTP `400` / gRPC `INVALID_ARGUMENT`). Distinct from `InvalidState` — the client's mistake, not the server's |
| `CorruptedData(String)` | constructed at call site | Stored data could not be decrypted (bad base64, short buffer, failed AES-GCM tag, or non-UTF-8 plaintext) |

## `SyncError` variants

| Variant | Description |
|---------|-------------|
| `Storage(StorageError)` | Underlying storage operation failed during sync |
| `Conflict { local_id, remote_id }` | Reserved — the default cycle resolves conflicts automatically via version vectors |
| `Failed(String)` | Reserved — general (non-storage) sync failure |

## `From` conversions

The module implements `From<libsql::Error>` manually (the `thiserror` `#[from]`
attribute handles `std::io::Error`, `serde_json::Error`, and `tungstenite::Error`).
The `libsql::Error` impl walks the full error source chain so that nested SQLite error
messages are preserved in the `Database` variant.

## Design notes

- `StorageError::Database` stores a `String` (not `Box<dyn Error>`) so that
  `StorageError` remains `Send + Sync + 'static` without a heap allocation per
  conversion. The trade-off is a small allocation for multi-hop error chains.
- `SyncError` wraps `StorageError` rather than flattening all variants into one enum,
  which keeps each layer's error contract separate and prevents accidental conflation.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `StorageError` — defined here (EXTRACTED; 400 cross-file edge(s))
- `SyncError` — defined here (EXTRACTED; 4 cross-file edge(s))
- `.from()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/collab/mod.rs` — client of the keeplin-srv collaborative channel (EXTRACTED: imports_from×1, references×41; e.g. `mod.rs`, `.apply_from_server()`, `.proxy_request()`)
- `keeplin-core/src/encryption.rs` — transparent at-rest encryption (EXTRACTED: references×51; e.g. `.new()`, `.encrypt_str()`, `.decrypt_str()`)
- `keeplin-core/src/history.rs` — change history reads + forward-revert (EXTRACTED: references×4; e.g. `revert_note()`, `revert_notebook()`, `revert_notebook_notes_to()`)
- `keeplin-core/src/interop.rs` — vCard & iCalendar format compatibility (EXTRACTED: imports_from×1, references×15; e.g. `contact_resources()`, `delete_contact()`, `delete_event()`)
- `keeplin-core/src/linking.rs` — `LinkingBackend` decorator + reference resolution (EXTRACTED: references×51; e.g. `add_manual_link()`, `alias_conflicts()`, `backlinks()`)
- `keeplin-core/src/migrate.rs` — one-shot state copy between backends (EXTRACTED: references×2; e.g. `collect()`, `migrate()`)
- `keeplin-core/src/ordering.rs` — the Inbox, pinning, manual ordering, and starring (EXTRACTED: references×11; e.g. `ensure_inbox()`, `pin_note()`, `place_new_note()`)
- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: references×69; e.g. `.add_column_if_missing()`, `.add_note_tag()`, `.apply_change()`)
- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: references×75; e.g. `atomic_write()`, `.add_note_tag()`, `.append_log()`)
- `keeplin-core/src/sync/engine.rs` — SyncEngine (EXTRACTED: references×2; e.g. `run_sync()`, `.sync()`)
- `keeplin-daemon/src/event_backend.rs` — `EventBackend` change-publishing decorator (EXTRACTED: references×37; e.g. `.add_note_tag()`, `.apply_change()`, `.create_note()`)
- `keeplin-daemon/src/metrics.rs` — operational metrics (EXTRACTED: references×38; e.g. `.add_note_tag()`, `.apply_change()`, `.create_note()`)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: references×4; e.g. `ApiError`, `.from()`)
- `keeplin-daemon/src/server.rs` — gRPC service implementation (EXTRACTED: references×2; e.g. `ensure_not_deleted()`, `storage_err()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- All crate errors live here; other modules must not define their own error enums (prevents circular deps and keeps `From` conversions in one place).
- Error messages shown to callers must not contain sensitive data (passwords, keys, plaintext).

## Related files

- `keeplin-core/src/storage/backend.rs` — uses `StorageError` in every method signature
- `keeplin-core/src/sync/engine.rs` — returns `SyncError`
