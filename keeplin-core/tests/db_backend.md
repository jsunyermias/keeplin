# `tests/db_backend.rs` — DbBackend integration tests

## What is tested

This file contains integration tests for `DbBackend`, the LibSQL + WebSocket
`StorageBackend` implementation. All tests use the `in_memory_backend` helper, which
creates a `DbBackend` pointing to a temporary `.db` file with an empty `server_url`.
The WebSocket connection is therefore skipped, and all tests exercise only local
persistence logic. The temporary file path remains valid for the duration of the test
because the `TempDir` guard is intentionally leaked via `std::mem::forget`.

## Fixtures and helpers

| Helper | Purpose |
|--------|---------|
| `async fn in_memory_backend() -> DbBackend` | Creates a fresh `DbBackend` backed by a temporary SQLite file with no WebSocket connection; leaks the `TempDir` guard to keep the path valid |

## Test cases

### Notes

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `create_and_read_note` | Create a note, read by ID | `title` and `body` match |
| `update_note` | Create a note, change `title`, read back | New title is reflected |
| `delete_note_soft_deletes` | Create and delete a note, list all notes | Deleted note is absent |
| `list_notes_excludes_deleted` | Create two notes, delete one, list | Only the un-deleted note is returned |
| `read_nonexistent_returns_not_found` | Read a note with an unknown UUID | `StorageError::NotFound` |
| `update_nonexistent_note_returns_not_found` | Update a note that was never stored | `StorageError::NotFound` |
| `delete_nonexistent_note_returns_not_found` | Delete with an unknown UUID | `StorageError::NotFound` |

### Device and sync state

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `device_id_is_stable` | Open the same `.db` file with two separate `DbBackend` instances | Both return the same device ID |
| `sync_state_round_trips` | Write a sync timestamp, read it back | Timestamps match at second-level precision |
| `get_changes_since_returns_updated_notes` | Create a note, call `get_changes_since(before)` | Returns at least one `Change::NoteCreate` |

### Notebooks

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `create_and_read_notebook` | Create a notebook, read by ID | `title` matches; `deleted_at` is `None` |
| `delete_notebook_soft_deletes` | Create, delete, list; then read raw | Absent from list; `deleted_at` set when read directly |
| `update_nonexistent_notebook_returns_not_found` | Update a missing notebook | `StorageError::NotFound` |
| `delete_nonexistent_notebook_returns_not_found` | Delete with an unknown UUID | `StorageError::NotFound` |

### Tags

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `create_and_read_tag` | Create a tag, read by ID | `title` matches |
| `add_and_list_note_tags` | Create note + tag, link them, list tags for the note | Returns one tag with the expected ID |
| `remove_note_tag` | Link then remove a note-tag association, list again | Returns an empty list |
| `update_nonexistent_tag_returns_not_found` | Update a tag that was never created | `StorageError::NotFound` |
| `delete_nonexistent_tag_returns_not_found` | Delete a tag with an unknown UUID | `StorageError::NotFound` |

### Resources

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `create_and_read_resource` | Create a resource with binary data, read it back | Both metadata and binary bytes match |
| `list_resources_excludes_data` | Create three resources, list | Returns three metadata records without binary payloads |
| `delete_resource` | Create, delete, then read | `StorageError::NotFound` |
| `purge_reclaims_old_tombstoned_payloads_only` | Delete a resource, purge before/after the cutoff | Only tombstones past the cutoff lose their payload; metadata survives; idempotent |

### Pinning, ordering & starring

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `ordering_fields_round_trip_and_manual_order_query` | Notes with pinned/normal/legacy/starred fields | Fields round-trip; `list_notes_in_notebook` returns manual order (pinned first, `0` sentinel as 1000); cursor pages match; starred list spans notebooks; `notebook_sort_profile` summarises |
| `sync_applied_change_carries_ordering_fields` | `apply_change` a note carrying the new fields | Stored and queryable; resolves under whole-note version vectors (#55) |

### Schema migration

The in-source `#[cfg(test)]` module in `db.rs` covers the migration ladder: a fresh DB stamps
the current `user_version`; a pre-framework DB is carried onto the ladder without data loss and
**v2 moves a `NULL notebook_id` to the Inbox**; a newer-than-build stamp is refused.

## Coverage gaps

- WebSocket send/receive paths (`send_changes`, `receive_changes`) are not exercised
  because no live server is available in the test environment. Testing these would require
  a mock WebSocket server.
- The `entity_changes` table and `record_change` helper are tested implicitly through
  `get_changes_since_returns_updated_notes` but only for the `NoteCreate` variant. Other
  entity types record changes using the same code path, so explicit tests for each variant
  would be redundant.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `in_memory_backend()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `delete_for_unknown_entity_leaves_a_tombstone_blocking_a_stale_create()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `create_and_read_note()` — defined here (EXTRACTED; file-local)
- `update_note()` — defined here (EXTRACTED; file-local)
- `delete_note_soft_deletes()` — defined here (EXTRACTED; file-local)
- `list_notes_excludes_deleted()` — defined here (EXTRACTED; file-local)
- `read_nonexistent_returns_not_found()` — defined here (EXTRACTED; file-local)
- `device_id_is_stable()` — defined here (EXTRACTED; file-local)
- `sync_state_round_trips()` — defined here (EXTRACTED; file-local)
- `get_changes_since_returns_updated_notes()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: references×1; e.g. `DbBackend`)
- `keeplin-core/src/storage/note_log.rs` — (no companion doc) (EXTRACTED: calls×1; e.g. `vv()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- All tests run offline (`server_url = ""`): they exercise local storage semantics only; the wire path is `ws_sync.rs`.
- Version-vector conflict semantics asserted here must match `note_log::resolve` exactly.

## Related files

- `keeplin-core/src/storage/db.rs` — the code under test
- `keeplin-core/tests/fs_backend.rs` — parallel test suite for `FsBackend`
