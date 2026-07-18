# `error.rs` — error types

Self-contained companion for `keeplin-core/src/error.rs`. It documents **every code
block of the source file, in source order** — a reader with only this file must be able
to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview`.

```rust
use thiserror::Error;
```

**What it does** — All error types used throughout `keeplin-core`, centralised so
every module returns a consistent type without circular dependencies. Storage
operations return `StorageError`; the synchronisation layer wraps it in `SyncError`
for sync-specific failure cases. Conversions from third-party errors
(`libsql::Error`, `tungstenite::Error`) are `From` impls so callers use `?` without
manual mapping. No logic beyond those conversions.

**Dependencies** — `thiserror` (derive); `libsql`, `tokio_tungstenite`,
`serde_json`, `std::io` appear in variant/impl signatures.

**Used by** — every module of the crate (`storage::backend` uses `StorageError` in
every trait method signature) and the daemon's error mapping (`rest.rs::ApiError`,
`server.rs::storage_err`).

**Repeated context** — Crate error conventions: **all** errors live here — other
modules must not define their own error enums; error messages shown to callers must
never contain sensitive data (passwords, keys, plaintext).

---

## StorageError

**Identification** — enum deriving `Debug` + `thiserror::Error`; marker
`// md:StorageError`.

**What it does** — Every error a storage operation can raise. `Io` and
`Serialization` wrap their third-party error via `#[from]` (auto with `?`);
`Database` and `WebSocket` also work with `?` through the hand-written `From` impls
below, which flatten the source into a `String` because the chains are worth
preserving as text. The `String`-only variants are always constructed explicitly at
the call site. Variant table:

| Variant | Source | When |
|---------|--------|------|
| `Io(std::io::Error)` | `#[from]` | filesystem read/write failure in `FsBackend` |
| `Serialization(serde_json::Error)` | `#[from]` | serde_json parse/serialise failure (e.g. a malformed global-log `data` value); MessagePack sidecars/logs decoded via `rmp_serde` do **not** land here — their decode failures surface as `CorruptedData` |
| `Database(String)` | `From<libsql::Error>` (chain flattened) | LibSQL/SQLite error; the payload includes the full nested chain so the root cause is visible in logs |
| `WebSocket(String)` | `From<tungstenite::Error>` | WebSocket protocol/connection error during sync |
| `NotFound(String)` | call site | the entity does not exist (or was soft-deleted); payload names it (e.g. `"note 3f4a…"`) |
| `Conflict(String)` | call site | a write rejected because it conflicts with existing state — today a **duplicate alias** from `LinkingBackend` (a create/update claims an alias already held by another live entity of the same type). Daemon maps it to HTTP `409` / gRPC `ALREADY_EXISTS`. Concurrent *edits* never surface this: `apply_change` reconciles them via version vectors + the deterministic `(timestamp, device_id)` tiebreak (`storage::note_log::resolve`) |
| `InvalidState(String)` | call site | unexpected internal state (key-derivation errors etc.) — the server's problem: HTTP `500` / gRPC `INTERNAL` |
| `InvalidInput(String)` | call site | the caller broke a domain rule — pinning an inbox note, a sort key outside the note's band, deleting the inbox. The client's mistake: HTTP `400` / gRPC `INVALID_ARGUMENT` |
| `CorruptedData(String)` | call site | stored data failed to decrypt: AES-GCM tag verification failed (wrong password or tampered ciphertext), bad base64, short buffer, or non-UTF-8 plaintext |

**Dependencies** — `thiserror`; the wrapped third-party error types.

**Used by** — every backend/decorator (`fs.rs` ×75 refs, `db.rs` ×69,
`encryption.rs`, `linking.rs`, `collab/mod.rs`, `ordering.rs`, `history.rs`,
`interop.rs`, `migrate.rs`), the daemon (`event_backend.rs`, `metrics.rs`,
`rest.rs`, `server.rs`), and `SyncError` below.

**Repeated context** — The `Conflict` vs `InvalidInput` vs `InvalidState`
distinction is the daemon's HTTP/gRPC status mapping contract, restated here because
handlers rely on it: 409/`ALREADY_EXISTS`, 400/`INVALID_ARGUMENT`,
500/`INTERNAL` respectively.

---

## impl From libsql for StorageError

**Identification** — trait impl (containing `fn from`); marker
`// md:impl From libsql for StorageError`.

**What it does** — Converts a `libsql::Error` into `StorageError::Database`,
flattening the **entire error source chain** into one `String` (each nested cause
appended on a new line, `caused by: …`) so the underlying SQLite error code/message
is not lost when the `libsql::Error` is dropped.

**Dependencies** — `std::error::Error::source` chain walking.

**Used by** — `?` in `storage/db.rs` (every LibSQL call).

**Repeated context** — `Database` stores a `String` (not `Box<dyn Error>`) so
`StorageError` stays `Send + Sync + 'static` cheaply; the trade-off is one
allocation per multi-hop chain.

---

## impl From tungstenite for StorageError

**Identification** — trait impl (containing `fn from`); marker
`// md:impl From tungstenite for StorageError`.

**What it does** — Converts a `tokio_tungstenite::tungstenite::Error` (WebSocket
protocol/connection error) into `StorageError::WebSocket(e.to_string())`.

**Dependencies** — `tokio_tungstenite`.

**Used by** — `?` in `storage/db.rs` (relay connection) and `collab/mod.rs`.

**Repeated context** — none.

---

## SyncError

**Identification** — enum deriving `Debug` + `thiserror::Error`; marker
`// md:SyncError`.

**What it does** — Errors specific to the synchronisation layer, wrapping
`StorageError` for the common case rather than flattening all variants into one enum
(keeps each layer's error contract separate):

| Variant | When |
|---------|------|
| `Storage(StorageError)` | `#[from]` — a storage operation failed during the sync cycle |
| `Conflict { local_id, remote_id }` | **reserved**: the default cycle resolves conflicts automatically via version vectors (+ the `(timestamp, device_id)` tiebreak, `note_log::resolve`) and never returns it; exists for callers layering strict conflict detection on `sync::run_sync` |
| `Failed(String)` | **reserved**: a non-storage sync failure (e.g. unexpected remote response format) |

**Dependencies** — `StorageError` (this file), `thiserror`.

**Used by** — `sync/engine.rs` (`run_sync`, `SyncEngine::sync`).

**Repeated context** — Automatic conflict resolution is the system default —
version vectors decide, ties break deterministically — so `Conflict` staying
reserved is by design, not an omission.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `StorageError` — defined here (EXTRACTED; 400 cross-file edge(s))
- `SyncError` — defined here (EXTRACTED; 4 cross-file edge(s))
- `.from()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/collab/mod.rs` — client of the keeplin-srv collaborative channel (EXTRACTED: imports_from×1, references×41)
- `keeplin-core/src/encryption.rs` — transparent at-rest encryption (EXTRACTED: references×51)
- `keeplin-core/src/history.rs` — change history reads + forward-revert (EXTRACTED: references×4)
- `keeplin-core/src/interop.rs` — vCard & iCalendar format compatibility (EXTRACTED: imports_from×1, references×15)
- `keeplin-core/src/linking.rs` — `LinkingBackend` decorator + reference resolution (EXTRACTED: references×51)
- `keeplin-core/src/migrate.rs` — one-shot state copy between backends (EXTRACTED: references×2)
- `keeplin-core/src/ordering.rs` — the inbox, pinning, manual ordering, and starring (EXTRACTED: references×11)
- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: references×69)
- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: references×75)
- `keeplin-core/src/sync/engine.rs` — SyncEngine (EXTRACTED: references×2)
- `keeplin-daemon/src/event_backend.rs` — `EventBackend` change-publishing decorator (EXTRACTED: references×37)
- `keeplin-daemon/src/metrics.rs` — operational metrics (EXTRACTED: references×38)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: references×4)
- `keeplin-daemon/src/server.rs` — gRPC service implementation (EXTRACTED: references×2)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `enum StorageError` | `// md:StorageError` |
| 3 | `impl From<libsql::Error>` | `// md:impl From libsql for StorageError` |
| 4 | `impl From<tungstenite::Error>` | `// md:impl From tungstenite for StorageError` |
| 5 | `enum SyncError` | `// md:SyncError` |
