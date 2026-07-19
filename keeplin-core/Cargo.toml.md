# `Cargo.toml` — keeplin-core

## Crate purpose

`keeplin-core` is the library crate that provides the complete Keeplin data model,
storage backends, encryption layer, and synchronisation engine. It has no binary targets
and is not published to crates.io; it is consumed exclusively by `keeplin-daemon` via a
local path dependency.

## Runtime dependencies

| Crate | Version | Why |
|-------|---------|-----|
| `tokio` | workspace | Async runtime for all I/O operations |
| `serde` | workspace | Derive macros for serialising domain types to JSON |
| `serde_json` | workspace | JSON encoding for log files and the change journal |
| `chrono` | workspace | UTC timestamps on all domain types |
| `uuid` | workspace | UUID v4 IDs generated at entity creation |
| `thiserror` | workspace | Derive macros for `StorageError` and `SyncError` |
| `anyhow` | workspace | General error propagation in a few utility functions |
| `async-trait` | workspace | `#[async_trait]` macro to allow `async fn` in `StorageBackend` |
| `tracing` | workspace | Structured log emission inside sync and storage code |
| `libsql` | 0.6 (`default-features = false`, `core`) | Embedded LibSQL/SQLite database for `DbBackend`. Default features are disabled so only the local engine is built — the remote replication stack (hyper-rustls / rustls 0.22 / rustls-webpki 0.102) is excluded, since `DbBackend` opens databases with `Builder::new_local` and never replicates through libsql. This also removes the source of four RUSTSEC advisories. |
| `tokio-tungstenite` | 0.24 (`rustls-tls-native-roots`) | Async WebSocket client for `DbBackend`'s real-time sync channel |
| `futures-util` | 0.3 | `SinkExt` and `StreamExt` for WebSocket message sending and receiving |
| `aes-gcm` | 0.10 | AES-256-GCM authenticated encryption used by `EncryptedBackend` |
| `argon2` | 0.5 | Argon2id key derivation for turning the user's passphrase into an AES key |
| `base64` | 0.22 | Base64 encoding of `(nonce ‖ ciphertext)` for string fields, and of binary resource payloads in the change journal |

## Dev / build dependencies

| Crate | Version | Why |
|-------|---------|-----|
| `tempfile` | workspace | Temporary directories for integration tests |
| `tokio` (full) | workspace | `#[tokio::test]` macro for async test functions |

## Feature flags

No feature flags are declared in this crate. The `libsql` dependency disables default
features and enables only `core`, which builds the local SQLite engine without the
remote replication stack.

## Build-time notes

- There is no `build.rs` in this crate. Only `keeplin-daemon` has a build script (for
  Protocol Buffers compilation).
- The `libsql` crate with `feature = "core"` compiles a bundled copy of the SQLite C
  library. This makes the first build slower (C compilation) but produces a self-contained
  library binary.

## Related files

- `Cargo.toml` (workspace root) — declares shared package metadata and dependency versions
- `keeplin-daemon/Cargo.toml` — the binary crate that depends on this library
