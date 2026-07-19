# `server.rs` — the gRPC service implementation

Self-contained companion for `keeplin-daemon/src/server.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must
be able to understand it without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module doc and the imports. Marker
`// md:Overview`.

```rust
use std::{pin::Pin, sync::Arc};
use keeplin_core::{error::{StorageError, SyncError}, linking,
    links::{Bookmark as CoreBookmark, LinkSource, NoteLink as CoreNoteLink},
    models::{now, Note as CoreNote, NoteTag, Notebook as CoreNotebook,
             Resource as CoreResource, Tag as CoreTag},
    ordering, storage::StorageBackend, sync::{run_sync, SyncStage}};
use tokio_stream::{wrappers::UnboundedReceiverStream, Stream};
use tonic::{Request, Response, Status};
use uuid::Uuid;
use crate::proto::keeplin::{…all request/response/entity wire types…};
```

**What it does** — Defines `KeeplinServer<B>`, the implementation of the
`KeeplinService` trait generated from `proto/keeplin.proto`. It bridges between the
protobuf wire types (`proto::keeplin::Note`, …) and the domain types in
`keeplin-core` (`models::Note`, …), delegating all persistence to a generic
`StorageBackend`. The file has four layers, in source order: stateless proto↔core
conversion helpers, the `KeeplinServer` struct with its inherent impl, the 43-item
`KeeplinService` trait impl (every RPC), and the post-sync maintenance helpers
shared with the REST surface.

**Dependencies** — `tonic` (RPC framework), `tokio_stream` (the `Sync` response
stream), `uuid`, `chrono` (via parsing), and `keeplin_core` (`models`, `ordering`,
`linking`, `sync::run_sync`, `storage::StorageBackend`).

**Used by** — `main.rs` (`KeeplinServer::from_shared` wrapped in
`KeeplinServiceServer`), `rest.rs` (`prune_journal_after_sync`,
`purge_resources_after_sync`).

**Repeated context** — conversion helpers are free functions rather than `From`
impls because the proto and domain types live in separate crates and the orphan
rule forbids `impl From<CoreNote> for proto::Note` here. They are stateless and
infallible in the core→proto direction (they only map known fields).

---

## fn bookmark_to_proto

**Identification** — `fn bookmark_to_proto(b: CoreBookmark) -> ProtoBookmark`.
Marker `// md:fn bookmark_to_proto`.

**What it does** — Field-for-field map (`number`, `text`, `alias`).

**Used by** — `note_to_proto`.

---

## fn link_source_str

**Identification** — `fn link_source_str(s: LinkSource) -> String`. Marker
`// md:fn link_source_str`.

**What it does** — `LinkSource::Content → "content"`, `Manual → "manual"` (the
wire encoding of the enum; the reverse mapping lives in `proto_to_notelink`).

**Used by** — `notelink_to_proto`.

---

## fn notelink_to_proto

**Identification** — `fn notelink_to_proto(l: CoreNoteLink) -> ProtoNoteLink`.
Marker `// md:fn notelink_to_proto`.

**What it does** — Maps `source` via `link_source_str`, copies `raw`, stringifies
the optional `target_note_id` UUID.

**Used by** — `note_to_proto`.

---

## fn note_to_proto

**Identification** — `fn note_to_proto(n: CoreNote) -> Note`. Marker
`// md:fn note_to_proto`.

**What it does** — Core note → wire note: UUIDs stringified, timestamps to
RFC-3339, bookmarks/links via the helpers above. **Inbox invariant on the wire**:
the Inbox (nil UUID) `notebook_id` is sent as *absent* —
`(!n.notebook_id.is_nil()).then(…)` — exactly what pre-Inbox clients always saw
for an unfiled note. Version-vector sync metadata (`vv`, `last_writer`) is not
exposed over gRPC.

**Used by** — every note-returning RPC in the trait impl, and the tests.

---

## fn notebook_to_proto

**Identification** — `fn notebook_to_proto(nb: CoreNotebook) -> Notebook`. Marker
`// md:fn notebook_to_proto`.

**What it does** — Field map with RFC-3339 timestamps; `vv`/`last_writer` not
exposed.

**Used by** — the notebook RPCs, `list_alias_conflicts`, tests.

---

## fn resource_to_proto

**Identification** — `fn resource_to_proto(r: CoreResource) -> Resource`. Marker
`// md:fn resource_to_proto`.

**What it does** — Metadata-only map (`id`, `title`, `mime_type`, `file_name`,
`size as i64`, `created_at`); payload bytes travel separately.

**Used by** — the resource RPCs and `assemble_upload`.

---

## fn tag_to_proto

**Identification** — `fn tag_to_proto(t: CoreTag) -> Tag`. Marker
`// md:fn tag_to_proto`.

**What it does** — Field map with RFC-3339 timestamps.

**Used by** — the tag RPCs and tests.

---

## fn storage_err

**Identification** — `fn storage_err(e: StorageError) -> Status`. Marker
`// md:fn storage_err`.

**What it does** — The single `StorageError` → gRPC `Status` mapping used by every
RPC:

| `StorageError` | gRPC code | Why |
|---|---|---|
| `NotFound` | `NOT_FOUND` | client can distinguish "does not exist" from failure |
| `CorruptedData` | `DATA_LOSS` | AES-GCM tag failure (wrong key / tampered ciphertext): data exists but cannot be recovered in a trustworthy form |
| `Conflict` | `ALREADY_EXISTS` | duplicate alias or similar uniqueness violation |
| `InvalidInput` | `INVALID_ARGUMENT` | domain-rule rejection (pin an Inbox note, out-of-band sort key, …) |
| everything else | `INTERNAL` | general server failure |

**Used by** — every RPC handler in this file and the `sync` error path.

---

## fn parse_uuid

**Identification** — `#[allow(clippy::result_large_err)] fn parse_uuid(s: &str,
field: &str) -> Result<Uuid, Status>`. Marker `// md:fn parse_uuid`.

**What it does** — Parses a UUID string from a protobuf field;
`INVALID_ARGUMENT` naming the offending field on failure. (The clippy allow
recurs on every helper returning `Result<_, Status>`: `tonic::Status` exceeds
clippy's `Err`-size threshold and boxing would cost a heap allocation per RPC.)

**Used by** — nearly every RPC handler.

---

## fn ensure_not_deleted

**Identification** — `#[allow(clippy::result_large_err)] fn
ensure_not_deleted<T>(read: Result<T, StorageError>, id: Uuid, deleted_at: impl
Fn(&T) -> Option<DateTime<Utc>>) -> Result<(), Status>`. Marker
`// md:fn ensure_not_deleted`.

**What it does** — Rejects an update aimed at a soft-deleted entity with
`NOT_FOUND`. The `Get*` RPCs intentionally return tombstones (sync needs to read
them), but an update on one would silently *revive* it (the client writes
`deleted_at: None` back). Revival is reserved for the sync path (`apply_change`
resolving a causal edit made after the delete), so the update RPCs answer
`NOT_FOUND` instead — mirroring the REST surface, where a tombstone already reads
and updates as `404`. Takes the read result (propagating its error through
`storage_err`) plus an accessor for the entity's `deleted_at`.

**Used by** — `update_notebook`, `update_tag` (`update_note` open-codes the same
check because it also needs the stored notebook id for move reconciliation).

---

## fn parse_optional_dt

**Identification** — `#[allow(clippy::result_large_err)] fn parse_optional_dt(s:
Option<String>) -> Result<Option<DateTime<Utc>>, Status>`. Marker
`// md:fn parse_optional_dt`.

**What it does** — `None → Ok(None)`; `Some(v)` parses as RFC-3339 or fails with
`INVALID_ARGUMENT` quoting the bad value.

**Used by** — `proto_to_note`, `create_note`, `update_notebook`, `update_tag`.

---

## fn proto_to_note

**Identification** — `#[allow(clippy::result_large_err)] fn proto_to_note(n:
Note) -> Result<CoreNote, Status>`. Marker `// md:fn proto_to_note`.

**What it does** — Wire note → core note, the fallible direction: UUIDs and
timestamps are parsed (each failure is a field-specific `INVALID_ARGUMENT`). An
absent or empty `notebook_id` becomes the **nil UUID** (the Inbox) — the inverse
of `note_to_proto`'s absent-on-the-wire rule. Incoming bookmarks/links are mapped
so a read-modify-write client preserves manual links and bookmark-alias edits;
`LinkingBackend` re-derives content entries from the body and resolves targets on
write. `vv`/`last_writer` are defaulted — sync metadata is internal to the
storage layer, which stamps it on write.

**Used by** — `update_note`.

---

## fn proto_to_bookmark

**Identification** — `fn proto_to_bookmark(b: ProtoBookmark) -> CoreBookmark`.
Marker `// md:fn proto_to_bookmark`.

**What it does** — Field-for-field map back to the core type.

**Used by** — `proto_to_note`.

---

## fn proto_to_notelink

**Identification** — `fn proto_to_notelink(l: ProtoNoteLink) -> CoreNoteLink`.
Marker `// md:fn proto_to_notelink`.

**What it does** — Any `source` value other than `"manual"` is treated as
content-derived (the default); an unparsable `target_note_id` becomes `None`
rather than an error.

**Used by** — `proto_to_note`.

---

## KeeplinServer

**Identification** — `pub struct KeeplinServer<B: StorageBackend>`. Marker
`// md:KeeplinServer`.

**What it does** — The gRPC service handler. Generic over the storage backend so
the compiler monomorphises one copy for the type chosen at startup (e.g. the full
decorator stack over `EncryptedBackend<FsBackend>` or `DbBackend`). Fields:
`backend: Arc<B>` (shared across the concurrent tasks tonic spawns per RPC — and
with the REST server), `journal_retention_days: u64` (post-sync journal prune
window, `0` disables), `resource_purge_days: u64` (post-sync tombstoned-resource
payload reclaim, `0` disables), `max_upload_bytes: usize` (cap on an assembled
streamed upload, `0` = unlimited — bounds the memory a single upload can consume,
since the payload is buffered before `create_resource`).

**Used by** — `main.rs` (`run_server_with`), the trait impl below, tests.

---

## impl KeeplinServer

**Identification** — inherent `impl<B: StorageBackend> KeeplinServer<B>`. Marker
`// md:impl KeeplinServer`. Two methods.

### fn from_shared

**Identification** — `pub fn from_shared(backend: Arc<B>,
journal_retention_days: u64, resource_purge_days: u64, max_upload_bytes: usize)
-> Self`; marker `// md:impl KeeplinServer > fn from_shared`.

**What it does** — Plain constructor over an already-shared `Arc<B>`; sharing the
`Arc` is what lets the gRPC server and the REST server drive one backend. The
result is passed to `KeeplinServiceServer::new` before registration with tonic.

**Used by** — `main.rs::run_server_with`; the test `server()` helper.

### fn assemble_upload

**Identification** — `#[allow(clippy::result_large_err)] async fn
assemble_upload<S>(&self, mut stream: S) ->
Result<Response<UploadResourceResponse>, Status> where S: Stream<Item =
Result<UploadResourceRequest, Status>> + Unpin`; marker
`// md:impl KeeplinServer > fn assemble_upload`.

**What it does** — Assembles a streamed `UploadResource` call into a stored
resource. Protocol: the **first** frame must carry `ResourceMeta` (else
`INVALID_ARGUMENT`; an empty stream too); every later frame carries a payload
chunk, appended in order; a second metadata frame mid-stream is a protocol error;
a frame with no payload is ignored. While appending, the running total is checked
against `max_upload_bytes` (`saturating_add`; `0` = no cap) —
`RESOURCE_EXHAUSTED` on excess. Because each frame is a small gRPC message, an
attachment larger than `max_message_size` uploads without any single oversized
frame. Finally builds a `CoreResource::new(meta…, size)` and calls
`backend.create_resource(resource, data)`.

**Why generic over `S`** — rather than taking `tonic::Streaming` directly, so the
assembly logic is unit-testable with an in-memory `tokio_stream::iter`.

**Used by** — the `upload_resource` RPC; the three `upload_resource_*` tests.

---

## SyncStreamItem

**Identification** — `type SyncStreamItem = Result<SyncProgress, Status>`. Marker
`// md:SyncStreamItem`.

**What it does** — Item type of the `Sync` response stream.

**Used by** — `SyncStreamPin`, the `sync` RPC's channel.

---

## SyncStreamPin

**Identification** — `type SyncStreamPin = Pin<Box<dyn Stream<Item =
SyncStreamItem> + Send>>`. Marker `// md:SyncStreamPin`.

**What it does** — The boxed, pinned stream type tonic requires for a
server-streaming response.

**Used by** — the associated `type SyncStream` and the `sync` RPC.

---

## impl KeeplinService for KeeplinServer

**Identification** — `#[tonic::async_trait] impl<B: StorageBackend>
KeeplinService for KeeplinServer<B>`. Marker
`// md:impl KeeplinService for KeeplinServer`; every item below carries
`// md:impl KeeplinService for KeeplinServer > <item>`.

**Shared conventions** (stated once here, they apply to every method):

- **Pagination**: list RPCs take `page_size` + `page_token`; an empty token means
  "first page" (`None` to the backend), and the returned
  `Option<String>` next-token is sent as `unwrap_or_default()` (empty = no more
  pages).
- **Errors**: backend errors go through `storage_err` (table above); UUID fields
  through `parse_uuid`; timestamps through `parse_optional_dt`.
- **Conversion**: every returned entity passes through the `*_to_proto` helpers.
- **Delegation targets**: plain CRUD goes straight to the `StorageBackend`
  repositories; pin/star/reorder go through `keeplin_core::ordering` (which
  enforces the pinned band `1..=999`, `NORMAL_START = 1000`, and Inbox rules);
  alias/link operations go through `keeplin_core::linking` (alias uniqueness,
  reference resolution, backlink queries).

### fn list_notes

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_notes`.

**What it does** — Paginated `backend.list_notes` (soft-deleted excluded by the
backend contract).

### fn create_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn create_note`.

**What it does** — Builds `CoreNote::new(title, body)`, applies `is_todo` and an
optional `todo_due` (empty string = absent), parses a non-empty `notebook_id`
(absent → the Inbox nil UUID from `CoreNote::new`). Then
`ordering::place_new_note` gives the note its initial manual position — top of
the Inbox, or the end of a normal notebook's unpinned band — before
`backend.create_note`.

### fn get_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn get_note`.

**What it does** — `backend.read_note(id)`. **Serves tombstones**: a soft-deleted
note is returned with its `deleted_at` set (sync needs to read tombstones); it is
the update path that answers `NOT_FOUND`.

### fn update_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn update_note`.

**What it does** — Requires the `note` message (`INVALID_ARGUMENT` if absent);
`proto_to_note`; reads the stored note and rejects the update with `NOT_FOUND`
when it is tombstoned (an update whose body defaults `deleted_at` to none would
silently revive it — revival is reserved for sync's `apply_change`). Then
`ordering::reconcile_notebook_move(stored.notebook_id, &mut note)`: moving the
note to a different notebook re-places it (its old position and pinned state
belonged to the source notebook); a plain edit keeps its position. Stamps
`updated_at = now()` server-side and calls `backend.update_note`.

### fn delete_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn delete_note`.

**What it does** — `backend.delete_note(id)` (soft delete: tombstone with
`deleted_at`, so the deletion syncs).

### fn list_notes_in_notebook

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_notes_in_notebook`.

**What it does** — Paginated `backend.list_notes_in_notebook(notebook_id, …)` —
manual order: pinned band first (`sort_key 1..=999`), then the unpinned band
(`>= 1000`). Pass the nil UUID to list the Inbox.

### fn list_starred_notes

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_starred_notes`.

**What it does** — Paginated `backend.list_starred_notes` (the cross-notebook
starred view).

### fn pin_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn pin_note`.

**What it does** — `ordering::pin_note`: moves the note into its notebook's
pinned band (`sort_key 1..=999`). Pinning an Inbox note is a domain-rule
rejection (`InvalidInput` → `INVALID_ARGUMENT`); a full band is `Conflict`.

### fn unpin_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn unpin_note`.

**What it does** — `ordering::unpin_note`: back to the unpinned band
(`NORMAL_START = 1000` onwards).

### fn star_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn star_note`.

**What it does** — `ordering::star_note` (sets the flag; no reordering).

### fn unstar_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn unstar_note`.

**What it does** — `ordering::unstar_note`.

### fn reorder_notes

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn reorder_notes`.

**What it does** — Applies each `(note_id, sort_key)` order **in request order**
via `ordering::reorder_note`; the first failure aborts the rest. Every move
already applied is durable, and re-sending the whole batch is idempotent.
Responds with the updated notes.

### fn list_notebooks

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_notebooks`.

**What it does** — Paginated `backend.list_notebooks`.

### fn create_notebook

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn create_notebook`.

**What it does** — `backend.create_notebook(CoreNotebook::new(title))`.

### fn get_notebook

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn get_notebook`.

**What it does** — `backend.read_notebook(id)`; serves tombstones (see
`get_note`).

### fn update_notebook

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn update_notebook`.

**What it does** — Requires the `notebook` message; parses fields;
**`updated_at = now()` server-side**, ignoring any client-supplied value, so
listings ordered by `updated_at` reflect the edit and a client cannot
back/post-date it — matching `update_note` and the REST endpoints (issue #75).
`ensure_not_deleted` guards against tombstone revival; then
`backend.update_notebook`.

### fn delete_notebook

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn delete_notebook`.

**What it does** — **The Inbox system notebook (nil UUID) cannot be deleted** —
`ordering::is_inbox(id)` → `INVALID_ARGUMENT`. Otherwise
`backend.delete_notebook` (soft delete; the backend moves the notebook's notes to
the Inbox per the storage contract).

### fn list_tags

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_tags`.

**What it does** — Paginated `backend.list_tags`.

### fn create_tag

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn create_tag`.

**What it does** — `backend.create_tag(CoreTag::new(title))`.

### fn add_note_tag

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn add_note_tag`.

**What it does** — `backend.add_note_tag(NoteTag { note_id, tag_id })` (both
UUIDs parsed; idempotent at the storage layer).

### fn remove_note_tag

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn remove_note_tag`.

**What it does** — `backend.remove_note_tag(note_id, tag_id)`.

### fn get_tag

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn get_tag`.

**What it does** — `backend.read_tag(id)`; serves tombstones.

### fn update_tag

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn update_tag`.

**What it does** — Same shape as `update_notebook`: required message, parsed
fields, server-side `updated_at = now()` (unspoofable ordering, issue #75),
`ensure_not_deleted`, then `backend.update_tag`.

### fn delete_tag

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn delete_tag`.

**What it does** — `backend.delete_tag(id)` (soft delete; note-tag pairs for it
stop listing).

### fn list_note_tags

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_note_tags`.

**What it does** — Paginated `backend.list_note_tags(note_id, …)` — the tags on
one note.

### fn list_resources

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_resources`.

**What it does** — Paginated `backend.list_resources` (metadata only).

### fn create_resource

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn create_resource`.

**What it does** — The unary upload: payload bytes arrive in one message
(bounded by tonic's `max_decoding_message_size`); size is taken from
`data.len()`, then `backend.create_resource(CoreResource::new(…), data)`. For
attachments larger than the message limit, use `upload_resource`.

### fn upload_resource

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn upload_resource`.

**What it does** — The client-streaming upload: delegates the
`tonic::Streaming<UploadResourceRequest>` to `assemble_upload` (protocol and
limits documented there).

### fn get_resource

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn get_resource`.

**What it does** — `backend.read_resource(id)` → metadata + full payload bytes in
one response.

### fn delete_resource

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn delete_resource`.

**What it does** — `backend.delete_resource(id)` (tombstone; the payload is
reclaimed later by `purge_resources_after_sync`).

### fn set_note_alias

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn set_note_alias`.

**What it does** — `linking::set_note_alias(backend, id, alias)`; a duplicate
alias is `Conflict` → `ALREADY_EXISTS`; an empty alias clears it.

### fn set_notebook_alias

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn set_notebook_alias`.

**What it does** — `linking::set_notebook_alias`; same rules as note aliases.

### fn add_note_link

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn add_note_link`.

**What it does** — `linking::add_manual_link(backend, note_id, raw)`: appends a
`LinkSource::Manual` link with the raw reference text and resolves its target;
manual links survive body rewrites (only content-derived links are re-derived).

### fn remove_note_link

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn remove_note_link`.

**What it does** — `linking::remove_link(backend, note_id, index)` — removal by
index into the note's `links` array.

### fn list_backlinks

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_backlinks`.

**What it does** — Paginated `linking::backlinks(backend, note_id, …)`: the notes
whose links resolve **to** this note.

### fn resolve_reference

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn resolve_reference`.

**What it does** — `linking::resolve(backend, reference)`; the response's
`note_id`/`bookmark_number` are both optional — an unresolvable reference is a
`None`/`None` response, **not** an error.

### fn list_alias_conflicts

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_alias_conflicts`.

**What it does** — `linking::alias_conflicts(backend)` → the aliases claimed by
more than one note (and, separately, notebook), each with the conflicting
entities. Conflicts can only arise from sync merges — local writes enforce
uniqueness — so this is the repair-surface for that state.

### type SyncStream

**Identification** — associated type `type SyncStream = SyncStreamPin`; marker
`// md:impl KeeplinService for KeeplinServer > type SyncStream`.

**What it does** — Binds the trait's server-streaming response type for `Sync` to
the boxed pinned stream alias.

### fn sync

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn sync`.

**What it does** — The server-streaming `Sync` RPC. Spawns the whole cycle on a
task and immediately returns the receiving half of an **unbounded** mpsc channel
as the response stream (unbounded so the synchronous progress callback in
`run_sync` can emit without awaiting; a cycle produces only a handful of
messages). The task: (1) forwards each core `SyncStage` to the client as a
`SyncProgress { stage, changes_count, message }` via `stage_to_proto`;
(2) runs `keeplin_core::sync::run_sync(backend, report)` — the whole cycle
including the watermark fix lives in core; the daemon only adapts progress and
error reporting; (3) on success runs `prune_journal_after_sync` and
`purge_resources_after_sync`; (4) on failure sends one terminal `Err` —
`SyncError::Storage` through `storage_err`, anything else as `INTERNAL`.

**Used by** — gRPC clients; the REST `POST /api/sync` has its own parallel
handler in `rest.rs` sharing the two maintenance helpers.

---

## fn prune_journal_after_sync

**Identification** — `pub(crate) async fn prune_journal_after_sync<B>(backend:
&B, retention_days: u64) where B: StorageBackend + ?Sized`. Marker
`// md:fn prune_journal_after_sync`.

**What it does** — Trims change-journal history after a successful sync cycle so
the `entity_changes` table cannot grow without bound. `0` disables. The window is
clamped to ~100 years (`min(36_500)` days) so an absurd config value cannot
overflow chrono's `Duration` (which would panic) or wrap to a negative window
that prunes the entire journal. A failure is non-fatal (WARN) because the sync
itself already succeeded. No-op on `FsBackend`, whose `prune_change_journal`
always returns `Ok(0)`. `?Sized` so it works on `dyn StorageBackend`.

**Used by** — the gRPC `sync` RPC and the REST `POST /api/sync` handler — both
surfaces honour `journal_retention_days` the same way.

---

## fn purge_resources_after_sync

**Identification** — `pub(crate) async fn purge_resources_after_sync<B>(backend:
&B, purge_days: u64) where B: StorageBackend + ?Sized`. Marker
`// md:fn purge_resources_after_sync`.

**What it does** — Reclaims payloads of resources tombstoned longer than
`purge_days` ago, after a successful sync cycle. `0` disables; same ~100-year
overflow clamp and non-fatal WARN failure handling as the journal prune.

**Used by** — the gRPC `sync` RPC and the REST `POST /api/sync` handler.

---

## fn stage_to_proto

**Identification** — `fn stage_to_proto(stage: SyncStage) -> (Stage, &'static
str)`. Marker `// md:fn stage_to_proto`.

**What it does** — Maps a core `SyncStage` to its protobuf `Stage` code plus a
human-readable message: Collecting/Sending/Receiving/Applying/Done →
"Collecting local changes" … "Sync complete".

**Used by** — the `sync` RPC's progress callback.

---

## mod tests

**Identification** — `#[cfg(test)] mod tests`. Marker `// md:mod tests`. Imports
`super::*`, the proto `ResourceMeta`/`UploadResourceRequest` types, `FsBackend`,
and the repository traits.

### fn server

**Identification** — helper; marker `// md:mod tests > fn server`.

**What it does** — A `KeeplinServer` over a fresh `FsBackend` in a leaked temp
dir (`std::mem::forget` keeps it alive for the test), plus a handle to the
backend for seeding state directly. `max_upload_bytes` is generous (1 GiB) so
the upload tests exercise assembly, not the cap.

### fn meta_frame

**Identification** — helper; marker `// md:mod tests > fn meta_frame`.

**What it does** — Builds the metadata first-frame of an upload stream.

### fn chunk_frame

**Identification** — helper; marker `// md:mod tests > fn chunk_frame`.

**What it does** — Builds a payload-chunk frame.

### fn upload_resource_assembles_chunks_in_order

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn upload_resource_assembles_chunks_in_order`.

**What it does** — A payload split across three chunks reassembles in order;
the response metadata carries the right title/file name/summed size; the
reassembled bytes round-trip through `backend.read_resource`.

### fn upload_resource_requires_metadata_first

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn upload_resource_requires_metadata_first`.

**What it does** — A stream starting with a chunk (no metadata frame) is
rejected with `INVALID_ARGUMENT`.

### fn upload_resource_enforces_the_cap

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn upload_resource_enforces_the_cap`.

**What it does** — With an explicit tiny 8-byte cap, a 10-byte payload is
refused with `RESOURCE_EXHAUSTED`.

### fn update_rpcs_reject_soft_deleted_entities

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn update_rpcs_reject_soft_deleted_entities`.

**What it does** — For note, notebook, and tag: create → delete → update with a
proto carrying `deleted_at: None` must be `NOT_FOUND`, not a silent revival; and
`GetNote` still serves the tombstone (sync reads it) — unchanged by the
rejection.

### fn update_notebook_and_tag_refresh_updated_at_server_side

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn update_notebook_and_tag_refresh_updated_at_server_side`.

**What it does** — Issue #75 regression: `UpdateNotebook`/`UpdateTag` set
`updated_at = now()` and ignore a client-supplied stale value
(`2000-01-01T00:00:00Z`); the returned timestamp must differ from the stale one
and advance past the entity's previous `updated_at`.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `KeeplinServer<B>` — defined here; the gRPC handler (EXTRACTED)
- `prune_journal_after_sync()`, `purge_resources_after_sync()` — defined here, shared with the REST surface (EXTRACTED; cross-file edges to `rest.rs`)
- the proto↔core conversion helpers (`note_to_proto`, `proto_to_note`, …) — defined here (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-daemon/src/proto.rs` — the generated wire types and the `KeeplinService` trait (EXTRACTED: imports_from)
- `keeplin-core/src/models.rs` — domain entities (EXTRACTED: references; e.g. `Note`, `Notebook`, `Tag`, `Resource`, `NoteTag`, `now`)
- `keeplin-core/src/ordering.rs` — pin/star/reorder/placement + Inbox rules (EXTRACTED: references)
- `keeplin-core/src/linking.rs` — alias/link/backlink/resolution operations (EXTRACTED: references)
- `keeplin-core/src/sync/mod.rs` — `run_sync`, `SyncStage` (EXTRACTED: references)
- `keeplin-core/src/error.rs` — `StorageError`, `SyncError` (EXTRACTED: references)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: references)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/main.rs` — constructs `KeeplinServer::from_shared` (EXTRACTED)
- `keeplin-daemon/src/rest.rs` — calls the two post-sync maintenance helpers (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- On the wire, the Inbox (nil UUID) `notebook_id` is absent; absent/empty parses back to nil (`note_to_proto` ↔ `proto_to_note`).
- `Get*` RPCs serve tombstones (sync reads them); `Update*` RPCs answer `NOT_FOUND` on a tombstone — revival is reserved for sync's `apply_change`.
- `updated_at` is stamped server-side on every update RPC; client-supplied values are ignored (issue #75).
- The Inbox system notebook cannot be deleted; pinning an Inbox note is rejected.
- The `StorageError` → `Status` mapping in `storage_err` is the single source of truth for gRPC error codes.
- Post-sync maintenance (journal prune, resource purge) must stay identical between the gRPC and REST sync handlers — both call the same two helpers.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | module doc + imports | `// md:Overview` |
| 2–8 | core→proto helpers (`bookmark_to_proto` … `tag_to_proto`) | `// md:fn <name>` |
| 9–12 | `storage_err`, `parse_uuid`, `ensure_not_deleted`, `parse_optional_dt` | `// md:fn <name>` |
| 13–15 | proto→core helpers (`proto_to_note`, `proto_to_bookmark`, `proto_to_notelink`) | `// md:fn <name>` |
| 16 | `struct KeeplinServer` | `// md:KeeplinServer` |
| 17 | `impl KeeplinServer` (+ `from_shared`, `assemble_upload`) | `// md:impl KeeplinServer` (+ `> fn …`) |
| 18–19 | `type SyncStreamItem`, `type SyncStreamPin` | `// md:SyncStreamItem`, `// md:SyncStreamPin` |
| 20 | `impl KeeplinService for KeeplinServer` (+ 37 RPC methods, `type SyncStream`, `fn sync`) | `// md:impl KeeplinService for KeeplinServer` (+ `> …`) |
| 21 | `fn prune_journal_after_sync` | `// md:fn prune_journal_after_sync` |
| 22 | `fn purge_resources_after_sync` | `// md:fn purge_resources_after_sync` |
| 23 | `fn stage_to_proto` | `// md:fn stage_to_proto` |
| 24 | `mod tests` (+ 3 helpers + 5 tests) | `// md:mod tests` (+ `> fn …`) |
