# `event_backend.rs` — `EventBackend` change-publishing decorator

Self-contained companion for `keeplin-daemon/src/event_backend.rs`. It documents
**every code block of the source file, in source order** — a reader with only this
file must be able to understand it without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

```rust
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::broadcast;
use uuid::Uuid;

use keeplin_core::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
    storage::{NoteRepository, NotebookRepository, ResourceRepository, SyncBackend, TagRepository},
};
```

**What it does** — `EventBackend<B>` wraps any `B: StorageBackend` and, after
every **successful** mutation, publishes the corresponding `Change` to a
`tokio::sync::broadcast` channel — the live WebSocket feed. Reads delegate
unchanged. Because it is itself a `StorageBackend`, one instance sits behind
**both** the gRPC service and the REST API, so a mutation from either surface
emits exactly one event. Placement: **outside** any `EncryptedBackend`
(`EventBackend<EncryptedBackend<Fs|Db>>`) — it publishes the value *returned*
by the inner backend, already decrypted, so subscribers receive plaintext; the
daemon is the trust boundary (at-rest encryption protects the disk, not
connected clients). Delivery is **lossy, best-effort**: a lagging subscriber
sees `Lagged` rather than blocking writers; the feed is a notification stream,
not a durable log — the authoritative history is the sync change journal.
Publishing never blocks a mutation.

**Dependencies** — `tokio::sync::broadcast`, `async_trait`, `chrono`, `uuid`,
keeplin-core's error/model/storage-trait types.

**Used by** — the daemon's stack assembly (`main.rs`), with a `tx` clone in the
REST `AppState` from which each WebSocket connection derives its receiver.

**Repeated context** — decorator conventions restated: implement every
sub-trait, delegate defaulted trait methods (`note_backlinks`) explicitly so
inner indexes are reached, publish only after the inner call succeeds.

---

## EventBackend

**Identification** — `pub struct EventBackend<B>`; marker `// md:EventBackend`.

**What it does** — `inner: B` (persists first; events only after success) and
`tx: broadcast::Sender<Change>` (the daemon keeps another clone in the REST
`AppState`).

**Dependencies** — `broadcast`. **Used by** — `main.rs`.
**Repeated context** — none.

---

## impl EventBackend

**Identification** — inherent impl; marker `// md:impl EventBackend`. Two
methods.

### fn new

**Identification** — `pub fn new(inner: B, tx: broadcast::Sender<Change>) -> Self`;
marker `// md:impl EventBackend > fn new`.

**What it does** — Wraps `inner`. `tx` is created once in `main`
(`broadcast::channel(capacity)`); pass a clone here and keep another for the
WebSocket route's `tx.subscribe()` calls.

### fn publish

**Identification** — `fn publish(&self, change: Change)`; marker
`// md:impl EventBackend > fn publish`.

**What it does** — Sends one change, discarding the only possible error —
"no active receivers", the normal state when no WebSocket client is
connected.

---

## impl NoteRepository for EventBackend

**Identification** — marker `// md:impl NoteRepository for EventBackend`.

**What it does** — `create_note`/`update_note` delegate then publish
`NoteCreate`/`NoteUpdate` with the **stored** (returned, decrypted) copy;
`delete_note` publishes a `NoteDelete` with empty vv/writer — the feed is a
best-effort notification (clients reload via REST/gRPC), so
conflict-resolution metadata is not needed; `read_note`, the listings,
`note_backlinks` (explicit delegation for inner indexes), and
`notebook_sort_profile` delegate silently.

**Dependencies** — `publish`, the inner backend.

**Used by** — all note traffic in the daemon.

**Repeated context** — publish-after-success: a failed mutation emits nothing.

---

## impl NotebookRepository for EventBackend

**Identification** — marker `// md:impl NotebookRepository for EventBackend`.

**What it does** — Same pattern for notebooks: create/update publish the
stored copy, delete publishes an empty-metadata tombstone, reads/listings
silent.

---

## impl TagRepository for EventBackend

**Identification** — marker `// md:impl TagRepository for EventBackend`.

**What it does** — Same pattern for tags; `add_note_tag`/`remove_note_tag`
publish `NoteTagAdd`/`NoteTagRemove` with fresh timestamps and empty version
metadata (notification only); `list_note_tags` silent.

---

## impl ResourceRepository for EventBackend

**Identification** — marker `// md:impl ResourceRepository for EventBackend`.

**What it does** — `create_resource` publishes `ResourceCreate` with
**`data: None`** — the feed carries metadata only; subscribers fetch bytes via
`GET /api/resources/:id/data` (keeps the channel light, matches `FsBackend`'s
journal); `delete_resource` publishes the tombstone;
`purge_deleted_resources` delegates silently (maintenance — the deletions were
published when they happened); reads/listings silent.

---

## impl SyncBackend for EventBackend

**Identification** — marker `// md:impl SyncBackend for EventBackend`.

**What it does** — All eight methods delegate without publishing: sync moves
changes that were already (or will be) published by the CRUD methods —
emitting here would duplicate events.

---

## impl HistoryRepository for EventBackend

**Identification** — marker `// md:impl HistoryRepository for EventBackend`.

**What it does** — Pure delegation of `note_history`/`notebook_history`.

---

## mod tests

**Identification** — `#[cfg(test)] mod tests`; marker `// md:mod tests`. One
helper + three tests over `EventBackend<FsBackend>`.

**What it does** — Pins publish-on-success, silence on reads, and silence on
failure.

### fn backend

**Identification** — helper; marker `// md:mod tests > fn backend`.

**What it does** — An `EventBackend<FsBackend>` over a leaked tempdir plus the
matching receiver (capacity 16).

### fn create_update_delete_emit_changes

**Identification** — tokio test; marker
`// md:mod tests > fn create_update_delete_emit_changes`.

**What it does** — Create/update/delete each emit their variant, with the
delete carrying the right id.

### fn reads_do_not_emit_changes

**Identification** — tokio test; marker
`// md:mod tests > fn reads_do_not_emit_changes`.

**What it does** — After draining the create event, `read_note` and
`list_notes` leave the channel `Empty`.

### fn failed_mutation_emits_nothing

**Identification** — tokio test; marker
`// md:mod tests > fn failed_mutation_emits_nothing`.

**What it does** — Updating a nonexistent note fails and publishes no event.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `EventBackend<B>` — defined here (EXTRACTED)
- the six trait implementations (implements×6) (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs`, `models.rs`, `storage/backend.rs` (EXTRACTED: references×37/×30/×3)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/main.rs` — stack assembly (INFERRED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `struct EventBackend` | `// md:EventBackend` |
| 3 | `impl EventBackend` (+ `new`, `publish`) | `// md:impl EventBackend` (+ `> fn …`) |
| 4 | `impl NoteRepository for EventBackend` (9 methods) | `// md:impl NoteRepository for EventBackend` |
| 5 | `impl NotebookRepository for EventBackend` (5 methods) | `// md:impl NotebookRepository for EventBackend` |
| 6 | `impl TagRepository for EventBackend` (8 methods) | `// md:impl TagRepository for EventBackend` |
| 7 | `impl ResourceRepository for EventBackend` (5 methods) | `// md:impl ResourceRepository for EventBackend` |
| 8 | `impl SyncBackend for EventBackend` (8 methods) | `// md:impl SyncBackend for EventBackend` |
| 9 | `impl HistoryRepository for EventBackend` (2 methods) | `// md:impl HistoryRepository for EventBackend` |
| 10 | `mod tests` (+ helper + three tests) | `// md:mod tests` (+ `> fn …`) |
