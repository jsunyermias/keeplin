# `tests/sync.rs` — cross-device change-propagation tests

Self-contained companion for `keeplin-core/tests/sync.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file
must be able to understand it without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Block header>`; grep it in either direction. Each section covers
**Identification**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the crate doc and the imports. Marker
`// md:Overview`.

```rust
use chrono::{Duration, Utc};
use keeplin_core::{error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
    storage::{db::DbBackend, fs::FsBackend, NoteRepository, NotebookRepository,
              ResourceRepository, SyncBackend, TagRepository}};
use tempfile::tempdir;
```

**What it does** — Integration tests modelling **two independent devices**, each
backed by its own `DbBackend` database file, verifying that changes recorded on
one device can be collected with `SyncBackend::get_changes_since` and replayed
on the other with `SyncBackend::apply_change` to reach a convergent state.
Version-vector conflict semantics are pinned here — including the concurrent
equal-timestamp case that bare `updated_at` last-write-wins would diverge on —
plus the tombstone rules that stop a stale edit from resurrecting a delete (and
vice versa). A couple of cases run on `FsBackend` to confirm it resolves the
same way through the shared `note_log::resolve`/`merge` primitives.

**Repeated context** — conflict resolution is unified on version vectors: the
`(timestamp, device_id)` pair breaks ties deterministically, so both sides of a
concurrent edit pick the same winner. `apply_change` does not re-journal, so
each device's journal holds only its own local edits — the exchange loops in
these tests rely on that. `FsBackend` resolves note conflicts through per-note
version-vector logs rather than wire `Change::NoteUpdate` records, so its LWW
equivalent lives in `fs_two_device_concurrent_edits_converge` in
`tests/fs_backend.rs`, not here.

---

## fn device

**Identification** — `async fn device() -> DbBackend`. Marker `// md:fn device`.

**What it does** — A standalone offline `DbBackend` on a temp `.db` file (empty
server URL/token → no WebSocket). The temp dir is leaked (`std::mem::forget`) so
it outlives the open connection for the test's duration.

**Used by** — every `db_*` test and the first two.

---

## fn create_propagates_between_devices

**Identification** — `#[tokio::test]`. Marker
`// md:fn create_propagates_between_devices`.

**What it does** — A creates a note; collecting all of A's changes since epoch
and applying them on B makes B read the same title/body back.

---

## fn stale_remote_update_does_not_clobber_newer_local

**Identification** — `#[tokio::test]`. Marker
`// md:fn stale_remote_update_does_not_clobber_newer_local`.

**What it does** — Applying a remote `Change::NoteUpdate` whose `updated_at` is
a minute older than the local record is a no-op: the newer local body survives
(last-write-wins by timestamp).

---

## fn db_stale_delete_does_not_override_newer_edit

**Identification** — `#[tokio::test]`. Marker
`// md:fn db_stale_delete_does_not_override_newer_edit`.

**What it does** — A `Change::NoteDelete` dated a minute before the local edit
must not tombstone the newer note (`deleted_at` stays `None`).

---

## fn db_stale_update_does_not_resurrect_tombstone

**Identification** — `#[tokio::test]`. Marker
`// md:fn db_stale_update_does_not_resurrect_tombstone`.

**What it does** — After a local delete (tombstone dated "now"), a stale peer
update from before the delete must not revive the note (`deleted_at` stays
set).

---

## fn db_concurrent_equal_timestamp_edits_converge

**Identification** — `#[tokio::test]`. Marker
`// md:fn db_concurrent_equal_timestamp_edits_converge`.

**What it does** — The divergence case bare LWW cannot handle: from a shared
baseline (created on A, replicated to B), both devices edit the same note with
the **identical** `updated_at`, then exchange changes both ways. Under strict
`>` comparison each device would keep its own edit — permanent divergence. The
version vector's `(timestamp, device_id)` tiebreak makes both devices pick the
same body (either "from A" or "from B", but the **same** on both).

---

## fn db_concurrent_notebook_edits_converge

**Identification** — `#[tokio::test]`. Marker
`// md:fn db_concurrent_notebook_edits_converge`.

**What it does** — The same equal-timestamp convergence for **notebooks**,
exercising the `DbBackend` notebook `apply_change` arm specifically: concurrent
same-`updated_at` title edits converge to one deterministic winner.

---

## fn fs_tombstones_resolve_by_timestamp

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_tombstones_resolve_by_timestamp`.

**What it does** — Tombstone semantics hold on `FsBackend` too: (a) a stale
`NoteDelete` (a minute older than the note) does not tombstone it; (b) a stale
`NoteUpdate` cannot resurrect a newer local delete.

---

## fn db_concurrent_note_tag_add_remove_converges

**Identification** — `#[tokio::test]`. Marker
`// md:fn db_concurrent_note_tag_add_remove_converges`.

**What it does** — From a shared baseline with the tag attached, A detaches
while B re-attaches concurrently; after exchanging changes both ways, both
devices agree on the association's final presence. Before Phase 3, associations
carried no version (add = INSERT OR IGNORE, remove = DELETE), so the outcome
was order-dependent and could differ between devices.

---

## fn db_resource_delete_propagates_and_converges

**Identification** — `#[tokio::test]`. Marker
`// md:fn db_resource_delete_propagates_and_converges`.

**What it does** — A resource create syncs A→B; the origin then soft-deletes
(a versioned tombstone) and the tombstone propagates: both devices read
`StorageError::NotFound` and exclude the resource from listings — instead of
the old order-dependent hard delete.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `device()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `create_propagates_between_devices()` — defined here (EXTRACTED; file-local)
- `stale_remote_update_does_not_clobber_newer_local()` — defined here (EXTRACTED; file-local)
- `db_stale_delete_does_not_override_newer_edit()` — defined here (EXTRACTED; file-local)
- `db_stale_update_does_not_resurrect_tombstone()` — defined here (EXTRACTED; file-local)
- `db_concurrent_equal_timestamp_edits_converge()` — defined here (EXTRACTED; file-local)
- `db_concurrent_notebook_edits_converge()` — defined here (EXTRACTED; file-local)
- `fs_tombstones_resolve_by_timestamp()` — defined here (EXTRACTED; file-local)
- `db_concurrent_note_tag_add_remove_converges()` — defined here (EXTRACTED; file-local)
- `db_resource_delete_propagates_and_converges()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: references×1; e.g. `DbBackend`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Two independent backends converge by exchanging `Change`s via `get_changes_since`/`apply_change` — version-vector semantics (dominance, concurrency tiebreak) are pinned here.
- Convergence must be order-independent: applying the same change sets in different orders yields the same final state.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | crate doc + imports | `// md:Overview` |
| 2 | `fn device` | `// md:fn device` |
| 3–11 | the nine `#[tokio::test]` fns | `// md:fn <name>` |
