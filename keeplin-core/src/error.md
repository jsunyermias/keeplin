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
| `Io(std::io::Error)` | auto-converted | Filesystem read/write failure in `FsBackend` |
| `Serialization(serde_json::Error)` | auto-converted | JSON parse or serialise failure |
| `Database(String)` | manual impl | LibSQL or SQLite error (full chain included) |
| `WebSocket(String)` | auto-converted | `tokio-tungstenite` connection or protocol error |
| `NotFound(String)` | manual | Entity with the given ID does not exist |
| `Conflict(String)` | manual | Uniqueness violation the client should retry differently: a duplicate alias from `LinkingBackend`, or pinning past the 999-note limit (HTTP `409` / gRPC `ALREADY_EXISTS`). Concurrent *edits* never surface it — they resolve automatically by version vectors |
| `InvalidState(String)` | manual | Server-side unexpected internal state (e.g. key-derivation failure) — HTTP `500` / gRPC `INTERNAL` |
| `InvalidInput(String)` | manual | The caller broke a domain rule: pinning an Inbox note, an out-of-band `sort_key`, deleting the Inbox (HTTP `400` / gRPC `INVALID_ARGUMENT`). Distinct from `InvalidState` — the client's mistake, not the server's |
| `CorruptedData(String)` | manual | Stored data could not be decrypted (bad base64, short buffer, failed AES-GCM tag, or non-UTF-8 plaintext) |

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

## Related files

- `keeplin-core/src/storage/backend.rs` — uses `StorageError` in every method signature
- `keeplin-core/src/sync/engine.rs` — returns `SyncError`
