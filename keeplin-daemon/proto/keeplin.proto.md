# `proto/keeplin.proto` — gRPC service definition

## Service overview

`KeeplinService` is the single gRPC service that `keeplin-daemon` exposes. It provides
CRUD operations for all five entity types (notes, notebooks, tags, note-tag associations,
resources), a set of **linking & reference** RPCs (aliases, manual links, backlinks,
reference resolution, alias-collision listing), and one server-streaming RPC for triggering
a synchronisation cycle with the remote peer. List RPCs (`ListNotes`, `ListNotebooks`,
`ListTags`, `ListResources`, `ListNoteTags`, `ListBacklinks`) are cursor-paginated:
requests carry `page_size` + `page_token`, responses carry a `next_page_token`.

## RPC methods

### Notes

| Method | Request | Response | Description |
|--------|---------|----------|-------------|
| `ListNotes` | `ListNotesRequest` | `ListNotesResponse` | Returns all notes that have not been soft-deleted |
| `CreateNote` | `CreateNoteRequest` | `CreateNoteResponse` | Creates a note and returns the stored copy |
| `GetNote` | `GetNoteRequest` | `GetNoteResponse` | Fetches one note by UUID |
| `UpdateNote` | `UpdateNoteRequest` | `UpdateNoteResponse` | Overwrites a note's fields; `updated_at` is set server-side |
| `DeleteNote` | `DeleteNoteRequest` | `DeleteNoteResponse` | Soft-deletes a note |

### Notebooks

| Method | Request | Response | Description |
|--------|---------|----------|-------------|
| `ListNotebooks` | `ListNotebooksRequest` | `ListNotebooksResponse` | Lists active notebooks |
| `CreateNotebook` | `CreateNotebookRequest` | `CreateNotebookResponse` | Creates a notebook |
| `GetNotebook` | `GetNotebookRequest` | `GetNotebookResponse` | Fetches one notebook by UUID |
| `UpdateNotebook` | `UpdateNotebookRequest` | `UpdateNotebookResponse` | Renames a notebook |
| `DeleteNotebook` | `DeleteNotebookRequest` | `DeleteNotebookResponse` | Soft-deletes a notebook |

### Tags

| Method | Request | Response | Description |
|--------|---------|----------|-------------|
| `ListTags` | `ListTagsRequest` | `ListTagsResponse` | Lists all tags |
| `CreateTag` | `CreateTagRequest` | `CreateTagResponse` | Creates a tag |
| `GetTag` | `GetTagRequest` | `GetTagResponse` | Fetches one tag by UUID |
| `UpdateTag` | `UpdateTagRequest` | `UpdateTagResponse` | Renames a tag |
| `DeleteTag` | `DeleteTagRequest` | `DeleteTagResponse` | Soft-deletes a tag |
| `AddNoteTag` | `AddNoteTagRequest` | `AddNoteTagResponse` | Attaches a tag to a note |
| `RemoveNoteTag` | `RemoveNoteTagRequest` | `RemoveNoteTagResponse` | Detaches a tag from a note |
| `ListNoteTags` | `ListNoteTagsRequest` | `ListNoteTagsResponse` | Lists all tags attached to a given note |

### Resources

| Method | Request | Response | Description |
|--------|---------|----------|-------------|
| `ListResources` | `ListResourcesRequest` | `ListResourcesResponse` | Lists resource metadata (no binary payload) |
| `CreateResource` | `CreateResourceRequest` | `CreateResourceResponse` | Uploads resource metadata and binary data together |
| `GetResource` | `GetResourceRequest` | `GetResourceResponse` | Returns metadata and binary data for one resource |
| `DeleteResource` | `DeleteResourceRequest` | `DeleteResourceResponse` | Permanently deletes a resource (hard delete) |

### Linking & references

| Method | Request | Response | Description |
|--------|---------|----------|-------------|
| `SetNoteAlias` | `SetNoteAliasRequest` | `SetNoteAliasResponse` | Set/clear a note's alias (rejected with `already_exists` if it duplicates a live alias) |
| `SetNotebookAlias` | `SetNotebookAliasRequest` | `SetNotebookAliasResponse` | Set/clear a notebook's alias |
| `AddNoteLink` | `AddNoteLinkRequest` | `AddNoteLinkResponse` | Add a manual link from one note to another |
| `RemoveNoteLink` | `RemoveNoteLinkRequest` | `RemoveNoteLinkResponse` | Remove a link by index |
| `ListBacklinks` | `ListBacklinksRequest` | `ListBacklinksResponse` | Paginated list of notes that link **to** a note |
| `ResolveReference` | `ResolveReferenceRequest` | `ResolveReferenceResponse` | Resolve a `#…` reference → `{ note_id, bookmark_number? }` |
| `ListAliasConflicts` | `ListAliasConflictsRequest` | `ListAliasConflictsResponse` | Aliases shared by 2+ live entities (post-sync collisions) |

There is **no** RPC to set bookmarks: bookmarks are declared inline in the note body with
`[text](### "alias")` markdown links and are returned inside each `Note` message.

### Sync

| Method | Request | Response | Description |
|--------|---------|----------|-------------|
| `Sync` | `SyncRequest` | `stream SyncProgress` | Server-streaming RPC; the server sends multiple `SyncProgress` messages as it moves through the sync stages |

## Message types

### `Note`
| Field | Field number | Type | Description |
|-------|-------------|------|-------------|
| `id` | 1 | `string` | UUID v4, generated at creation |
| `title` | 2 | `string` | User-visible title |
| `body` | 3 | `string` | Full text content |
| `notebook_id` | 4 | `string` | UUID of the parent notebook, or empty string if none |
| `is_todo` | 5 | `bool` | Whether this note is a to-do item |
| `todo_due` | 6 | `string` | RFC-3339 deadline, or empty string |
| `todo_completed` | 7 | `string` | RFC-3339 completion timestamp, or empty string |
| `created_at` | 8 | `string` | RFC-3339 creation timestamp |
| `updated_at` | 9 | `string` | RFC-3339 last-update timestamp |
| `deleted_at` | 10 | `optional string` | RFC-3339 soft-delete timestamp; absent = active |
| `alias` | 11 | `optional string` | Human-readable alias; absent = none |
| `bookmarks` | 12 | `repeated Bookmark` | In-note anchors derived from `[text](### "alias")` links in the body |
| `links` | 13 | `repeated NoteLink` | Links to other notes (content-derived + manual) |

`notebook_id` (4), `todo_due` (6), and `todo_completed` (7) are `optional string`; the
proto3 `optional` presence bit distinguishes "unset" from "empty string".

### `Notebook`
| Field | Field number | Type | Description |
|-------|-------------|------|-------------|
| `id` | 1 | `string` | UUID v4 |
| `title` | 2 | `string` | User-visible name |
| `created_at` | 3 | `string` | RFC-3339 |
| `updated_at` | 4 | `string` | RFC-3339 |
| `deleted_at` | 5 | `optional string` | RFC-3339; absent = active |
| `alias` | 6 | `optional string` | Human-readable alias; absent = none |

### `Tag`

Same fields as `Notebook` minus `alias`: `id`, `title`, `created_at`, `updated_at`, `deleted_at`.

### `Bookmark`
| Field | Field number | Type | Description |
|-------|-------------|------|-------------|
| `number` | 1 | `uint32` | 1-based order of the bookmark within the body |
| `text` | 2 | `string` | The link text (`[text]…`) |
| `alias` | 3 | `string` | The optional link title, or `text` when no title was given |

### `NoteLink`
| Field | Field number | Type | Description |
|-------|-------------|------|-------------|
| `source` | 1 | `string` | `"content"` (derived from the body) or `"manual"` |
| `raw` | 2 | `string` | The raw `#…` reference as written |
| `target_note_id` | 3 | `optional string` | Resolved destination note UUID; absent = unresolved |

### `Resource`
| Field | Field number | Type | Description |
|-------|-------------|------|-------------|
| `id` | 1 | `string` | UUID v4 |
| `title` | 2 | `string` | User-visible name |
| `mime_type` | 3 | `string` | IANA media type |
| `file_name` | 4 | `string` | Original file name |
| `size` | 5 | `int64` | Binary payload size in bytes |
| `created_at` | 6 | `string` | RFC-3339 |

### `SyncProgress`

Sent repeatedly during the server-streaming `Sync` RPC to report progress.

| Field | Type | Description |
|-------|------|-------------|
| `stage` | `Stage` enum | Current stage in the sync cycle |
| `changes_count` | `int32` | Number of changes relevant to this stage |
| `message` | `string` | Human-readable description of the current stage |

#### `Stage` enum

| Value | Integer | Meaning |
|-------|---------|---------|
| `COLLECTING` | 0 | Collecting local changes that occurred since the last sync |
| `SENDING` | 1 | Sending local changes to the remote peer |
| `RECEIVING` | 2 | Receiving changes from the remote peer |
| `APPLYING` | 3 | Applying received changes to the local store |
| `DONE` | 4 | Sync cycle completed successfully |

## Versioning and compatibility

- The service uses proto3, which does not have required fields. All fields are optional by
  default; missing fields receive zero values (empty string, `false`, `0`).
- Field numbers must never be reused after a field is removed. Adding new fields with
  new numbers is backward-compatible.
- Optional fields (e.g. `notebook_id`, `todo_due`, `deleted_at`, `alias`) use proto3
  `optional`, whose presence bit distinguishes "unset" from an empty string.
- `resource.size` uses `int64` (the only signed integer type in proto3) rather than
  `uint64` to maximise compatibility with client languages that do not support unsigned
  64-bit integers. The server validates that the value is non-negative.

## Related files

- `keeplin-daemon/build.rs` — compiles this file into Rust source code at build time
- `keeplin-daemon/src/proto.rs` — includes the generated Rust code
- `keeplin-daemon/src/server.rs` — implements all the RPCs declared here
