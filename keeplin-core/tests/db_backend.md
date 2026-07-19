# `tests/db_backend.rs` — DbBackend integration tests

Self-contained companion for `keeplin-core/tests/db_backend.rs`. It documents
**every code block of the source file, in source order** — a reader with only this
file must be able to understand it without opening anything else, so project-wide
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
use keeplin_core::{error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
    storage::{db::DbBackend, NoteRepository, NotebookRepository,
              ResourceRepository, SyncBackend, TagRepository}};
use tempfile::tempdir;
```

**What it does** — Integration tests for `DbBackend`, the LibSQL-backed storage
implementation. Every test uses `in_memory_backend` (a temp-file database with
an empty `server_url`, so no WebSocket is attempted) and covers the complete
`StorageBackend` API at the SQLite level: CRUD for all entities, soft-deletion
semantics, the `entity_changes` change journal (including the
no-re-journal-on-apply invariant and pruning), device-ID persistence, keyset
pagination, backlink indexing, ordering/starring fields, tombstone-first
races, and write concurrency. WebSocket sync paths live in `ws_sync.rs`.

**Repeated context** — soft delete everywhere: `delete_*` stamps `deleted_at`
(a tombstone that still reads back but is excluded from listings); reading a
missing entity is `StorageError::NotFound`. The journal records the **original
operation type** (create vs update). Keyset pagination orders by
`(created_at, id)`.

---

## fn in_memory_backend

**Identification** — `async fn in_memory_backend() -> DbBackend`. Marker
`// md:fn in_memory_backend`.

**What it does** — A `DbBackend` on a temp-file database with empty
`server_url`/`auth_token` (offline mode — no WebSocket). The tempdir is leaked
with `std::mem::forget` so the directory outlives the open database file; the
OS cleans it up at process exit.

**Used by** — every test except `device_id_is_stable`.

---

## fn create_and_read_note

**Identification** — `#[tokio::test]`. Marker `// md:fn create_and_read_note`.

**What it does** — Basic note round-trip: create, read back title and body.

---

## fn update_note

**Identification** — `#[tokio::test]`. Marker `// md:fn update_note`.

**What it does** — Update persists the new title.

---

## fn delete_note_soft_deletes

**Identification** — `#[tokio::test]`. Marker `// md:fn delete_note_soft_deletes`.

**What it does** — A deleted note disappears from `list_notes`.

---

## fn list_notes_excludes_deleted

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_notes_excludes_deleted`.

**What it does** — Of two notes, deleting one leaves exactly the survivor in
the listing.

---

## fn read_nonexistent_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn read_nonexistent_returns_not_found`.

**What it does** — Reading a random UUID → `StorageError::NotFound`.

---

## fn device_id_is_stable

**Identification** — `#[tokio::test]`. Marker `// md:fn device_id_is_stable`.

**What it does** — Two `DbBackend` openings of the same `.db` file return the
same persisted device id.

---

## fn sync_state_round_trips

**Identification** — `#[tokio::test]`. Marker `// md:fn sync_state_round_trips`.

**What it does** — `update_sync_time`/`get_last_sync_time` round-trip at
second precision.

---

## fn get_changes_since_returns_updated_notes

**Identification** — `#[tokio::test]`. Marker
`// md:fn get_changes_since_returns_updated_notes`.

**What it does** — A note created after `since` appears in the change list as
`Change::NoteCreate`, not `NoteUpdate` — the `entity_changes` journal records
the original operation type.

---

## fn prune_change_journal_removes_rows_older_than_cutoff

**Identification** — `#[tokio::test]`. Marker
`// md:fn prune_change_journal_removes_rows_older_than_cutoff`.

**What it does** — With two journaled creates: a cutoff in the past removes 0
rows (journal untouched); a cutoff in the future removes both and reports the
count, leaving the journal empty.

---

## fn apply_change_is_not_re_journaled

**Identification** — `#[tokio::test]`. Marker
`// md:fn apply_change_is_not_re_journaled`.

**What it does** — The journal holds only changes that **originated on this
device**: a `NoteCreate` ingested via `apply_change` (a remote change from the
relay) is applied to the tables (readable) but never enters the journal — so it
is never re-sent to the relay — while a locally created note does. Pins the
invariant documented on `DbBackend::apply_change`.

---

## fn update_nonexistent_note_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn update_nonexistent_note_returns_not_found`.

**What it does** — Error path: update of an unknown note → `NotFound`.

---

## fn delete_nonexistent_note_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn delete_nonexistent_note_returns_not_found`.

**What it does** — Delete of an unknown note → `NotFound`.

---

## fn update_nonexistent_notebook_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn update_nonexistent_notebook_returns_not_found`.

**What it does** — Update of an unknown notebook → `NotFound`.

---

## fn delete_nonexistent_notebook_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn delete_nonexistent_notebook_returns_not_found`.

**What it does** — Delete of an unknown notebook → `NotFound`.

---

## fn update_nonexistent_tag_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn update_nonexistent_tag_returns_not_found`.

**What it does** — Update of an unknown tag → `NotFound`.

---

## fn delete_nonexistent_tag_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn delete_nonexistent_tag_returns_not_found`.

**What it does** — Delete of an unknown tag → `NotFound`.

---

## fn create_and_read_notebook

**Identification** — `#[tokio::test]`. Marker `// md:fn create_and_read_notebook`.

**What it does** — Notebook round-trip; `deleted_at` starts `None`.

---

## fn delete_notebook_soft_deletes

**Identification** — `#[tokio::test]`. Marker
`// md:fn delete_notebook_soft_deletes`.

**What it does** — A deleted notebook leaves the listing but a direct read
still returns the tombstone with `deleted_at` set.

---

## fn create_and_read_tag

**Identification** — `#[tokio::test]`. Marker `// md:fn create_and_read_tag`.

**What it does** — Tag round-trip.

---

## fn add_and_list_note_tags

**Identification** — `#[tokio::test]`. Marker `// md:fn add_and_list_note_tags`.

**What it does** — Attach a tag; `list_note_tags` returns it.

---

## fn add_note_tag_rejects_missing_or_deleted_ends

**Identification** — `#[tokio::test]`. Marker
`// md:fn add_note_tag_rejects_missing_or_deleted_ends`.

**What it does** — `add_note_tag` with a nonexistent note or tag id, or a
soft-deleted note, fails with `NotFound` — no dangling association is created
(the listing stays empty).

---

## fn pagination_walks_notes_sharing_a_created_at

**Identification** — `#[tokio::test]`. Marker
`// md:fn pagination_walks_notes_sharing_a_created_at`.

**What it does** — Keyset pagination visits every row exactly once even when
three rows share one `created_at` — the case relying on the cursor's
`created_at = ?` equality branch, which in turn relies on the fixed-precision
timestamp format. Walked page size 1; order is `(created_at, id)`.

---

## fn remove_note_tag

**Identification** — `#[tokio::test]`. Marker `// md:fn remove_note_tag`.

**What it does** — Detach after attach; the listing is empty again.

---

## fn purge_reclaims_old_tombstoned_payloads_only

**Identification** — `#[tokio::test]`. Marker
`// md:fn purge_reclaims_old_tombstoned_payloads_only`.

**What it does** — `purge_deleted_resources`: a cutoff before the tombstone
purges nothing; one after it frees exactly the dead payload (count 1); the call
is idempotent (second run counts 0); the tombstone still reads as `NotFound`
and the live resource's bytes are untouched.

---

## fn create_and_read_resource

**Identification** — `#[tokio::test]`. Marker `// md:fn create_and_read_resource`.

**What it does** — Resource metadata + bytes round-trip.

---

## fn list_resources_excludes_data

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_resources_excludes_data`.

**What it does** — Three resources list as metadata (no payloads inline).

---

## fn delete_resource

**Identification** — `#[tokio::test]`. Marker `// md:fn delete_resource`.

**What it does** — A deleted resource reads back as `NotFound`.

---

## fn list_notes_paginates_without_duplicates_or_gaps

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_notes_paginates_without_duplicates_or_gaps`.

**What it does** — 25 notes walked with page size 10: no page exceeds the
size, every note appears exactly once, and the paged order equals the
single-shot order (stable keyset `created_at ASC, id ASC`).

---

## fn concurrent_note_creates_all_succeed

**Identification** — `#[tokio::test(flavor = "multi_thread", worker_threads =
4)]`. Marker `// md:fn concurrent_note_creates_all_succeed`.

**What it does** — 50 concurrent `create_note` tasks all commit. `DbBackend`
wraps every mutation in `BEGIN IMMEDIATE … COMMIT` on a single shared
connection, so without serialisation a second `BEGIN` before the first `COMMIT`
would fail ("cannot start a transaction within a transaction"). All 50 notes
are queryable afterwards.

---

## fn concurrent_reads_and_writes_make_progress

**Identification** — `#[tokio::test(flavor = "multi_thread", worker_threads =
4)]`. Marker `// md:fn concurrent_reads_and_writes_make_progress`.

**What it does** — 20 writers interleaved with 20 readers (point reads + list
reads) all complete — the read/write guard around the shared connection must
never deadlock (a reader must not block a reader; the two sides are never
acquired re-entrantly by one task). Final count is seed + 20.

---

## fn note_alias_bookmarks_links_round_trip

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_alias_bookmarks_links_round_trip`.

**What it does** — Alias, bookmarks, and links persist through the SQLite
columns: create preserves the content fields verbatim while stamping `vv` and
`last_writer` (asserted non-empty); read-back matches; editing the alias and a
bookmark alias persists.

---

## fn notebook_alias_round_trip

**Identification** — `#[tokio::test]`. Marker `// md:fn notebook_alias_round_trip`.

**What it does** — A notebook alias survives read and listing.

---

## fn indexed_backlinks_track_writes_and_deletes

**Identification** — `#[tokio::test]`. Marker
`// md:fn indexed_backlinks_track_writes_and_deletes`.

**What it does** — The backlink index: two sources linking to a target both
appear (an unrelated note does not); clearing src1's links via update drops it
from the index; soft-deleting src2 excludes it too (the JOIN filters deleted
sources) — backlinks end empty.

---

## fn backlinks_are_paginated

**Identification** — `#[tokio::test]`. Marker `// md:fn backlinks_are_paginated`.

**What it does** — Three backlinks walked with page size 2: pages of 2 and 1,
no third page, and the union covers all three sources without overlap.

---

## fn ordering_fields_round_trip_and_manual_order_query

**Identification** — `#[tokio::test]`. Marker
`// md:fn ordering_fields_round_trip_and_manual_order_query`.

**What it does** — Issues #49–#52: `is_pinned`/`sort_key` round-trip; the
manual order query returns pinned band first, then the legacy `sort_key 0`
sentinel (ordering as the effective `NORMAL_START = 1000`), then 1500 — and
cursor pagination walks the identical order one note at a time.
`list_starred_notes` spans notebooks and excludes everything unstarred (the
starred note lives in the Inbox). `notebook_sort_profile` summarises the
notebook for placement (pinned keys, min key, max normal key).

---

## fn sync_applied_change_carries_ordering_fields

**Identification** — `#[tokio::test]`. Marker
`// md:fn sync_applied_change_carries_ordering_fields`.

**What it does** — Issue #55: an `apply_change`-ingested note carries
`is_pinned`/`is_starred`/`sort_key` intact (whole-note version-vector
resolution treats them like any other field), and sync-applied stars are
queryable via `list_starred_notes`.

---

## fn delete_for_unknown_entity_leaves_a_tombstone_blocking_a_stale_create

**Identification** — `#[tokio::test]`. Marker
`// md:fn delete_for_unknown_entity_leaves_a_tombstone_blocking_a_stale_create`.

**What it does** — Issue #71, for all four versioned entity types (note,
notebook, tag, resource): a delete arriving for an entity this backend has
never seen (peer vv `{peer:2}`) inserts a minimal tombstone (each `apply_change`
arm does this when the `UPDATE` hits no row), so the causally older create
(vv `{peer:1}`) that arrives afterwards loses against the stored tombstone —
nothing is resurrected, listings stay empty, and the note's tombstone reads
back with `deleted_at` set.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `in_memory_backend()` — defined here (EXTRACTED; the shared fixture)
- the 37 `#[tokio::test]` functions — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: references; e.g. `DbBackend`)
- `keeplin-core/src/models.rs` — entities and `Change` (EXTRACTED: references)
- `keeplin-core/src/error.rs` — `StorageError` (EXTRACTED: references)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- The full `StorageBackend` surface of `DbBackend` is covered at the SQLite level; WebSocket paths belong to `ws_sync.rs`.
- The journal invariants (original op type recorded; `apply_change` never re-journals; prune respects the cutoff) are pinned here.
- Tombstone-first races (#71) must stay covered for all four versioned entity types.
- Concurrency tests must run on a multi-threaded runtime to exercise real interleavings.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | crate doc + imports | `// md:Overview` |
| 2 | `fn in_memory_backend` | `// md:fn in_memory_backend` |
| 3–39 | the 37 test fns, in source order | `// md:fn <name>` |
