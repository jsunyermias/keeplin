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

## Fixtures and helpers

This test file uses no shared helper functions. Each test creates its own `FsBackend`
instance inside the test function body.

| Utility | Source | Purpose |
|---------|--------|---------|
| `tempdir()` | `tempfile` crate | Creates a unique temporary directory that is deleted when the `TempDir` guard is dropped |
| `Note::new`, `Notebook::new`, etc. | `keeplin_core::models` | Constructs domain objects with a fresh UUID and current timestamps |

## Coverage gaps

- Concurrent access from two `FsBackend` instances to the same directory is not tested.
  `FsBackend` is not designed for concurrent access, so this is intentional.
- The `apply_change` method is tested indirectly through `get_changes_since_scans_other_device_logs`
  but only for the `NoteCreate` variant. All other variants are covered in the
  `db_backend.rs` tests via `DbBackend::apply_change`.

## Related files

- `keeplin-core/src/storage/fs.rs` — the code under test
- `keeplin-core/tests/encryption.rs` — tests the `EncryptedBackend` wrapping `FsBackend`
