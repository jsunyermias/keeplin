# `tests/fs_backend.rs` — FsBackend integration tests

Self-contained companion for `keeplin-core/tests/fs_backend.rs`. It documents
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
use chrono::Utc;
use keeplin_core::{error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
    storage::{fs::FsBackend, NoteRepository, NotebookRepository,
              ResourceRepository, SyncBackend, TagRepository}};
use tempfile::tempdir;
```

**What it does** — Integration tests for `FsBackend`, the filesystem-backed
storage implementation. Every test creates a fresh tempdir, roots a new
`FsBackend` there, and exercises the full `StorageBackend` API against real
files on disk: CRUD happy paths and error paths (`NotFound`), soft-deletion,
device-id persistence, keyset pagination, the **version-vector note model**
(three-file layout, per-device note logs, Syncthing-style log replication,
causal and concurrent convergence, log compaction), the **global NDJSON
journal** with generation-epoch snapshot compaction, the in-memory note index,
ordering/starring fields, and tombstone-first races (#71).

**Repeated context** — FsBackend layout: each note lives in `notes/{id}/` as
`note.md` (body), `meta.msgpack` (metadata projection), and one
`log.{device}.msgpack` per device (the **single-writer source of truth**);
global entities journal to `logs/{device}.log` (NDJSON). Replication copies
**only** the single-writer logs — projections are per-device caches regenerated
on sync. Conflicts resolve through `note_log::resolve` with the
`(timestamp, device_id)` tiebreak.

---

## fn create_and_read_note

**Identification** — `#[tokio::test]`. Marker `// md:fn create_and_read_note`.

**What it does** — Note round-trip: create returns the same id; read returns
title and body.

---

## fn update_note

**Identification** — `#[tokio::test]`. Marker `// md:fn update_note`.

**What it does** — Update returns and persists the new title.

---

## fn delete_note_soft_deletes

**Identification** — `#[tokio::test]`. Marker `// md:fn delete_note_soft_deletes`.

**What it does** — A deleted note disappears from `list_notes`.

---

## fn list_notes_excludes_deleted

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_notes_excludes_deleted`.

**What it does** — Of two notes, deleting one leaves only the survivor listed.

---

## fn read_nonexistent_note_returns_not_found

**Identification** — `#[tokio::test]`. Marker
`// md:fn read_nonexistent_note_returns_not_found`.

**What it does** — Reading a random UUID → `StorageError::NotFound`.

---

## fn device_id_is_stable_across_instances

**Identification** — `#[tokio::test]`. Marker
`// md:fn device_id_is_stable_across_instances`.

**What it does** — Two `FsBackend`s over the same root return the same
persisted device id (`.keeplin/device_id`).

---

## fn sync_state_persists

**Identification** — `#[tokio::test]`. Marker `// md:fn sync_state_persists`.

**What it does** — `update_sync_time`/`get_last_sync_time` round-trip. The
timestamp is serialised as RFC-3339, which may lose sub-second precision, so
the comparison is at second granularity.

---

## fn get_changes_since_scans_other_device_logs

**Identification** — `#[tokio::test]`. Marker
`// md:fn get_changes_since_scans_other_device_logs`.

**What it does** — Simulates a log file written by a **different** device and
replicated into `logs/` by Syncthing (its name differs from this device's own
log, so it is not skipped): `get_changes_since` parses it and yields the
`Change::NoteCreate`.

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

## fn list_notebooks_includes_created

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_notebooks_includes_created`.

**What it does** — Regression: the sidecar is written as `{id}.msgpack`, so the
listing must filter on that extension — a previous `.json` filter matched
nothing and returned an empty list.

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

## fn list_tags_includes_created

**Identification** — `#[tokio::test]`. Marker `// md:fn list_tags_includes_created`.

**What it does** — Regression: the same `.msgpack`-vs-`.json` listing bug as
notebooks, for tags.

---

## fn add_and_list_note_tags

**Identification** — `#[tokio::test]`. Marker `// md:fn add_and_list_note_tags`.

**What it does** — Attach a tag; `list_note_tags` returns it.

---

## fn add_note_tag_rejects_missing_or_deleted_ends

**Identification** — `#[tokio::test]`. Marker
`// md:fn add_note_tag_rejects_missing_or_deleted_ends`.

**What it does** — `add_note_tag` with a nonexistent note or tag, or a
soft-deleted tag, fails with `NotFound` — no dangling association is created.

---

## fn remove_note_tag

**Identification** — `#[tokio::test]`. Marker `// md:fn remove_note_tag`.

**What it does** — Detach after attach; the listing is empty again.

---

## fn create_and_read_resource

**Identification** — `#[tokio::test]`. Marker `// md:fn create_and_read_resource`.

**What it does** — Resource metadata + bytes round-trip.

---

## fn list_resources_excludes_data

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_resources_excludes_data`.

**What it does** — Three resources list as metadata only.

---

## fn delete_resource

**Identification** — `#[tokio::test]`. Marker `// md:fn delete_resource`.

**What it does** — A deleted resource reads back as `NotFound`.

---

## fn list_notes_paginates_without_duplicates_or_gaps

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_notes_paginates_without_duplicates_or_gaps`.

**What it does** — 23 notes walked with page size 7: no page exceeds the size,
every note appears exactly once, and the paged order equals the single-shot
order.

---

## fn replicate_note

**Identification** — `async fn replicate_note(from_root, to_root, id)`. Marker
`// md:fn replicate_note`.

**What it does** — Simulates Syncthing replicating one note between roots by
copying **only** its per-device `log.*.msgpack` files (the single-writer
source of truth) — never the local projections.

**Used by** — the two-device note tests and the note-log compaction test.

---

## fn fs_note_uses_three_file_layout

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_note_uses_three_file_layout`.

**What it does** — After a create, `notes/{id}/` contains `note.md`,
`meta.msgpack`, and a per-device `log.*.msgpack`; the markdown body is stored
verbatim (unencrypted backend).

---

## fn fs_two_device_causal_sync

**Identification** — `#[tokio::test]`. Marker `// md:fn fs_two_device_causal_sync`.

**What it does** — A creates; the log replicates A→B and B reads the note. B
then edits **causally** (having seen A's version) and replicates back: the
causal edit wins on A with no conflict.

---

## fn fs_two_device_concurrent_edits_converge

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_two_device_concurrent_edits_converge`.

**What it does** — Concurrent edits with no exchange between them, then
cross-replication of both logs: both devices converge to the **same** winner
(deterministic last-write-wins by timestamp, then device id). This is the
FsBackend counterpart of `sync.rs`'s equal-timestamp DbBackend test —
FsBackend resolves through per-note logs, not wire `Change` records.

---

## fn note_alias_bookmarks_links_persist_in_meta

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_alias_bookmarks_links_persist_in_meta`.

**What it does** — Alias, bookmarks, and a manual link survive the round-trip
through `log.{device}.msgpack` + `meta.msgpack` (reads materialise from the
per-device log); a second backend over the same root (a different "device")
materialises the same state from the replicated log.

---

## fn backlinks_default_scan_is_paginated

**Identification** — `#[tokio::test]`. Marker
`// md:fn backlinks_default_scan_is_paginated`.

**What it does** — Three backlinks walked with page size 2: pages of 2 and 1,
no third page, union covers all three sources without overlap.

---

## fn fs_notebook_concurrent_equal_timestamp_edits_converge

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_notebook_concurrent_equal_timestamp_edits_converge`.

**What it does** — Two `FsBackend` devices edit the same notebook with the
**identical** `updated_at` (exchanged via `apply_change`): version-vector
`resolve` picks one deterministic winner on both sides. Under the old
`updated_at`-only `>` comparison, equal timestamps meant neither device applied
the other's edit — permanent divergence.

---

## fn fs_concurrent_note_tag_add_remove_converges

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_concurrent_note_tag_add_remove_converges`.

**What it does** — Concurrent attach-vs-detach of a note↔tag association
converges on `FsBackend` exactly as on `DbBackend` (the FS mirror of
`sync::db_concurrent_note_tag_add_remove_converges`): after exchanging
replicated logs both ways and draining sync, both devices agree on the
association's final presence.

---

## fn note_log_len

**Identification** — `async fn note_log_len(root, id) -> usize`. Marker
`// md:fn note_log_len`.

**What it does** — Counts the entries in a note's single per-device
`log.*.msgpack` (rmp-serde-decoded `Vec<NoteLogEntry>`); panics if no log
exists.

**Used by** — `fs_note_log_compacts_and_still_converges`.

---

## fn fs_note_log_compacts_and_still_converges

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_note_log_compacts_and_still_converges`.

**What it does** — 1000 edits on one note keep its per-device log bounded to at
most the compaction threshold (+1): each time it grows past 256 entries it is
collapsed back to its frontier, rather than growing to 1001. Reads still return
the latest body; the compacted log replicates to a fresh peer that converges;
and a delete after all that churn still produces a tombstone carrying the
recovered content (the newest upsert was retained by compaction) that
propagates.

---

## fn replicate_logs

**Identification** — `async fn replicate_logs(from, to)`. Marker
`// md:fn replicate_logs`.

**What it does** — Simulates Syncthing replicating one device's **single-writer
log files**: every global `logs/*.log` plus every per-note
`notes/{id}/log.*.msgpack`. Each has a single writer, so this never conflicts.
Projections (`note.md`, `meta.msgpack`) are **not** copied — they are
per-device caches the receiver regenerates from the logs on sync.

**Used by** — the global-log and note-index tests.

---

## fn drain_sync

**Identification** — `async fn drain_sync(b: &FsBackend)`. Marker
`// md:fn drain_sync`.

**What it does** — Pulls (`receive_changes`) and applies every change a device
can currently see from its peers' replicated logs.

---

## fn own_log_stats

**Identification** — `async fn own_log_stats(root, backend) -> (u64, usize)`.
Marker `// md:fn own_log_stats`.

**What it does** — Parses a device's own global `logs/{device}.log` text
directly (no `FsBackend` internals): returns the generation epoch (from the
`__keeplin_epoch__` header line) and the count of change entries.

**Used by** — the two global-log compaction tests.

---

## fn fs_global_log_compacts_and_peer_converges

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_global_log_compacts_and_peer_converges`.

**What it does** — The global NDJSON journal is bounded by generation-epoch
snapshot compaction: after a synced two-notebook baseline, churning one
notebook 600 times (past the 512 threshold) and deleting the other leaves A's
log with an epoch ≥ 1 and far fewer entries than the ~601 mutations (each
entity collapsed to one snapshot entry). Peer B — which synced only the
epoch-0 baseline — detects the new generation, re-reads the snapshot, and
converges: the live notebook at `x600`, the deleted one tombstoned.

---

## fn fs_global_log_snapshot_covers_all_entity_types

**Identification** — `#[tokio::test]`. Marker
`// md:fn fs_global_log_snapshot_covers_all_entity_types`.

**What it does** — The compaction snapshot covers **every** globally-journalled
entity type: after forcing a compaction, a brand-new peer that receives only
the post-compaction snapshot still reconstructs the notebook (latest title),
the tag, the resource, and the note↔tag association.

---

## fn ordering_fields_round_trip_and_manual_order_query

**Identification** — `#[tokio::test]`. Marker
`// md:fn ordering_fields_round_trip_and_manual_order_query`.

**What it does** — Issues #49–#52 on `FsBackend`: `is_pinned`/`sort_key`
round-trip; single-note-page cursor pagination walks pinned band first, then
the legacy `sort_key 0` sentinel (effective 1000), then 1500;
`list_starred_notes` returns only the starred Inbox note;
`notebook_sort_profile` reports pinned keys and max normal key.

---

## fn note_index_reflects_local_writes_after_it_is_built

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_index_reflects_local_writes_after_it_is_built`.

**What it does** — The in-memory note index is built lazily on the first
listing, then maintained in place: a create after the build appears
(incremental insert) and a delete disappears (incremental remove) — no listing
re-reads every note's logs.

---

## fn note_index_reflects_changes_pulled_from_a_peer

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_index_reflects_changes_pulled_from_a_peer`.

**What it does** — With B's index warmed while empty, a peer note (starred)
replicated and drained through a sync cycle flows through the same
`persist_note_projection` choke point, so it appears in both `list_notes` and
`list_starred_notes`.

---

## fn delete_for_unknown_sidecar_entity_leaves_a_tombstone_blocking_a_stale_create

**Identification** — `#[tokio::test]`. Marker
`// md:fn delete_for_unknown_sidecar_entity_leaves_a_tombstone_blocking_a_stale_create`.

**What it does** — Issue #71 on `FsBackend`, for the sidecar entity types
(notebook, tag, resource): a delete arriving for an unknown entity writes a
minimal tombstone sidecar, so the causally older create (peer vv `{peer:1}` vs
the delete's `{peer:2}`) loses in `resolve` instead of resurrecting the
entity. Note deletes converge through the Syncthing-replicated per-note logs,
not this apply path — covered by the two-device convergence tests.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `replicate_note()`, `replicate_logs()`, `drain_sync()`, `note_log_len()`, `own_log_stats()` — helpers defined here (EXTRACTED; file-local)
- the 40 `#[tokio::test]` functions — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: references; e.g. `FsBackend`)
- `keeplin-core/src/storage/note_log.rs` — `NoteLogEntry` and the resolve/merge semantics exercised via replication (EXTRACTED: references)
- `keeplin-core/src/models.rs` — entities and `Change` (EXTRACTED: references)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Replication copies only single-writer logs; projections are regenerated — the tests must never copy `note.md`/`meta.msgpack` between roots.
- Convergence (causal, concurrent equal-timestamp, add/remove races) and both compaction bounds (per-note 256, global 512 with epoch snapshots) are pinned here.
- Tombstone-first races (#71) stay covered for the sidecar entity types; note deletes converge through the per-note logs.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | crate doc + imports | `// md:Overview` |
| 2–27 | CRUD, error-path, listing, pagination tests | `// md:fn <name>` |
| 28 | `fn replicate_note` (helper) | `// md:fn replicate_note` |
| 29–35 | note-model and convergence tests | `// md:fn <name>` |
| 36 | `fn note_log_len` (helper) | `// md:fn note_log_len` |
| 37 | `fn fs_note_log_compacts_and_still_converges` | `// md:fn fs_note_log_compacts_and_still_converges` |
| 38–40 | `fn replicate_logs`, `fn drain_sync`, `fn own_log_stats` (helpers) | `// md:fn <name>` |
| 41–46 | global-log, ordering, index, tombstone tests | `// md:fn <name>` |
