# `tests/fs_backend.rs` — FsBackend integration tests

## What is tested

This file contains integration tests for `FsBackend`, the filesystem-backed
`StorageBackend` implementation. Every test creates a fresh temporary directory
(via `tempfile::tempdir()`), constructs a new `FsBackend` pointing to it, and exercises
one specific behaviour. The temporary directory is automatically removed when the test
function returns. Tests run against the real filesystem — there is no mocking.

## Test cases

### Notes

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `create_and_read_note` | Create a note, then read it back by ID | Returned note has the same `title` and `body` |
| `update_note` | Create a note, update its `title`, read it back | `updated_at` note reflects new title |
| `delete_note_soft_deletes` | Create a note, delete it, call `list_notes` | Deleted note is absent from the list |
| `list_notes_excludes_deleted` | Create two notes, delete one, list | Only the un-deleted note is returned |
| `read_nonexistent_note_returns_not_found` | Call `read_note` with an unknown UUID | Returns `StorageError::NotFound` |
| `update_nonexistent_note_returns_not_found` | Call `update_note` with a note that was never stored | Returns `StorageError::NotFound` |
| `delete_nonexistent_note_returns_not_found` | Call `delete_note` with an unknown UUID | Returns `StorageError::NotFound` |

### Device and sync state

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `device_id_is_stable_across_instances` | Open the same directory with two separate `FsBackend` instances | Both return the same device ID string |
| `sync_state_persists` | Write a sync timestamp, read it back | Returned timestamp matches at second-level precision |
| `get_changes_since_scans_other_device_logs` | Write a fake log file for a second device, call `get_changes_since(epoch)` | Returns one `Change::NoteCreate` corresponding to the fake log entry |

### Notebooks

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `create_and_read_notebook` | Create a notebook, read by ID | `title` matches; `deleted_at` is `None` |
| `list_notebooks_includes_created` | Create a notebook, then `list_notebooks` | The notebook appears in the list (regression: the `.msgpack` sidecar must be matched by the listing filter) |
| `delete_notebook_soft_deletes` | Create, delete, list, then read raw | Absent from list; `deleted_at` is set when read directly |
| `update_nonexistent_notebook_returns_not_found` | Update a notebook that does not exist | `StorageError::NotFound` |
| `delete_nonexistent_notebook_returns_not_found` | Delete a notebook with an unknown UUID | `StorageError::NotFound` |

### Tags

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `create_and_read_tag` | Create a tag, read by ID | `title` matches |
| `list_tags_includes_created` | Create a tag, then `list_tags` | The tag appears in the list (same `.msgpack` listing regression as notebooks) |
| `add_and_list_note_tags` | Create note + tag, add association, list tags for the note | Returns one tag with the expected ID |
| `remove_note_tag` | Add then remove a note-tag association, list again | Returns an empty list |
| `update_nonexistent_tag_returns_not_found` | Update a tag that was never created | `StorageError::NotFound` |
| `delete_nonexistent_tag_returns_not_found` | Delete a tag with an unknown UUID | `StorageError::NotFound` |

### Resources

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `create_and_read_resource` | Create a resource with binary data, read it back | Metadata and binary bytes match the originals |
| `list_resources_excludes_data` | Create three resources, call `list_resources` | Returns three metadata records (no binary data in the list) |
| `delete_resource` | Create a resource, delete it, attempt to read | `StorageError::NotFound` |

### Durability, hygiene & multi-device safety

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `concurrent_same_note_updates_keep_every_log_entry` | Many concurrent updates to one note | Every entry lands in the single device log; none dropped by a racing rename |
| `failed_atomic_write_cleans_up_its_temp_file` | An `atomic_write` whose rename fails | No `*.tmp` litter; destination untouched |
| `startup_sweeps_orphaned_tmp_files_but_not_syncthing_ones` | Plant orphan `*.tmp` + a `.syncthing.*.tmp` | The orphans are swept at startup; the Syncthing temp is preserved |
| `detects_syncthing_conflict_copies_without_removing_them` | Plant `*.sync-conflict-*` files | All detected and reported; none deleted; startup not blocked |
| format-version tests | Legacy stamp / newer-than-build stamp | Migration ladder runs; a newer on-disk format is refused |

### Ordering, starring & the note index

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `ordering_fields_round_trip_and_manual_order_query` | Notes with pinned/normal/starred fields | Fields round-trip; `list_notes_in_notebook` returns pinned-first manual order; starred list spans notebooks; `notebook_sort_profile` summarises |
| `note_index_reflects_local_writes_after_it_is_built` | List (build index), then create/delete | Incremental insert/remove reflected without re-scanning |
| `note_index_reflects_changes_pulled_from_a_peer` | A peer's log replicated, then a sync cycle | The synced note appears in the listings |

Two-device convergence (`fs_two_device_*`), per-note and global-log **compaction**, and
resource **purge** are also exercised in this file; see the source for the full matrix.

## Fixtures and helpers

Most tests create their own `FsBackend` on a fresh temp dir. Two-device and index tests share:

| Utility | Source | Purpose |
|---------|--------|---------|
| `tempdir()` | `tempfile` crate | A unique temporary directory, deleted when the guard drops |
| `Note::new`, `Notebook::new`, … | `keeplin_core::models` | Domain objects with a fresh UUID and current timestamps |
| `replicate_logs(from, to)` | in-file | Copy one device's global + per-note logs to another (simulate Syncthing) |
| `drain_sync(backend)` | in-file | `receive_changes` + `apply_change` — one pull-and-apply cycle |

## Coverage gaps

- Cross-**process** access to one store is not tested here; it is prevented at runtime by the
  daemon's per-store OS lock (`keeplin-daemon/src/main.rs`), not by `FsBackend`.
- The FS note-listing index reflects a peer edit only after a sync cycle materializes it (by
  design); single-note `read_note` freshness is covered by `read_does_not_rewrite_projection`
  and the two-device tests.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `drain_sync()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `own_log_stats()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `delete_for_unknown_sidecar_entity_leaves_a_tombstone_blocking_a_stale_create()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `create_and_read_note()` — defined here (EXTRACTED; file-local)
- `update_note()` — defined here (EXTRACTED; file-local)
- `delete_note_soft_deletes()` — defined here (EXTRACTED; file-local)
- `list_notes_excludes_deleted()` — defined here (EXTRACTED; file-local)
- `read_nonexistent_note_returns_not_found()` — defined here (EXTRACTED; file-local)
- `device_id_is_stable_across_instances()` — defined here (EXTRACTED; file-local)
- `sync_state_persists()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: references×2; e.g. `FsBackend`)
- `keeplin-core/src/storage/note_log.rs` — (no companion doc) (EXTRACTED: calls×1; e.g. `vv()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Each test gets a fresh tempdir and a fresh `FsBackend`; tests must not share state.
- Asserts the single-writer log layout and merge-on-read semantics that make file replication safe.

## Related files

- `keeplin-core/src/storage/fs.rs` — the code under test
- `keeplin-core/tests/encryption.rs` — tests the `EncryptedBackend` wrapping `FsBackend`
