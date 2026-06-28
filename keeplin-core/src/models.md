# `models.rs` — domain data types

## Purpose

This module defines every domain type that the Keeplin data model is built on: notes,
notebooks, tags, note-tag associations, resources, and the `Change` enum that is the
fundamental unit of synchronisation. All types derive `serde::{Serialize, Deserialize}`
so they can be stored to JSON and transmitted over the network without any conversion
layer.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `Note` | struct | A user-created note, optionally inside a notebook, optionally a to-do |
| `Notebook` | struct | A named collection that groups notes |
| `Tag` | struct | A short label that can be attached to any number of notes |
| `NoteTag` | struct | A many-to-many link between one note and one tag |
| `Resource` | struct | Metadata for a binary attachment (the binary bytes live separately) |
| `Change` | enum | One unit of change that can be sent to or received from another device |

## Struct fields

### `Note`
| Field | Type | Description |
|-------|------|-------------|
| `id` | `Uuid` | Unique identifier, generated at creation time |
| `title` | `String` | User-visible title; may be encrypted at rest |
| `body` | `String` | Full text content; may be encrypted at rest |
| `notebook_id` | `Option<Uuid>` | Parent notebook, or `None` if the note is un-filed |
| `is_todo` | `bool` | Whether this note is a to-do item |
| `todo_due` | `Option<DateTime<Utc>>` | Optional deadline for the to-do |
| `todo_completed` | `Option<DateTime<Utc>>` | Timestamp when the to-do was checked off |
| `created_at` | `DateTime<Utc>` | Set once at creation; never modified |
| `updated_at` | `DateTime<Utc>` | Refreshed on every mutation |
| `deleted_at` | `Option<DateTime<Utc>>` | Set on soft-delete; `None` means the note is active |

### `Notebook`
| Field | Type | Description |
|-------|------|-------------|
| `id` | `Uuid` | Unique identifier |
| `title` | `String` | User-visible name; may be encrypted at rest |
| `created_at` | `DateTime<Utc>` | Set once at creation |
| `updated_at` | `DateTime<Utc>` | Refreshed on every mutation |
| `deleted_at` | `Option<DateTime<Utc>>` | Set on soft-delete |

### `Tag`

Same fields as `Notebook`: `id`, `title`, `created_at`, `updated_at`, `deleted_at`.

### `NoteTag`
| Field | Type | Description |
|-------|------|-------------|
| `note_id` | `Uuid` | The note that this tag is attached to |
| `tag_id` | `Uuid` | The tag that is attached to the note |

### `Resource`
| Field | Type | Description |
|-------|------|-------------|
| `id` | `Uuid` | Unique identifier |
| `title` | `String` | User-visible name; may be encrypted at rest |
| `mime_type` | `String` | IANA media type (e.g. `"image/png"`); may be encrypted |
| `file_name` | `String` | Original file name; may be encrypted |
| `size` | `u64` | Binary payload size in bytes; stored in plaintext |
| `created_at` | `DateTime<Utc>` | Set once at creation |

## `Change` enum — all 13 variants

`Change` is the synchronisation payload. It is serialised with a `"op"` discriminant
tag and snake-cased variant names (e.g. `"note_create"`). `#[serde(alias)]` attributes
on the `NoteCreate`, `NoteUpdate`, and `NoteDelete` variants accept the old short tags
(`"create"`, `"update"`, `"delete"`) so that v1 log files remain readable.

| Variant | Payload | Description |
|---------|---------|-------------|
| `NoteCreate` | `{ note: Note }` | A new note was created |
| `NoteUpdate` | `{ note: Note }` | An existing note was updated |
| `NoteDelete` | `{ id: Uuid }` | A note was soft-deleted |
| `NotebookCreate` | `{ notebook: Notebook }` | A new notebook was created |
| `NotebookUpdate` | `{ notebook: Notebook }` | A notebook was renamed |
| `NotebookDelete` | `{ id: Uuid }` | A notebook was soft-deleted |
| `TagCreate` | `{ tag: Tag }` | A new tag was created |
| `TagUpdate` | `{ tag: Tag }` | A tag was renamed |
| `TagDelete` | `{ id: Uuid }` | A tag was soft-deleted |
| `NoteTagAdd` | `{ note_id, tag_id }` | A tag was attached to a note |
| `NoteTagRemove` | `{ note_id, tag_id }` | A tag was detached from a note |
| `ResourceCreate` | `{ resource, data? }` | A resource was added; `data` is `Some` in `DbBackend` and `None` in `FsBackend` |
| `ResourceDelete` | `{ id: Uuid }` | A resource was permanently deleted |

### `ResourceCreate.data` semantics

`data: Option<Vec<u8>>` carries the binary payload when syncing through `DbBackend`
(where there is no shared filesystem). The field is omitted from JSON when `None`
(`#[serde(skip_serializing_if = "Option::is_none")]`) and defaults to `None` when
absent (`#[serde(default)]`), ensuring full backward compatibility with v1 log entries.

## Public utility functions

### `fn new_id() -> Uuid`
Generates a new random UUID v4. Used by every `::new()` constructor; callers should
never generate IDs themselves.

### `fn now() -> DateTime<Utc>`
Returns the current UTC timestamp. Used by every `::new()` constructor and by the sync
engine when recording a sync timestamp.

## Design notes

- All structs derive `PartialEq + Eq + Hash` so they can be stored in `HashSet` or used
  as `HashMap` keys, which is necessary for deduplicating change lists in the sync engine.
- Soft deletes (`deleted_at: Option<DateTime<Utc>>`) are used for notes, notebooks, and
  tags. Resources use hard delete because binary blobs can be very large and there is no
  benefit to retaining them after deletion.
- `Uuid::new_v4()` produces a random UUID that is globally unique with overwhelming
  probability, so IDs generated on different offline devices will never collide.

## Related files

- `keeplin-core/src/storage/backend.rs` — every `StorageBackend` method takes or
  returns these types
- `keeplin-core/src/encryption.rs` — encrypts/decrypts `title`, `body`, `mime_type`,
  `file_name` fields before they touch disk
