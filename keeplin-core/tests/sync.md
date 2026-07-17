# `tests/sync.rs` — cross-device change-propagation tests

## What is tested

These integration tests model **two independent devices**, each with its own backend, and
verify that a `Change` recorded on one device can be collected with
`SyncBackend::get_changes_since` and replayed on the other with `SyncBackend::apply_change`
until both converge. They pin down the **version-vector** conflict semantics — including the
concurrent equal-timestamp case that a bare `updated_at` last-write-wins diverges on — and the
**tombstone** rules that stop a stale edit from resurrecting a delete (and vice versa).

Most tests use two `DbBackend` instances; a couple use `FsBackend` to confirm it resolves the
same way through the shared `note_log::resolve`/`merge` primitives.

## Test cases

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `create_propagates_between_devices` | Device A creates a note; collect + apply on B | B reads the same note back |
| `stale_remote_update_does_not_clobber_newer_local` | Apply a remote update older than the local `updated_at` | The newer local note is kept |
| `db_stale_delete_does_not_override_newer_edit` | Apply a delete older than a later local edit | The note stays alive (the delete loses) |
| `db_stale_update_does_not_resurrect_tombstone` | Apply an update older than a local tombstone | The note stays deleted (the tombstone wins) |
| `db_concurrent_equal_timestamp_edits_converge` | Two devices edit the same note with the **identical** `updated_at` | Both converge on one deterministic winner (the `(timestamp, device_id)` tiebreak), where bare LWW would diverge |
| `fs_tombstones_resolve_by_timestamp` | Same delete-vs-edit races on `FsBackend` | Resolved deterministically, matching `DbBackend` |
| `db_concurrent_note_tag_add_remove_converges` | One device attaches a tag while another detaches it, concurrently | Both devices agree on the association's final presence |
| `db_resource_delete_propagates_and_converges` | A resource create syncs, then the origin soft-deletes it | Both devices read `NotFound` and exclude it from listings |

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `device()` | Builds a fresh `DbBackend` on a temp `.db` file in offline mode (no server URL) |
| `tempfile::tempdir()` | Unique temp dir removed when the guard drops |

## Graph context

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

## Related files

- `keeplin-core/src/storage/note_log.rs` — `resolve`/`merge`, the shared version-vector decision under test
- `keeplin-core/src/storage/db.rs` — `apply_change` (resolves every entity via `note_log::resolve`)
- `keeplin-core/src/storage/fs.rs` — the filesystem backend used by the `fs_*` cases
- `keeplin-core/tests/ws_sync.rs` — the same propagation over a real WebSocket relay
- `SECURITY.md` — "Conflict resolution is unified on version vectors"
