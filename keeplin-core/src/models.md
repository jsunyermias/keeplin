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
| `Note` | struct | A user-created note; always in exactly one notebook (the Inbox / nil UUID when unfiled), optionally a to-do |
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
| `notebook_id` | `Uuid` | Parent notebook — **never "none"**: an unfiled note belongs to the Inbox (nil UUID, `ordering::INBOX_ID`). A custom deserializer maps an old `null`/missing value to the nil UUID |
| `is_todo` | `bool` | Whether this note is a to-do item |
| `todo_due` | `Option<DateTime<Utc>>` | Optional deadline for the to-do |
| `todo_completed` | `Option<DateTime<Utc>>` | Timestamp when the to-do was checked off |
| `created_at` | `DateTime<Utc>` | Set once at creation; never modified |
| `updated_at` | `DateTime<Utc>` | Refreshed on every mutation |
| `deleted_at` | `Option<DateTime<Utc>>` | Set on soft-delete; `None` means the note is active |
| `alias` | `Option<String>` | Optional human-readable alias, unique among live notes **in the same notebook** (the same alias may recur elsewhere); lets links target `#<alias>`. Inbox notes carry none and are never link targets. Encrypted at rest. |
| `bookmarks` | `Vec<Bookmark>` | In-note anchors derived from `[text](### "alias")` links in the body (see `links.rs`) |
| `links` | `Vec<NoteLink>` | Links to other notes: content-derived (`[t](#…)`) and manual |
| `vv` | `VersionVector` | Per-device version vector for conflict resolution; a local write increments this device's counter. Plaintext sync metadata. See `note_log::resolve` |
| `last_writer` | `String` | Device id that authored the current value; the concurrent tiebreak alongside `updated_at`. Plaintext |
| `is_pinned` | `bool` | Pinned to the top of its notebook (`sort_key` in `1..=999`). The Inbox has no pinning. Plaintext |
| `is_starred` | `bool` | Globally starred; orthogonal to pinning and to the notebook (never moves the note). Plaintext |
| `sort_key` | `u32` | Manual position within the notebook, ascending. `0` is the legacy "never positioned" sentinel |

The three navigation fields are `#[serde(default)]` (older rows without them still parse) and are
maintained by `LinkingBackend` — see `links.md` / `linking.md`. The `vv`/`last_writer` fields
are also `#[serde(default)]` (empty ⇒ pre-VV record) and are stamped by the backends on write.

The ordering fields (`is_pinned`, `is_starred`, `sort_key`) are likewise `#[serde(default)]`, so
old records and old peers parse without them; the placement rules that set them live in
`ordering.rs` (`pin_note`, `unpin_note`, `star_note`, `unstar_note`, `reorder_note`,
`place_new_note`, etc.). `Note::effective_sort_key()` maps the `0` sentinel to `DEFAULT_SORT_KEY`
(1000), so a never-positioned note orders at the start of the normal band without any data
rewrite. See `ordering.md`.

### `Notebook`
| Field | Type | Description |
|-------|------|-------------|
| `id` | `Uuid` | Unique identifier |
| `title` | `String` | User-visible name; may be encrypted at rest |
| `created_at` | `DateTime<Utc>` | Set once at creation |
| `updated_at` | `DateTime<Utc>` | Refreshed on every mutation |
| `deleted_at` | `Option<DateTime<Utc>>` | Set on soft-delete |
| `alias` | `Option<String>` | Optional alias, unique among live notebooks; scopes `#<notebook>#<note>`. Encrypted at rest. |
| `vv` / `last_writer` | `VersionVector` / `String` | Version-vector conflict-resolution metadata (see `Note`). |

### `Tag`

Same fields as `Notebook` minus `alias`: `id`, `title`, `created_at`, `updated_at`,
`deleted_at`, `vv`, `last_writer`.

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
| `deleted_at` | `Option<DateTime<Utc>>` | Set on soft-delete; `None` means the resource is active |
| `vv` | `VersionVector` | Version vector; bumped on every write and on delete |
| `last_writer` | `String` | Device id of the most recent writer; the `resolve` tiebreak |

## `Change` enum — all 13 variants

`Change` is the synchronisation payload. It is serialised with a `"op"` discriminant
tag and snake-cased variant names (e.g. `"note_create"`). `#[serde(alias)]` attributes
on the `NoteCreate`, `NoteUpdate`, and `NoteDelete` variants accept the old short tags
(`"create"`, `"update"`, `"delete"`) so that v1 log files remain readable.

| Variant | Payload | Description |
|---------|---------|-------------|
| `NoteCreate` | `{ note: Note }` | A new note was created |
| `NoteUpdate` | `{ note: Note }` | An existing note was updated |
| `NoteDelete` | `{ id, deleted_at, vv, last_writer }` | A note was soft-deleted; the tombstone carries its version vector so it resolves like an edit |
| `NotebookCreate` | `{ notebook: Notebook }` | A new notebook was created |
| `NotebookUpdate` | `{ notebook: Notebook }` | A notebook was renamed |
| `NotebookDelete` | `{ id, deleted_at, vv, last_writer }` | A notebook was soft-deleted |
| `TagCreate` | `{ tag: Tag }` | A new tag was created |
| `TagUpdate` | `{ tag: Tag }` | A tag was renamed |
| `TagDelete` | `{ id, deleted_at, vv, last_writer }` | A tag was soft-deleted |
| `NoteTagAdd` | `{ note_id, tag_id, updated_at, vv, last_writer }` | A tag was attached (versioned present state) |
| `NoteTagRemove` | `{ note_id, tag_id, updated_at, vv, last_writer }` | A tag was detached (versioned tombstone) |
| `ResourceCreate` | `{ resource, data? }` | A resource was added; `data` is `Some` in `DbBackend` and `None` in `FsBackend` |
| `ResourceDelete` | `{ id, deleted_at, vv, last_writer }` | A resource was soft-deleted; the tombstone carries its version vector so it resolves like an edit |

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
- Soft deletes (`deleted_at: Option<DateTime<Utc>>`) are used for every entity — notes,
  notebooks, tags, note↔tag associations, and resources — so that every delete is a versioned
  tombstone that competes in `note_log::resolve`. A resource's binary blob is retained on disk /
  in the database after a soft delete; reclaiming that space is left to the `FsBackend` compaction
  phase.
- `Uuid::new_v4()` produces a random UUID that is globally unique with overwhelming
  probability, so IDs generated on different offline devices will never collide.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `Note` — defined here (EXTRACTED; 140 cross-file edge(s))
- `Notebook` — defined here (EXTRACTED; 58 cross-file edge(s))
- `Tag` — defined here (EXTRACTED; 45 cross-file edge(s))
- `Change` — defined here (EXTRACTED; 44 cross-file edge(s))
- `Resource` — defined here (EXTRACTED; 34 cross-file edge(s))
- `NoteTag` — defined here (EXTRACTED; 7 cross-file edge(s))
- `new_id()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `now()` — defined here (EXTRACTED; file-local)
- `de_notebook_id()` — defined here (EXTRACTED; file-local)
- `.new()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/links.rs` — bookmark & link types and pure parsing (EXTRACTED: references×2; e.g. `Bookmark`, `NoteLink`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/collab/mod.rs` — client of the keeplin-srv collaborative channel (EXTRACTED: references×31; e.g. `.apply_from_server()`, `.push_local_edit()`, `.patch_meta()`)
- `keeplin-core/src/encryption.rs` — transparent at-rest encryption (EXTRACTED: references×34; e.g. `.enc_note()`, `.dec_note()`, `.enc_notebook()`)
- `keeplin-core/src/history.rs` — change history reads + forward-revert (EXTRACTED: references×4; e.g. `revert_note()`, `revert_notebook()`, `revert_notes_to()`)
- `keeplin-core/src/interop.rs` — vCard & iCalendar format compatibility (EXTRACTED: calls×2, imports_from×1, references×9; e.g. `interop.rs`, `.from_note()`, `.apply_to_note()`)
- `keeplin-core/src/linking.rs` — `LinkingBackend` decorator + reference resolution (EXTRACTED: references×52; e.g. `AliasConflicts`, `.from_snapshots()`, `.upsert_note()`)
- `keeplin-core/src/ordering.rs` — the Inbox, pinning, manual ordering, and starring (EXTRACTED: references×12; e.g. `create_placed()`, `move_note()`, `pin_note()`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: references×1; e.g. `paginate_notes()`)
- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: calls×2, references×32; e.g. `.get_or_create_device_id()`, `.send_changes()`, `.create_note()`)
- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: calls×1, references×37; e.g. `.read_or_create_device_id()`, `.append_note_op()`, `.create_note()`)
- `keeplin-core/src/storage/note_log.rs` — (no companion doc) (EXTRACTED: imports_from×1, references×2; e.g. `Merged`, `NoteOp`, `note_log.rs`)
- `keeplin-core/src/sync/engine.rs` — SyncEngine (EXTRACTED: references×2; e.g. `run_sync()`, `.sync()`)
- `keeplin-daemon/src/event_backend.rs` — `EventBackend` change-publishing decorator (EXTRACTED: references×30; e.g. `.create_note()`, `.list_notes()`, `.list_notes_in_notebook()`)
- `keeplin-daemon/src/metrics.rs` — operational metrics (EXTRACTED: references×26; e.g. `.create_note()`, `.list_notes()`, `.list_notes_in_notebook()`)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: references×42; e.g. `add_link()`, `batch_revert_notes_ep()`, `create_note()`)
- `keeplin-daemon/src/search.rs` — daemon full-text search (EXTRACTED: references×5; e.g. `denormalize()`, `index_note()`, `.upsert()`)
- `keeplin-daemon/src/server.rs` — gRPC service implementation (EXTRACTED: references×5; e.g. `note_to_proto()`, `proto_to_note()`, `notebook_to_proto()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Every entity carries `vv`, `last_writer`, `updated_at`, and soft-delete via `deleted_at` — the conflict-resolution contract every backend relies on.
- `Change` is serialised with tag `op` in snake_case (v1 aliases kept for old logs); renaming variants or fields breaks stored journals and the server relay — additive evolution only.
- `Change::ResourceCreate.data` is optional and skipped when `None` (blob-stripped relay form).

## Related files

- `keeplin-core/src/links.rs` — defines `Bookmark` and `NoteLink` (embedded in `Note`) plus
  the reference grammar; see `links.md`
- `keeplin-core/src/linking.rs` — the `LinkingBackend` decorator that maintains
  `Note.alias`/`bookmarks`/`links`; see `linking.md`
- `keeplin-core/src/storage/backend.rs` — every `StorageBackend` method takes or
  returns these types
- `keeplin-core/src/encryption.rs` — encrypts/decrypts the prose fields (`title`, `body`,
  `alias`, bookmark text/alias, link `raw`, `mime_type`, `file_name`) before they touch disk
