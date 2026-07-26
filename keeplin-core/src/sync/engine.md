# `sync/engine.rs` — SyncEngine

Self-contained companion for `keeplin-core/src/sync/engine.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use crate::{
    error::SyncError,
    models::{now, Change},
    storage::StorageBackend,
};
```

**What it does** — Drives a complete push-then-pull synchronisation cycle. The
module is intentionally thin: it sequences six operations against a
`StorageBackend` and handles the sync-timestamp bookkeeping; all real work
(collecting, sending, receiving, applying changes) is delegated to the backend. The
cycle lives in the free function `run_sync`, which takes a progress callback so
callers that surface per-stage progress (the gRPC daemon's streaming `Sync` RPC)
and callers that don't (`SyncEngine::sync`) share one implementation — the
watermark and ordering logic exists in exactly one place.

**Dependencies** — `crate::error::SyncError`, `crate::models::{now, Change}`,
`crate::storage::StorageBackend`; `tracing` for structured logs.

**Used by** — `sync/mod.rs` re-exports `run_sync`, `SyncEngine`, `SyncStage`;
`keeplin-daemon/src/server.rs` (streaming `Sync` RPC),
`keeplin-daemon/src/rest.rs` (sync endpoint), `keeplin-core/tests/ws_sync.rs`.

**Repeated context** — The engine never resolves conflicts itself: each remote
`Change` goes to `apply_change`, and every backend resolves it with version vectors
plus the deterministic `(timestamp, device_id)` last-writer-wins tiebreak
(`storage::note_log::resolve`/`merge`). That decision is order-independent, so all
devices converge regardless of arrival order.

---

## SyncStage

**Identification** — enum deriving `Debug, Clone, Copy, PartialEq, Eq`; marker
`// md:SyncStage`.

**Code** — complete and verbatim:

```rust
// md:SyncStage
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStage {
    Collecting,
    Sending,
    Receiving,
    Applying,
    Done,
}
```

**What it does** — The stage a synchronisation cycle has reached, reported through
the `run_sync` progress callback: `Collecting` (about to collect local changes
since the last sync), `Sending` (about to push them), `Receiving` (about to pull
the remote's changes), `Applying` (about to apply them locally), `Done` (cycle
finished successfully). Variants mirror the natural ordering of the cycle so a UI
can render a determinate progress bar.

**Dependencies** — none.

**Used by** — the `report` callback in `run_sync`;
`keeplin-daemon/src/server.rs::stage_to_proto` maps it onto the gRPC streaming
progress message.

**Repeated context** — none.

---

## fn run_sync

**Identification** —
`pub async fn run_sync<B, F>(backend: &B, mut report: F) -> Result<Vec<Change>, SyncError>`
where `B: StorageBackend + ?Sized`, `F: FnMut(SyncStage, usize)`; marker
`// md:fn run_sync`.

**Code** — complete and verbatim:

```rust
// md:fn run_sync
pub async fn run_sync<B, F>(backend: &B, mut report: F) -> Result<Vec<Change>, SyncError>
where
    B: StorageBackend + ?Sized,
    F: FnMut(SyncStage, usize),
{
    let last_sync = backend.get_last_sync_time().await?;
    tracing::info!(last_sync = %last_sync, "Starting sync");

    let sync_ts = now();

    report(SyncStage::Collecting, 0);
    let local_changes = backend.get_changes_since(last_sync).await?;
    tracing::info!(count = local_changes.len(), "Local changes collected");

    report(SyncStage::Sending, local_changes.len());
    backend.send_changes(local_changes).await?;
    tracing::info!("Local changes sent");

    report(SyncStage::Receiving, 0);
    let remote_changes = backend.receive_changes().await?;
    tracing::info!(count = remote_changes.len(), "Remote changes received");

    report(SyncStage::Applying, remote_changes.len());
    for change in &remote_changes {
        backend.apply_change(change.clone()).await?;
    }
    tracing::debug!(applied = remote_changes.len(), "Remote changes applied");

    backend.update_sync_time(sync_ts).await?;
    tracing::info!(new_sync_ts = %sync_ts, "Sync complete");

    report(SyncStage::Done, remote_changes.len());
    Ok(remote_changes)
}
```

**What it does** — Runs one complete push-then-pull cycle against `backend`,
invoking `report(stage, count)` immediately before each stage begins (and once more
with `SyncStage::Done` on success). The six steps:

1. `get_last_sync_time()` — read the timestamp of the most recent successful sync
   (Unix epoch on a first sync), defining which local changes are "new".
2. Capture the new watermark `sync_ts = now()` **before** collecting. Any mutation
   recorded while the cycle runs has `changed_at > sync_ts`, so it is guaranteed to
   be collected next cycle; capturing at the end would silently drop changes
   written during the cycle from every future sync.
3. `report(Collecting, 0)`; `get_changes_since(last_sync)` — collect local changes
   other devices haven't seen.
4. `report(Sending, local.len())`; `send_changes(local)` — push to the remote peer
   (WebSocket journal for `DbBackend`; a no-op for `FsBackend`, which relies on
   Syncthing replicating its log files).
5. `report(Receiving, 0)`; `receive_changes()` — pull everything the remote
   accumulated since the last pull.
6. `report(Applying, remote.len())`; `apply_change(change)` for each remote change
   **in arrival order** — every implementation is idempotent, so re-running after a
   partial failure is safe. Then `update_sync_time(sync_ts)` persists the
   start-of-cycle watermark.

`count` is the number of changes relevant to the stage (local for `Sending`, remote
for `Applying`/`Done`, `0` otherwise). Returns the remote changes applied this
cycle. Errors as `SyncError::Storage` if any storage call fails; a failed cycle
leaves the last-sync timestamp unchanged, so the next cycle re-collects and
re-applies everything missed — the caller schedules retries.

**Dependencies** — `StorageBackend`'s
`get_last_sync_time`/`get_changes_since`/`send_changes`/`receive_changes`/
`apply_change`/`update_sync_time`; `models::now` (UTC); `SyncStage`; `tracing`.

**Used by** — `SyncEngine::sync` (no-op callback);
`keeplin-daemon/src/server.rs` and `rest.rs` (progress-reporting callers);
`tests/ws_sync.rs`
(`failed_send_keeps_watermark_and_changes_are_resent_after_recovery` asserts the
watermark invariant).

**Repeated context** — Watermark invariant (restated because it is the file's core
guarantee): the last-sync timestamp advances **only after a fully successful
cycle**, and always to the time the cycle *started* — never a fresh `now()` at the
end. Idempotent `apply_change` everywhere is the project convention that makes
at-least-once delivery safe.

---

## SyncEngine

**Identification** — `pub struct SyncEngine<T: StorageBackend>` with a single
`pub backend: T` field; marker `// md:SyncEngine`.

**Code** — complete and verbatim:

```rust
// md:SyncEngine
pub struct SyncEngine<T: StorageBackend> {
    pub backend: T,
}
```

**What it does** — Orchestrates a single synchronisation cycle for any
`StorageBackend`. Generic over `T`, so the compiler produces a monomorphised,
zero-cost implementation per concrete backend — no runtime dispatch, no
`Box<dyn StorageBackend>`. The `backend` field is `pub` so callers can perform CRUD
directly between sync cycles without going through the engine.

**Dependencies** — `StorageBackend`.

**Used by** — the daemon (holds one per configured backend); tests.

**Repeated context** — none.

---

## impl SyncEngine

**Identification** — `impl<T: StorageBackend> SyncEngine<T>`; marker
`// md:impl SyncEngine`. Two methods, each with its own marker below.

**Code** — container: members documented as sub-blocks below: fn new, fn sync.

**What it does** — Constructor and the cycle entry point.

**Dependencies / Used by / Repeated context** — per method.

### fn new

**Identification** — `pub fn new(backend: T) -> Self`; marker
`// md:impl SyncEngine > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncEngine > fn new
    pub fn new(backend: T) -> Self {
        Self { backend }
    }
```

**What it does** — Wraps `backend` in a new engine. No validation, no I/O.

**Dependencies** — none.

**Used by** — daemon startup and tests constructing engines.

**Repeated context** — none.

### fn sync

**Identification** — `pub async fn sync(&self) -> Result<Vec<Change>, SyncError>`;
marker `// md:impl SyncEngine > fn sync`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncEngine > fn sync
    pub async fn sync(&self) -> Result<Vec<Change>, SyncError> {
        run_sync(&self.backend, |_, _| {}).await
    }
```

**What it does** — Runs one complete push-then-pull cycle: a thin wrapper over
`run_sync(&self.backend, |_, _| {})` with a no-op progress callback. Same return
value (remote changes applied) and same error behaviour (failed cycle leaves the
watermark unchanged; no built-in retry — callers re-invoke periodically or on
reconnect).

**Dependencies** — `run_sync`.

**Used by** — `keeplin-daemon/src/rest.rs` and `server.rs`; `tests/ws_sync.rs`.

**Repeated context** — none beyond `run_sync`'s.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

<!-- Data source: CI artifact or local graphify-out/graph.json from this exact commit.
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. Never
     present inference as fact. -->
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

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `enum SyncStage` | `// md:SyncStage` |
| 3 | `fn run_sync` | `// md:fn run_sync` |
| 4 | `struct SyncEngine` | `// md:SyncEngine` |
| 5 | `impl SyncEngine` | `// md:impl SyncEngine` |
| 6 | `fn new` | `// md:impl SyncEngine > fn new` |
| 7 | `fn sync` | `// md:impl SyncEngine > fn sync` |
