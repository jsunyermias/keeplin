# `server.rs` — the gRPC service implementation

Self-contained companion for `keeplin-daemon/src/server.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must
be able to understand it without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the module doc and the imports. Marker
`// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview

use std::{pin::Pin, sync::Arc};

use keeplin_core::{
    error::{StorageError, SyncError},
    format, linking,
    links::{Bookmark as CoreBookmark, LinkSource, NoteLink as CoreNoteLink},
    models::{
        now, Note as CoreNote, NoteTag, Notebook as CoreNotebook, Resource as CoreResource,
        Tag as CoreTag,
    },
    ordering,
    storage::StorageBackend,
    sync::{run_sync, SyncStage},
};
use tokio_stream::{wrappers::UnboundedReceiverStream, Stream};
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::proto::keeplin::upload_resource_request::Payload as UploadPayload;
use crate::proto::keeplin::{
    keeplin_service_server::KeeplinService, sync_progress::Stage, AddNoteLinkRequest,
    AddNoteLinkResponse, AddNoteTagRequest, AddNoteTagResponse, Bookmark as ProtoBookmark,
    CreateNoteRequest, CreateNoteResponse, CreateNotebookRequest, CreateNotebookResponse,
    CreateResourceRequest, CreateResourceResponse, CreateTagRequest, CreateTagResponse,
    DeleteNoteRequest, DeleteNoteResponse, DeleteNotebookRequest, DeleteNotebookResponse,
    DeleteResourceRequest, DeleteResourceResponse, DeleteTagRequest, DeleteTagResponse,
    GetNoteRequest, GetNoteResponse, GetNotebookRequest, GetNotebookResponse, GetResourceRequest,
    GetResourceResponse, GetTagRequest, GetTagResponse, ListAliasConflictsRequest,
    ListAliasConflictsResponse, ListBacklinksRequest, ListBacklinksResponse, ListNoteTagsRequest,
    ListNoteTagsResponse, ListNotebooksRequest, ListNotebooksResponse, ListNotesInNotebookRequest,
    ListNotesInNotebookResponse, ListNotesRequest, ListNotesResponse, ListResourcesRequest,
    ListResourcesResponse, ListStarredNotesRequest, ListStarredNotesResponse, ListTagsRequest,
    ListTagsResponse, Note, NoteAliasConflict, NoteLink as ProtoNoteLink, Notebook,
    NotebookAliasConflict, PinNoteRequest, PinNoteResponse, RemoveNoteLinkRequest,
    RemoveNoteLinkResponse, RemoveNoteTagRequest, RemoveNoteTagResponse, ReorderNotesRequest,
    ReorderNotesResponse, ResolveReferenceRequest, ResolveReferenceResponse, Resource,
    SetNoteAliasRequest, SetNoteAliasResponse, SetNotebookAliasRequest, SetNotebookAliasResponse,
    StarNoteRequest, StarNoteResponse, SyncProgress, SyncRequest, Tag, UnpinNoteRequest,
    UnpinNoteResponse, UnstarNoteRequest, UnstarNoteResponse, UpdateNoteRequest,
    UpdateNoteResponse, UpdateNotebookRequest, UpdateNotebookResponse, UpdateTagRequest,
    UpdateTagResponse, UploadResourceRequest, UploadResourceResponse,
};
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
`format`, `linking`, `sync::run_sync`, `storage::StorageBackend`). `format`
supplies the hard limit checks the note RPCs apply before writing; it expects the
same constants keeplin-srv enforces, so the two surfaces cannot disagree.

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

**Code** — complete and verbatim:

```rust
// md:fn bookmark_to_proto
fn bookmark_to_proto(b: CoreBookmark) -> ProtoBookmark {
    ProtoBookmark {
        number: b.number,
        text: b.text,
        alias: b.alias,
    }
}
```

**What it does** — Field-for-field map (`number`, `text`, `alias`).

**Used by** — `note_to_proto`.

---

## fn link_source_str

**Identification** — `fn link_source_str(s: LinkSource) -> String`. Marker
`// md:fn link_source_str`.

**Code** — complete and verbatim:

```rust
// md:fn link_source_str
fn link_source_str(s: LinkSource) -> String {
    match s {
        LinkSource::Content => "content",
        LinkSource::Manual => "manual",
    }
    .to_string()
}
```

**What it does** — `LinkSource::Content → "content"`, `Manual → "manual"` (the
wire encoding of the enum; the reverse mapping lives in `proto_to_notelink`).

**Used by** — `notelink_to_proto`.

---

## fn notelink_to_proto

**Identification** — `fn notelink_to_proto(l: CoreNoteLink) -> ProtoNoteLink`.
Marker `// md:fn notelink_to_proto`.

**Code** — complete and verbatim:

```rust
// md:fn notelink_to_proto
fn notelink_to_proto(l: CoreNoteLink) -> ProtoNoteLink {
    ProtoNoteLink {
        source: link_source_str(l.source),
        raw: l.raw,
        target_note_id: l.target_note_id.map(|u| u.to_string()),
    }
}
```

**What it does** — Maps `source` via `link_source_str`, copies `raw`, stringifies
the optional `target_note_id` UUID.

**Used by** — `note_to_proto`.

---

## fn note_to_proto

**Identification** — `fn note_to_proto(n: CoreNote) -> Note`. Marker
`// md:fn note_to_proto`.

**Code** — complete and verbatim:

```rust
// md:fn note_to_proto
fn note_to_proto(n: CoreNote) -> Note {
    Note {
        id: n.id.to_string(),
        title: n.title,
        body: n.body,
        notebook_id: (!n.notebook_id.is_nil()).then(|| n.notebook_id.to_string()),
        is_todo: n.is_todo,
        todo_due: n.todo_due.map(|d| d.to_rfc3339()),
        todo_completed: n.todo_completed.map(|d| d.to_rfc3339()),
        created_at: n.created_at.to_rfc3339(),
        updated_at: n.updated_at.to_rfc3339(),
        deleted_at: n.deleted_at.map(|d| d.to_rfc3339()),
        alias: n.alias,
        bookmarks: n.bookmarks.into_iter().map(bookmark_to_proto).collect(),
        links: n.links.into_iter().map(notelink_to_proto).collect(),
        is_pinned: n.is_pinned,
        is_starred: n.is_starred,
        sort_key: n.sort_key,
    }
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn notebook_to_proto
fn notebook_to_proto(nb: CoreNotebook) -> Notebook {
    Notebook {
        id: nb.id.to_string(),
        title: nb.title,
        created_at: nb.created_at.to_rfc3339(),
        updated_at: nb.updated_at.to_rfc3339(),
        deleted_at: nb.deleted_at.map(|d| d.to_rfc3339()),
        alias: nb.alias,
    }
}
```

**What it does** — Field map with RFC-3339 timestamps; `vv`/`last_writer` not
exposed.

**Used by** — the notebook RPCs, `list_alias_conflicts`, tests.

---

## fn resource_to_proto

**Identification** — `fn resource_to_proto(r: CoreResource) -> Resource`. Marker
`// md:fn resource_to_proto`.

**Code** — complete and verbatim:

```rust
// md:fn resource_to_proto
fn resource_to_proto(r: CoreResource) -> Resource {
    Resource {
        id: r.id.to_string(),
        note_id: r.note_id.to_string(),
        title: r.title,
        mime_type: r.mime_type,
        file_name: r.file_name,
        size: r.size as i64,
        created_at: r.created_at.to_rfc3339(),
        duration_ms: r.duration_ms,
        width: r.dimensions.map(|(w, _)| w),
        height: r.dimensions.map(|(_, h)| h),
    }
}
```

The media metadata is copied straight through (the daemon transports `duration_ms` and
`dimensions` without interpreting them); `dimensions` is split back into the proto's separate
optional `width`/`height`.

**What it does** — Metadata-only map (`id`, `title`, `mime_type`, `file_name`,
`size as i64`, `created_at`); payload bytes travel separately.

**Used by** — the resource RPCs and `assemble_upload`.

---

## fn tag_to_proto

**Identification** — `fn tag_to_proto(t: CoreTag) -> Tag`. Marker
`// md:fn tag_to_proto`.

**Code** — complete and verbatim:

```rust
// md:fn tag_to_proto
fn tag_to_proto(t: CoreTag) -> Tag {
    Tag {
        id: t.id.to_string(),
        title: t.title,
        created_at: t.created_at.to_rfc3339(),
        updated_at: t.updated_at.to_rfc3339(),
        deleted_at: t.deleted_at.map(|d| d.to_rfc3339()),
        system: t.system,
    }
}
```

**What it does** — Field map with RFC-3339 timestamps. `system` is copied straight through
(the daemon transports the internal-function flag without interpreting it).

**Used by** — the tag RPCs and tests.

---

## fn storage_err

**Identification** — `fn storage_err(e: StorageError) -> Status`. Marker
`// md:fn storage_err`.

**Code** — complete and verbatim:

```rust
// md:fn storage_err
fn storage_err(e: StorageError) -> Status {
    match &e {
        StorageError::NotFound(_) => Status::not_found(e.to_string()),
        StorageError::CorruptedData(_) => Status::data_loss(e.to_string()),
        StorageError::Conflict(_) => Status::already_exists(e.to_string()),
        StorageError::InvalidInput(_) => Status::invalid_argument(e.to_string()),
        StorageError::TooLarge(_) => Status::out_of_range(e.to_string()),
        _ => Status::internal(e.to_string()),
    }
}
```

**What it does** — The single `StorageError` → gRPC `Status` mapping used by every
RPC:

| `StorageError` | gRPC code | Why |
|---|---|---|
| `NotFound` | `NOT_FOUND` | client can distinguish "does not exist" from failure |
| `CorruptedData` | `DATA_LOSS` | AES-GCM tag failure (wrong key / tampered ciphertext): data exists but cannot be recovered in a trustworthy form |
| `Conflict` | `ALREADY_EXISTS` | duplicate alias or similar uniqueness violation |
| `InvalidInput` | `INVALID_ARGUMENT` | domain-rule rejection (pin an Inbox note, out-of-band sort key, …) |
| `TooLarge` | `OUT_OF_RANGE` | a hard format limit was exceeded — line > 4096 bytes, note > 65 536 lines, notebook already at 2²⁴ notes. `OUT_OF_RANGE` rather than `INVALID_ARGUMENT` because the request is well-formed and only the magnitude is wrong; it is the gRPC counterpart of the REST surface's 413 |
| everything else | `INTERNAL` | general server failure |

**Used by** — every RPC handler in this file and the `sync` error path.

---

## fn parse_uuid

**Identification** — `#[allow(clippy::result_large_err)] fn parse_uuid(s: &str,
field: &str) -> Result<Uuid, Status>`. Marker `// md:fn parse_uuid`.

**Code** — complete and verbatim:

```rust
// md:fn parse_uuid
#[allow(clippy::result_large_err)]
fn parse_uuid(s: &str, field: &str) -> Result<Uuid, Status> {
    s.parse::<Uuid>()
        .map_err(|_| Status::invalid_argument(format!("{field} is not a valid UUID")))
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn ensure_not_deleted
#[allow(clippy::result_large_err)]
fn ensure_not_deleted<T>(
    read: Result<T, StorageError>,
    id: Uuid,
    deleted_at: impl Fn(&T) -> Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(), Status> {
    let entity = read.map_err(storage_err)?;
    if deleted_at(&entity).is_some() {
        return Err(Status::not_found(id.to_string()));
    }
    Ok(())
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn parse_optional_dt
#[allow(clippy::result_large_err)]
fn parse_optional_dt(s: Option<String>) -> Result<Option<chrono::DateTime<chrono::Utc>>, Status> {
    match s {
        None => Ok(None),
        Some(v) => v
            .parse::<chrono::DateTime<chrono::Utc>>()
            .map(Some)
            .map_err(|_| {
                Status::invalid_argument(format!("{v} is not a valid RFC-3339 timestamp"))
            }),
    }
}
```

**What it does** — `None → Ok(None)`; `Some(v)` parses as RFC-3339 or fails with
`INVALID_ARGUMENT` quoting the bad value.

**Used by** — `proto_to_note`, `create_note`, `update_notebook`, `update_tag`.

---

## fn proto_to_note

**Identification** — `#[allow(clippy::result_large_err)] fn proto_to_note(n:
Note) -> Result<CoreNote, Status>`. Marker `// md:fn proto_to_note`.

**Code** — complete and verbatim:

```rust
// md:fn proto_to_note
#[allow(clippy::result_large_err)]
fn proto_to_note(n: Note) -> Result<CoreNote, Status> {
    Ok(CoreNote {
        id: parse_uuid(&n.id, "id")?,
        title: n.title,
        body: n.body,
        notebook_id: n
            .notebook_id
            .filter(|s| !s.is_empty())
            .map(|s| parse_uuid(&s, "notebook_id"))
            .transpose()?
            .unwrap_or_else(uuid::Uuid::nil),
        is_todo: n.is_todo,
        todo_due: parse_optional_dt(n.todo_due)?,
        todo_completed: parse_optional_dt(n.todo_completed)?,
        created_at: n
            .created_at
            .parse::<chrono::DateTime<chrono::Utc>>()
            .map_err(|_| Status::invalid_argument("created_at is invalid"))?,
        updated_at: n
            .updated_at
            .parse::<chrono::DateTime<chrono::Utc>>()
            .map_err(|_| Status::invalid_argument("updated_at is invalid"))?,
        deleted_at: parse_optional_dt(n.deleted_at)?,
        alias: n.alias,
        bookmarks: n.bookmarks.into_iter().map(proto_to_bookmark).collect(),
        links: n.links.into_iter().map(proto_to_notelink).collect(),
        vv: Default::default(),
        last_writer: String::new(),
        is_pinned: n.is_pinned,
        is_starred: n.is_starred,
        sort_key: n.sort_key,
    })
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn proto_to_bookmark
fn proto_to_bookmark(b: ProtoBookmark) -> CoreBookmark {
    CoreBookmark {
        number: b.number,
        text: b.text,
        alias: b.alias,
    }
}
```

**What it does** — Field-for-field map back to the core type.

**Used by** — `proto_to_note`.

---

## fn proto_to_notelink

**Identification** — `fn proto_to_notelink(l: ProtoNoteLink) -> CoreNoteLink`.
Marker `// md:fn proto_to_notelink`.

**Code** — complete and verbatim:

```rust
// md:fn proto_to_notelink
fn proto_to_notelink(l: ProtoNoteLink) -> CoreNoteLink {
    CoreNoteLink {
        source: if l.source == "manual" {
            LinkSource::Manual
        } else {
            LinkSource::Content
        },
        raw: l.raw,
        target_note_id: l.target_note_id.and_then(|s| s.parse().ok()),
    }
}
```

**What it does** — Any `source` value other than `"manual"` is treated as
content-derived (the default); an unparsable `target_note_id` becomes `None`
rather than an error.

**Used by** — `proto_to_note`.

---

## KeeplinServer

**Identification** — `pub struct KeeplinServer<B: StorageBackend>`. Marker
`// md:KeeplinServer`.

**Code** — complete and verbatim:

```rust
// md:KeeplinServer
pub struct KeeplinServer<B: StorageBackend> {
    backend: Arc<B>,
    journal_retention_days: u64,
    resource_purge_days: u64,
    max_upload_bytes: usize,
}
```

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

**Code** — container: members documented as sub-blocks below: fn from_shared, fn assemble_upload.

### fn from_shared

**Identification** — `pub fn from_shared(backend: Arc<B>,
journal_retention_days: u64, resource_purge_days: u64, max_upload_bytes: usize)
-> Self`; marker `// md:impl KeeplinServer > fn from_shared`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinServer > fn from_shared
    pub fn from_shared(
        backend: Arc<B>,
        journal_retention_days: u64,
        resource_purge_days: u64,
        max_upload_bytes: usize,
    ) -> Self {
        Self {
            backend,
            journal_retention_days,
            resource_purge_days,
            max_upload_bytes,
        }
    }
```

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

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinServer > fn assemble_upload
    #[allow(clippy::result_large_err)]
    async fn assemble_upload<S>(
        &self,
        mut stream: S,
    ) -> Result<Response<UploadResourceResponse>, Status>
    where
        S: tokio_stream::Stream<Item = Result<UploadResourceRequest, Status>> + Unpin,
    {
        use tokio_stream::StreamExt;

        let first = stream
            .next()
            .await
            .transpose()?
            .ok_or_else(|| Status::invalid_argument("upload stream was empty"))?;
        let meta = match first.payload {
            Some(UploadPayload::Meta(m)) => m,
            _ => {
                return Err(Status::invalid_argument(
                    "the first UploadResource frame must be resource metadata",
                ))
            }
        };

        let mut data: Vec<u8> = Vec::new();
        while let Some(frame) = stream.next().await.transpose()? {
            match frame.payload {
                Some(UploadPayload::Chunk(bytes)) => {
                    if self.max_upload_bytes != 0
                        && data.len().saturating_add(bytes.len()) > self.max_upload_bytes
                    {
                        return Err(Status::resource_exhausted(format!(
                            "upload exceeds max_upload_bytes ({})",
                            self.max_upload_bytes
                        )));
                    }
                    data.extend_from_slice(&bytes);
                }
                Some(UploadPayload::Meta(_)) => {
                    return Err(Status::invalid_argument(
                        "unexpected metadata frame in the middle of an upload stream",
                    ))
                }
                None => {}
            }
        }

        let size = data.len() as u64;
        let note_id = parse_uuid(&meta.note_id, "note_id")?;
        let mut resource =
            CoreResource::new(note_id, meta.title, meta.mime_type, meta.file_name, size);
        resource.duration_ms = meta.duration_ms;
        resource.dimensions = match (meta.width, meta.height) {
            (Some(w), Some(h)) => Some((w, h)),
            _ => None,
        };
        let created = self
            .backend
            .create_resource(resource, data)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(UploadResourceResponse {
            resource: Some(resource_to_proto(created)),
        }))
    }
```

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

**Code** — complete and verbatim:

```rust
// md:SyncStreamItem
type SyncStreamItem = Result<SyncProgress, Status>;
```

**What it does** — Item type of the `Sync` response stream.

**Used by** — `SyncStreamPin`, the `sync` RPC's channel.

---

## SyncStreamPin

**Identification** — `type SyncStreamPin = Pin<Box<dyn Stream<Item =
SyncStreamItem> + Send>>`. Marker `// md:SyncStreamPin`.

**Code** — complete and verbatim:

```rust
// md:SyncStreamPin
type SyncStreamPin = Pin<Box<dyn Stream<Item = SyncStreamItem> + Send>>;
```

**What it does** — The boxed, pinned stream type tonic requires for a
server-streaming response.

**Used by** — the associated `type SyncStream` and the `sync` RPC.

---

## impl KeeplinService for KeeplinServer

**Identification** — `#[tonic::async_trait] impl<B: StorageBackend>
KeeplinService for KeeplinServer<B>`. Marker
`// md:impl KeeplinService for KeeplinServer`; every item below carries
`// md:impl KeeplinService for KeeplinServer > <item>`.

**Code** — container: members documented as sub-blocks below: fn list_notes, fn create_note, fn get_note, fn update_note, fn delete_note, fn list_notes_in_notebook, fn list_starred_notes, fn pin_note, fn unpin_note, fn star_note, fn unstar_note, fn reorder_notes, fn list_notebooks, fn create_notebook, fn get_notebook, fn update_notebook, fn delete_notebook, fn list_tags, fn create_tag, fn add_note_tag, fn remove_note_tag, fn get_tag, fn update_tag, fn delete_tag, fn list_note_tags, fn list_resources, fn create_resource, fn upload_resource, fn get_resource, fn delete_resource, fn set_note_alias, fn set_notebook_alias, fn add_note_link, fn remove_note_link, fn list_backlinks, fn resolve_reference, fn list_alias_conflicts, type SyncStream, fn sync.

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

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn list_notes
    async fn list_notes(
        &self,
        req: Request<ListNotesRequest>,
    ) -> Result<Response<ListNotesResponse>, Status> {
        let r = req.into_inner();
        let token = if r.page_token.is_empty() {
            None
        } else {
            Some(r.page_token)
        };
        let (notes, next_page_token) = self
            .backend
            .list_notes(r.page_size, token)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(ListNotesResponse {
            notes: notes.into_iter().map(note_to_proto).collect(),
            next_page_token: next_page_token.unwrap_or_default(),
        }))
    }
```

**What it does** — Paginated `backend.list_notes` (soft-deleted excluded by the
backend contract).

### fn create_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn create_note`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn create_note
    async fn create_note(
        &self,
        req: Request<CreateNoteRequest>,
    ) -> Result<Response<CreateNoteResponse>, Status> {
        let r = req.into_inner();
        let mut note = CoreNote::new(r.title, r.body);
        note.is_todo = r.is_todo;
        note.todo_due = parse_optional_dt(if r.todo_due.is_empty() {
            None
        } else {
            Some(r.todo_due)
        })?;
        if !r.notebook_id.is_empty() {
            note.notebook_id = parse_uuid(&r.notebook_id, "notebook_id")?;
        }
        format::check_body(&note.body)
            .map_err(StorageError::from)
            .map_err(storage_err)?;
        ordering::place_new_note(self.backend.as_ref(), &mut note)
            .await
            .map_err(storage_err)?;
        let created = self.backend.create_note(note).await.map_err(storage_err)?;
        Ok(Response::new(CreateNoteResponse {
            note: Some(note_to_proto(created)),
        }))
    }
```

**What it does** — Builds `CoreNote::new(title, body)`, applies `is_todo` and an
optional `todo_due` (empty string = absent), parses a non-empty `notebook_id`
(absent → the Inbox nil UUID from `CoreNote::new`). `format::check_body` then
enforces the hard format limits (≤ 4096 bytes per line, ≤ 65 536 lines) and fails
the RPC with `OUT_OF_RANGE` instead of storing an over-sized note — the gRPC twin
of the REST surface's 413, so both entry points reject the same content. Finally
`ordering::place_new_note` gives the note its initial manual position — top of
the Inbox, or the end of a normal notebook's unpinned band — and refuses the note
when the destination notebook is already at `format::MAX_NOTES_PER_NOTEBOOK` live
notes, before `backend.create_note`.

### fn get_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn get_note`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn get_note
    async fn get_note(
        &self,
        req: Request<GetNoteRequest>,
    ) -> Result<Response<GetNoteResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        let note = self.backend.read_note(id).await.map_err(storage_err)?;
        Ok(Response::new(GetNoteResponse {
            note: Some(note_to_proto(note)),
        }))
    }
```

**What it does** — `backend.read_note(id)`. **Serves tombstones**: a soft-deleted
note is returned with its `deleted_at` set (sync needs to read tombstones); it is
the update path that answers `NOT_FOUND`.

### fn update_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn update_note`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn update_note
    async fn update_note(
        &self,
        req: Request<UpdateNoteRequest>,
    ) -> Result<Response<UpdateNoteResponse>, Status> {
        let note_proto = req
            .into_inner()
            .note
            .ok_or_else(|| Status::invalid_argument("note is required"))?;
        let mut note = proto_to_note(note_proto)?;
        let stored = self.backend.read_note(note.id).await.map_err(storage_err)?;
        if stored.deleted_at.is_some() {
            return Err(Status::not_found(note.id.to_string()));
        }
        format::check_body(&note.body)
            .map_err(StorageError::from)
            .map_err(storage_err)?;
        ordering::reconcile_notebook_move(self.backend.as_ref(), stored.notebook_id, &mut note)
            .await
            .map_err(storage_err)?;
        note.updated_at = now();
        let updated = self.backend.update_note(note).await.map_err(storage_err)?;
        Ok(Response::new(UpdateNoteResponse {
            note: Some(note_to_proto(updated)),
        }))
    }
```

**What it does** — Requires the `note` message (`INVALID_ARGUMENT` if absent);
`proto_to_note`; reads the stored note and rejects the update with `NOT_FOUND`
when it is tombstoned (an update whose body defaults `deleted_at` to none would
silently revive it — revival is reserved for sync's `apply_change`).
`format::check_body` rejects an over-limit body with `OUT_OF_RANGE` before
anything is written. Then
`ordering::reconcile_notebook_move(stored.notebook_id, &mut note)`: moving the
note to a different notebook re-places it (its old position and pinned state
belonged to the source notebook), subject to the destination's notes-per-notebook
cap; a plain edit keeps its position. Stamps
`updated_at = now()` server-side and calls `backend.update_note`.

### fn delete_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn delete_note`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn delete_note
    async fn delete_note(
        &self,
        req: Request<DeleteNoteRequest>,
    ) -> Result<Response<DeleteNoteResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        self.backend.delete_note(id).await.map_err(storage_err)?;
        Ok(Response::new(DeleteNoteResponse {}))
    }
```

**What it does** — `backend.delete_note(id)` (soft delete: tombstone with
`deleted_at`, so the deletion syncs).

### fn list_notes_in_notebook

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_notes_in_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn list_notes_in_notebook
    async fn list_notes_in_notebook(
        &self,
        req: Request<ListNotesInNotebookRequest>,
    ) -> Result<Response<ListNotesInNotebookResponse>, Status> {
        let r = req.into_inner();
        let notebook_id = parse_uuid(&r.notebook_id, "notebook_id")?;
        let token = if r.page_token.is_empty() {
            None
        } else {
            Some(r.page_token)
        };
        let (notes, next) = self
            .backend
            .list_notes_in_notebook(notebook_id, r.page_size, token)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(ListNotesInNotebookResponse {
            notes: notes.into_iter().map(note_to_proto).collect(),
            next_page_token: next.unwrap_or_default(),
        }))
    }
```

**What it does** — Paginated `backend.list_notes_in_notebook(notebook_id, …)` —
manual order: pinned band first (`sort_key 1..=999`), then the unpinned band
(`>= 1000`). Pass the nil UUID to list the Inbox.

### fn list_starred_notes

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_starred_notes`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn list_starred_notes
    async fn list_starred_notes(
        &self,
        req: Request<ListStarredNotesRequest>,
    ) -> Result<Response<ListStarredNotesResponse>, Status> {
        let r = req.into_inner();
        let token = if r.page_token.is_empty() {
            None
        } else {
            Some(r.page_token)
        };
        let (notes, next) = self
            .backend
            .list_starred_notes(r.page_size, token)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(ListStarredNotesResponse {
            notes: notes.into_iter().map(note_to_proto).collect(),
            next_page_token: next.unwrap_or_default(),
        }))
    }
```

**What it does** — Paginated `backend.list_starred_notes` (the cross-notebook
starred view).

### fn pin_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn pin_note`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn pin_note
    async fn pin_note(
        &self,
        req: Request<PinNoteRequest>,
    ) -> Result<Response<PinNoteResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        let note = ordering::pin_note(self.backend.as_ref(), id)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(PinNoteResponse {
            note: Some(note_to_proto(note)),
        }))
    }
```

**What it does** — `ordering::pin_note`: moves the note into its notebook's
pinned band (`sort_key 1..=999`). Pinning an Inbox note is a domain-rule
rejection (`InvalidInput` → `INVALID_ARGUMENT`); a full band is `Conflict`.

### fn unpin_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn unpin_note`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn unpin_note
    async fn unpin_note(
        &self,
        req: Request<UnpinNoteRequest>,
    ) -> Result<Response<UnpinNoteResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        let note = ordering::unpin_note(self.backend.as_ref(), id)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(UnpinNoteResponse {
            note: Some(note_to_proto(note)),
        }))
    }
```

**What it does** — `ordering::unpin_note`: back to the unpinned band
(`NORMAL_START = 1000` onwards).

### fn star_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn star_note`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn star_note
    async fn star_note(
        &self,
        req: Request<StarNoteRequest>,
    ) -> Result<Response<StarNoteResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        let note = ordering::star_note(self.backend.as_ref(), id)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(StarNoteResponse {
            note: Some(note_to_proto(note)),
        }))
    }
```

**What it does** — `ordering::star_note` (sets the flag; no reordering).

### fn unstar_note

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn unstar_note`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn unstar_note
    async fn unstar_note(
        &self,
        req: Request<UnstarNoteRequest>,
    ) -> Result<Response<UnstarNoteResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        let note = ordering::unstar_note(self.backend.as_ref(), id)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(UnstarNoteResponse {
            note: Some(note_to_proto(note)),
        }))
    }
```

**What it does** — `ordering::unstar_note`.

### fn reorder_notes

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn reorder_notes`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn reorder_notes
    async fn reorder_notes(
        &self,
        req: Request<ReorderNotesRequest>,
    ) -> Result<Response<ReorderNotesResponse>, Status> {
        let mut notes = Vec::new();
        for order in req.into_inner().orders {
            let id = parse_uuid(&order.note_id, "note_id")?;
            let note = ordering::reorder_note(self.backend.as_ref(), id, order.sort_key)
                .await
                .map_err(storage_err)?;
            notes.push(note_to_proto(note));
        }
        Ok(Response::new(ReorderNotesResponse { notes }))
    }
```

**What it does** — Applies each `(note_id, sort_key)` order **in request order**
via `ordering::reorder_note`; the first failure aborts the rest. Every move
already applied is durable, and re-sending the whole batch is idempotent.
Responds with the updated notes.

### fn list_notebooks

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_notebooks`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn list_notebooks
    async fn list_notebooks(
        &self,
        req: Request<ListNotebooksRequest>,
    ) -> Result<Response<ListNotebooksResponse>, Status> {
        let r = req.into_inner();
        let token = if r.page_token.is_empty() {
            None
        } else {
            Some(r.page_token)
        };
        let (notebooks, next_page_token) = self
            .backend
            .list_notebooks(r.page_size, token)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(ListNotebooksResponse {
            notebooks: notebooks.into_iter().map(notebook_to_proto).collect(),
            next_page_token: next_page_token.unwrap_or_default(),
        }))
    }
```

**What it does** — Paginated `backend.list_notebooks`.

### fn create_notebook

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn create_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn create_notebook
    async fn create_notebook(
        &self,
        req: Request<CreateNotebookRequest>,
    ) -> Result<Response<CreateNotebookResponse>, Status> {
        let notebook = CoreNotebook::new(req.into_inner().title);
        let created = self
            .backend
            .create_notebook(notebook)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(CreateNotebookResponse {
            notebook: Some(notebook_to_proto(created)),
        }))
    }
```

**What it does** — `backend.create_notebook(CoreNotebook::new(title))`.

### fn get_notebook

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn get_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn get_notebook
    async fn get_notebook(
        &self,
        req: Request<GetNotebookRequest>,
    ) -> Result<Response<GetNotebookResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        let notebook = self.backend.read_notebook(id).await.map_err(storage_err)?;
        Ok(Response::new(GetNotebookResponse {
            notebook: Some(notebook_to_proto(notebook)),
        }))
    }
```

**What it does** — `backend.read_notebook(id)`; serves tombstones (see
`get_note`).

### fn update_notebook

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn update_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn update_notebook
    async fn update_notebook(
        &self,
        req: Request<UpdateNotebookRequest>,
    ) -> Result<Response<UpdateNotebookResponse>, Status> {
        let nb = req
            .into_inner()
            .notebook
            .ok_or_else(|| Status::invalid_argument("notebook is required"))?;
        let notebook = CoreNotebook {
            id: parse_uuid(&nb.id, "id")?,
            title: nb.title,
            created_at: nb
                .created_at
                .parse()
                .map_err(|_| Status::invalid_argument("created_at is invalid"))?,
            updated_at: now(),
            deleted_at: parse_optional_dt(nb.deleted_at)?,
            alias: nb.alias,
            vv: Default::default(),
            last_writer: String::new(),
        };
        ensure_not_deleted(
            self.backend.read_notebook(notebook.id).await,
            notebook.id,
            |nb| nb.deleted_at,
        )?;
        let updated = self
            .backend
            .update_notebook(notebook)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(UpdateNotebookResponse {
            notebook: Some(notebook_to_proto(updated)),
        }))
    }
```

**What it does** — Requires the `notebook` message; parses fields;
**`updated_at = now()` server-side**, ignoring any client-supplied value, so
listings ordered by `updated_at` reflect the edit and a client cannot
back/post-date it — matching `update_note` and the REST endpoints (issue #75).
`ensure_not_deleted` guards against tombstone revival; then
`backend.update_notebook`.

### fn delete_notebook

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn delete_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn delete_notebook
    async fn delete_notebook(
        &self,
        req: Request<DeleteNotebookRequest>,
    ) -> Result<Response<DeleteNotebookResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        if ordering::is_inbox(id) {
            return Err(Status::invalid_argument(
                "the Inbox system notebook cannot be deleted",
            ));
        }
        self.backend
            .delete_notebook(id)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(DeleteNotebookResponse {}))
    }
```

**What it does** — **The Inbox system notebook (nil UUID) cannot be deleted** —
`ordering::is_inbox(id)` → `INVALID_ARGUMENT`. Otherwise
`backend.delete_notebook` (soft delete; the backend moves the notebook's notes to
the Inbox per the storage contract).

### fn list_tags

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_tags`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn list_tags
    async fn list_tags(
        &self,
        req: Request<ListTagsRequest>,
    ) -> Result<Response<ListTagsResponse>, Status> {
        let r = req.into_inner();
        let token = if r.page_token.is_empty() {
            None
        } else {
            Some(r.page_token)
        };
        let (tags, next_page_token) = self
            .backend
            .list_tags(r.page_size, token)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(ListTagsResponse {
            tags: tags.into_iter().map(tag_to_proto).collect(),
            next_page_token: next_page_token.unwrap_or_default(),
        }))
    }
```

**What it does** — Paginated `backend.list_tags`.

### fn create_tag

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn create_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn create_tag
    async fn create_tag(
        &self,
        req: Request<CreateTagRequest>,
    ) -> Result<Response<CreateTagResponse>, Status> {
        let tag = CoreTag::new(req.into_inner().title);
        let created = self.backend.create_tag(tag).await.map_err(storage_err)?;
        Ok(Response::new(CreateTagResponse {
            tag: Some(tag_to_proto(created)),
        }))
    }
```

**What it does** — `backend.create_tag(CoreTag::new(title))`.

### fn add_note_tag

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn add_note_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn add_note_tag
    async fn add_note_tag(
        &self,
        req: Request<AddNoteTagRequest>,
    ) -> Result<Response<AddNoteTagResponse>, Status> {
        let r = req.into_inner();
        self.backend
            .add_note_tag(NoteTag {
                note_id: parse_uuid(&r.note_id, "note_id")?,
                tag_id: parse_uuid(&r.tag_id, "tag_id")?,
            })
            .await
            .map_err(storage_err)?;
        Ok(Response::new(AddNoteTagResponse {}))
    }
```

**What it does** — `backend.add_note_tag(NoteTag { note_id, tag_id })` (both
UUIDs parsed; idempotent at the storage layer).

### fn remove_note_tag

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn remove_note_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn remove_note_tag
    async fn remove_note_tag(
        &self,
        req: Request<RemoveNoteTagRequest>,
    ) -> Result<Response<RemoveNoteTagResponse>, Status> {
        let r = req.into_inner();
        self.backend
            .remove_note_tag(
                parse_uuid(&r.note_id, "note_id")?,
                parse_uuid(&r.tag_id, "tag_id")?,
            )
            .await
            .map_err(storage_err)?;
        Ok(Response::new(RemoveNoteTagResponse {}))
    }
```

**What it does** — `backend.remove_note_tag(note_id, tag_id)`.

### fn get_tag

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn get_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn get_tag
    async fn get_tag(
        &self,
        req: Request<GetTagRequest>,
    ) -> Result<Response<GetTagResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        let tag = self.backend.read_tag(id).await.map_err(storage_err)?;
        Ok(Response::new(GetTagResponse {
            tag: Some(tag_to_proto(tag)),
        }))
    }
```

**What it does** — `backend.read_tag(id)`; serves tombstones.

### fn update_tag

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn update_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn update_tag
    async fn update_tag(
        &self,
        req: Request<UpdateTagRequest>,
    ) -> Result<Response<UpdateTagResponse>, Status> {
        let t = req
            .into_inner()
            .tag
            .ok_or_else(|| Status::invalid_argument("tag is required"))?;
        let tag = CoreTag {
            id: parse_uuid(&t.id, "id")?,
            title: t.title,
            created_at: t
                .created_at
                .parse()
                .map_err(|_| Status::invalid_argument("created_at is invalid"))?,
            updated_at: now(),
            deleted_at: parse_optional_dt(t.deleted_at)?,
            vv: Default::default(),
            last_writer: String::new(),
            system: t.system,
        };
        ensure_not_deleted(self.backend.read_tag(tag.id).await, tag.id, |t| {
            t.deleted_at
        })?;
        let updated = self.backend.update_tag(tag).await.map_err(storage_err)?;
        Ok(Response::new(UpdateTagResponse {
            tag: Some(tag_to_proto(updated)),
        }))
    }
```

**What it does** — Same shape as `update_notebook`: required message, parsed
fields, server-side `updated_at = now()` (unspoofable ordering, issue #75),
`ensure_not_deleted`, then `backend.update_tag`. `system` is carried from the request
message into the core `Tag` so an update can set or clear the internal-function flag.

### fn delete_tag

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn delete_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn delete_tag
    async fn delete_tag(
        &self,
        req: Request<DeleteTagRequest>,
    ) -> Result<Response<DeleteTagResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        self.backend.delete_tag(id).await.map_err(storage_err)?;
        Ok(Response::new(DeleteTagResponse {}))
    }
```

**What it does** — `backend.delete_tag(id)` (soft delete; note-tag pairs for it
stop listing).

### fn list_note_tags

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_note_tags`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn list_note_tags
    async fn list_note_tags(
        &self,
        req: Request<ListNoteTagsRequest>,
    ) -> Result<Response<ListNoteTagsResponse>, Status> {
        let r = req.into_inner();
        let note_id = parse_uuid(&r.note_id, "note_id")?;
        let token = if r.page_token.is_empty() {
            None
        } else {
            Some(r.page_token)
        };
        let (tags, next_page_token) = self
            .backend
            .list_note_tags(note_id, r.page_size, token)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(ListNoteTagsResponse {
            tags: tags.into_iter().map(tag_to_proto).collect(),
            next_page_token: next_page_token.unwrap_or_default(),
        }))
    }
```

**What it does** — Paginated `backend.list_note_tags(note_id, …)` — the tags on
one note.

### fn list_resources

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_resources`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn list_resources
    async fn list_resources(
        &self,
        req: Request<ListResourcesRequest>,
    ) -> Result<Response<ListResourcesResponse>, Status> {
        let r = req.into_inner();
        let token = if r.page_token.is_empty() {
            None
        } else {
            Some(r.page_token)
        };
        let (resources, next_page_token) = if r.note_id.is_empty() {
            self.backend
                .list_resources(r.page_size, token)
                .await
                .map_err(storage_err)?
        } else {
            let note_id = parse_uuid(&r.note_id, "note_id")?;
            self.backend
                .list_resources_for_note(note_id, r.page_size, token)
                .await
                .map_err(storage_err)?
        };
        Ok(Response::new(ListResourcesResponse {
            resources: resources.into_iter().map(resource_to_proto).collect(),
            next_page_token: next_page_token.unwrap_or_default(),
        }))
    }
```

**What it does** — Paginated resource metadata. With an empty `note_id` it lists every
resource; with a non-empty `note_id` it filters to that note via
`list_resources_for_note` (issue #125).

### fn create_resource

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn create_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn create_resource
    async fn create_resource(
        &self,
        req: Request<CreateResourceRequest>,
    ) -> Result<Response<CreateResourceResponse>, Status> {
        let r = req.into_inner();
        let note_id = parse_uuid(&r.note_id, "note_id")?;
        let size = r.data.len() as u64;
        let mut resource = CoreResource::new(note_id, r.title, r.mime_type, r.file_name, size);
        resource.duration_ms = r.duration_ms;
        resource.dimensions = match (r.width, r.height) {
            (Some(w), Some(h)) => Some((w, h)),
            _ => None,
        };
        let created = self
            .backend
            .create_resource(resource, r.data)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(CreateResourceResponse {
            resource: Some(resource_to_proto(created)),
        }))
    }
```

**What it does** — The unary upload: payload bytes arrive in one message
(bounded by tonic's `max_decoding_message_size`); size is taken from
`data.len()`, then `backend.create_resource(CoreResource::new(…), data)`. For
attachments larger than the message limit, use `upload_resource`.

### fn upload_resource

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn upload_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn upload_resource
    async fn upload_resource(
        &self,
        req: Request<tonic::Streaming<UploadResourceRequest>>,
    ) -> Result<Response<UploadResourceResponse>, Status> {
        self.assemble_upload(req.into_inner()).await
    }
```

**What it does** — The client-streaming upload: delegates the
`tonic::Streaming<UploadResourceRequest>` to `assemble_upload` (protocol and
limits documented there).

### fn get_resource

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn get_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn get_resource
    async fn get_resource(
        &self,
        req: Request<GetResourceRequest>,
    ) -> Result<Response<GetResourceResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        let (meta, data) = self.backend.read_resource(id).await.map_err(storage_err)?;
        Ok(Response::new(GetResourceResponse {
            resource: Some(resource_to_proto(meta)),
            data,
        }))
    }
```

**What it does** — `backend.read_resource(id)` → metadata + full payload bytes in
one response.

### fn delete_resource

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn delete_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn delete_resource
    async fn delete_resource(
        &self,
        req: Request<DeleteResourceRequest>,
    ) -> Result<Response<DeleteResourceResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        self.backend
            .delete_resource(id)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(DeleteResourceResponse {}))
    }
```

**What it does** — `backend.delete_resource(id)` (tombstone; the payload is
reclaimed later by `purge_resources_after_sync`).

### fn set_note_alias

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn set_note_alias`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn set_note_alias
    async fn set_note_alias(
        &self,
        req: Request<SetNoteAliasRequest>,
    ) -> Result<Response<SetNoteAliasResponse>, Status> {
        let r = req.into_inner();
        let id = parse_uuid(&r.id, "id")?;
        let note = linking::set_note_alias(self.backend.as_ref(), id, r.alias)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(SetNoteAliasResponse {
            note: Some(note_to_proto(note)),
        }))
    }
```

**What it does** — `linking::set_note_alias(backend, id, alias)`; a duplicate
alias is `Conflict` → `ALREADY_EXISTS`; an empty alias clears it.

### fn set_notebook_alias

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn set_notebook_alias`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn set_notebook_alias
    async fn set_notebook_alias(
        &self,
        req: Request<SetNotebookAliasRequest>,
    ) -> Result<Response<SetNotebookAliasResponse>, Status> {
        let r = req.into_inner();
        let id = parse_uuid(&r.id, "id")?;
        let notebook = linking::set_notebook_alias(self.backend.as_ref(), id, r.alias)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(SetNotebookAliasResponse {
            notebook: Some(notebook_to_proto(notebook)),
        }))
    }
```

**What it does** — `linking::set_notebook_alias`; same rules as note aliases.

### fn add_note_link

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn add_note_link`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn add_note_link
    async fn add_note_link(
        &self,
        req: Request<AddNoteLinkRequest>,
    ) -> Result<Response<AddNoteLinkResponse>, Status> {
        let r = req.into_inner();
        let note_id = parse_uuid(&r.note_id, "note_id")?;
        let note = linking::add_manual_link(self.backend.as_ref(), note_id, &r.raw)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(AddNoteLinkResponse {
            note: Some(note_to_proto(note)),
        }))
    }
```

**What it does** — `linking::add_manual_link(backend, note_id, raw)`: appends a
`LinkSource::Manual` link with the raw reference text and resolves its target;
manual links survive body rewrites (only content-derived links are re-derived).

### fn remove_note_link

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn remove_note_link`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn remove_note_link
    async fn remove_note_link(
        &self,
        req: Request<RemoveNoteLinkRequest>,
    ) -> Result<Response<RemoveNoteLinkResponse>, Status> {
        let r = req.into_inner();
        let note_id = parse_uuid(&r.note_id, "note_id")?;
        let note = linking::remove_link(self.backend.as_ref(), note_id, r.index as usize)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(RemoveNoteLinkResponse {
            note: Some(note_to_proto(note)),
        }))
    }
```

**What it does** — `linking::remove_link(backend, note_id, index)` — removal by
index into the note's `links` array.

### fn list_backlinks

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_backlinks`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn list_backlinks
    async fn list_backlinks(
        &self,
        req: Request<ListBacklinksRequest>,
    ) -> Result<Response<ListBacklinksResponse>, Status> {
        let r = req.into_inner();
        let note_id = parse_uuid(&r.note_id, "note_id")?;
        let token = if r.page_token.is_empty() {
            None
        } else {
            Some(r.page_token)
        };
        let (notes, next_page_token) =
            linking::backlinks(self.backend.as_ref(), note_id, r.page_size, token)
                .await
                .map_err(storage_err)?;
        Ok(Response::new(ListBacklinksResponse {
            notes: notes.into_iter().map(note_to_proto).collect(),
            next_page_token: next_page_token.unwrap_or_default(),
        }))
    }
```

**What it does** — Paginated `linking::backlinks(backend, note_id, …)`: the notes
whose links resolve **to** this note.

### fn resolve_reference

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn resolve_reference`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn resolve_reference
    async fn resolve_reference(
        &self,
        req: Request<ResolveReferenceRequest>,
    ) -> Result<Response<ResolveReferenceResponse>, Status> {
        let resolved = linking::resolve(self.backend.as_ref(), &req.into_inner().reference)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(match resolved {
            Some(r) => ResolveReferenceResponse {
                note_id: Some(r.note_id.to_string()),
                bookmark_number: r.bookmark_number,
            },
            None => ResolveReferenceResponse {
                note_id: None,
                bookmark_number: None,
            },
        }))
    }
```

**What it does** — `linking::resolve(backend, reference)`; the response's
`note_id`/`bookmark_number` are both optional — an unresolvable reference is a
`None`/`None` response, **not** an error.

### fn list_alias_conflicts

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn list_alias_conflicts`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn list_alias_conflicts
    async fn list_alias_conflicts(
        &self,
        _req: Request<ListAliasConflictsRequest>,
    ) -> Result<Response<ListAliasConflictsResponse>, Status> {
        let conflicts = linking::alias_conflicts(self.backend.as_ref())
            .await
            .map_err(storage_err)?;
        Ok(Response::new(ListAliasConflictsResponse {
            notes: conflicts
                .notes
                .into_iter()
                .map(|c| NoteAliasConflict {
                    alias: c.alias,
                    notes: c.entities.into_iter().map(note_to_proto).collect(),
                })
                .collect(),
            notebooks: conflicts
                .notebooks
                .into_iter()
                .map(|c| NotebookAliasConflict {
                    alias: c.alias,
                    notebooks: c.entities.into_iter().map(notebook_to_proto).collect(),
                })
                .collect(),
        }))
    }
```

**What it does** — `linking::alias_conflicts(backend)` → the aliases claimed by
more than one note (and, separately, notebook), each with the conflicting
entities. Conflicts can only arise from sync merges — local writes enforce
uniqueness — so this is the repair-surface for that state.

### type SyncStream

**Identification** — associated type `type SyncStream = SyncStreamPin`; marker
`// md:impl KeeplinService for KeeplinServer > type SyncStream`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > type SyncStream
    type SyncStream = SyncStreamPin;
```

**What it does** — Binds the trait's server-streaming response type for `Sync` to
the boxed pinned stream alias.

### fn sync

**Identification** — marker
`// md:impl KeeplinService for KeeplinServer > fn sync`.

**Code** — complete and verbatim:

```rust
    // md:impl KeeplinService for KeeplinServer > fn sync
    async fn sync(&self, _req: Request<SyncRequest>) -> Result<Response<Self::SyncStream>, Status> {
        let backend = Arc::clone(&self.backend);
        let retention_days = self.journal_retention_days;
        let purge_days = self.resource_purge_days;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<SyncStreamItem>();

        tokio::spawn(async move {
            let progress_tx = tx.clone();
            let report = move |stage: SyncStage, count: usize| {
                let (proto_stage, message) = stage_to_proto(stage);
                let _ = progress_tx.send(Ok(SyncProgress {
                    stage: proto_stage as i32,
                    changes_count: count as i32,
                    message: message.to_string(),
                }));
            };

            match run_sync(&*backend, report).await {
                Ok(_) => {
                    prune_journal_after_sync(&*backend, retention_days).await;
                    purge_resources_after_sync(&*backend, purge_days).await;
                }
                Err(e) => {
                    let status = match e {
                        SyncError::Storage(se) => storage_err(se),
                        other => Status::internal(other.to_string()),
                    };
                    let _ = tx.send(Err(status));
                }
            }
        });

        Ok(Response::new(
            Box::pin(UnboundedReceiverStream::new(rx)) as SyncStreamPin
        ))
    }
```

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

**Code** — complete and verbatim:

```rust
// md:fn prune_journal_after_sync
pub(crate) async fn prune_journal_after_sync<B>(backend: &B, retention_days: u64)
where
    B: StorageBackend + ?Sized,
{
    if retention_days == 0 {
        return;
    }
    let days = retention_days.min(36_500) as i64;
    let cutoff = now() - chrono::Duration::days(days);
    if let Err(e) = backend.prune_change_journal(cutoff).await {
        tracing::warn!("change-journal prune failed: {e}");
    }
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn purge_resources_after_sync
pub(crate) async fn purge_resources_after_sync<B>(backend: &B, purge_days: u64)
where
    B: StorageBackend + ?Sized,
{
    if purge_days == 0 {
        return;
    }
    let days = purge_days.min(36_500) as i64;
    let cutoff = now() - chrono::Duration::days(days);
    if let Err(e) = backend.purge_deleted_resources(cutoff).await {
        tracing::warn!("resource payload purge failed: {e}");
    }
}
```

**What it does** — Reclaims payloads of resources tombstoned longer than
`purge_days` ago, after a successful sync cycle. `0` disables; same ~100-year
overflow clamp and non-fatal WARN failure handling as the journal prune.

**Used by** — the gRPC `sync` RPC and the REST `POST /api/sync` handler.

---

## fn stage_to_proto

**Identification** — `fn stage_to_proto(stage: SyncStage) -> (Stage, &'static
str)`. Marker `// md:fn stage_to_proto`.

**Code** — complete and verbatim:

```rust
// md:fn stage_to_proto
fn stage_to_proto(stage: SyncStage) -> (Stage, &'static str) {
    match stage {
        SyncStage::Collecting => (Stage::Collecting, "Collecting local changes"),
        SyncStage::Sending => (Stage::Sending, "Sending local changes"),
        SyncStage::Receiving => (Stage::Receiving, "Receiving remote changes"),
        SyncStage::Applying => (Stage::Applying, "Applying remote changes"),
        SyncStage::Done => (Stage::Done, "Sync complete"),
    }
}
```

**What it does** — Maps a core `SyncStage` to its protobuf `Stage` code plus a
human-readable message: Collecting/Sending/Receiving/Applying/Done →
"Collecting local changes" … "Sync complete".

**Used by** — the `sync` RPC's progress callback.

---

## mod tests

**Identification** — `#[cfg(test)] mod tests`. Marker `// md:mod tests`. Imports
`super::*`, the proto `ResourceMeta`/`UploadResourceRequest` types, `FsBackend`,
and the repository traits.

**Code** — container: members documented as sub-blocks below: imports, fn server,
fn meta_frame, fn chunk_frame, fn upload_resource_assembles_chunks_in_order,
fn upload_resource_requires_metadata_first, fn upload_resource_enforces_the_cap,
fn update_rpcs_reject_soft_deleted_entities,
fn update_notebook_and_tag_refresh_updated_at_server_side.

The explicit `imports` leaf below preserves the test-module dependency preamble
verbatim.

### imports

**Identification** — test-module dependencies; marker `// md:mod tests > imports`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > imports
    use super::*;
    use crate::proto::keeplin::{ResourceMeta, UploadResourceRequest};
    use keeplin_core::storage::fs::FsBackend;
    use keeplin_core::storage::{
        NoteRepository, NotebookRepository, ResourceRepository, TagRepository,
    };
```

**What it does** — Brings the parent module API and test-only dependencies into
scope.

### fn server

**Identification** — helper; marker `// md:mod tests > fn server`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn server
    async fn server() -> (KeeplinServer<FsBackend>, Arc<FsBackend>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let backend = Arc::new(FsBackend::new(&path).await.unwrap());
        (
            KeeplinServer::from_shared(backend.clone(), 0, 0, 1024 * 1024 * 1024),
            backend,
        )
    }
```

**What it does** — A `KeeplinServer` over a fresh `FsBackend` in a leaked temp
dir (`std::mem::forget` keeps it alive for the test), plus a handle to the
backend for seeding state directly. `max_upload_bytes` is generous (1 GiB) so
the upload tests exercise assembly, not the cap.

### fn meta_frame

**Identification** — helper; marker `// md:mod tests > fn meta_frame`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn meta_frame
    fn meta_frame(title: &str, mime: &str, file: &str) -> UploadResourceRequest {
        UploadResourceRequest {
            payload: Some(UploadPayload::Meta(ResourceMeta {
                title: title.into(),
                mime_type: mime.into(),
                file_name: file.into(),
                duration_ms: None,
                width: None,
                height: None,
                note_id: keeplin_core::models::SYSTEM_RESOURCE_NOTE_ID.to_string(),
            })),
        }
    }
```

**What it does** — Builds the metadata first-frame of an upload stream.

### fn chunk_frame

**Identification** — helper; marker `// md:mod tests > fn chunk_frame`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn chunk_frame
    fn chunk_frame(bytes: &[u8]) -> UploadResourceRequest {
        UploadResourceRequest {
            payload: Some(UploadPayload::Chunk(bytes.to_vec())),
        }
    }
```

**What it does** — Builds a payload-chunk frame.

### fn upload_resource_assembles_chunks_in_order

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn upload_resource_assembles_chunks_in_order`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn upload_resource_assembles_chunks_in_order
    #[tokio::test]
    async fn upload_resource_assembles_chunks_in_order() {
        let (srv, backend) = server().await;

        let frames = vec![
            Ok(meta_frame("pic", "image/png", "p.png")),
            Ok(chunk_frame(b"hello ")),
            Ok(chunk_frame(b"streamed ")),
            Ok(chunk_frame(b"world")),
        ];
        let resp = srv
            .assemble_upload(tokio_stream::iter(frames))
            .await
            .unwrap()
            .into_inner();
        let meta = resp.resource.unwrap();
        assert_eq!(meta.title, "pic");
        assert_eq!(meta.file_name, "p.png");
        assert_eq!(meta.size, "hello streamed world".len() as i64);

        let id = meta.id.parse().unwrap();
        let (_, data) = backend.read_resource(id).await.unwrap();
        assert_eq!(data, b"hello streamed world");
    }
```

**What it does** — A payload split across three chunks reassembles in order;
the response metadata carries the right title/file name/summed size; the
reassembled bytes round-trip through `backend.read_resource`.

### fn upload_resource_requires_metadata_first

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn upload_resource_requires_metadata_first`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn upload_resource_requires_metadata_first
    #[tokio::test]
    async fn upload_resource_requires_metadata_first() {
        let (srv, _backend) = server().await;
        let frames = vec![Ok(chunk_frame(b"data"))];
        let err = srv
            .assemble_upload(tokio_stream::iter(frames))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }
```

**What it does** — A stream starting with a chunk (no metadata frame) is
rejected with `INVALID_ARGUMENT`.

### fn upload_resource_enforces_the_cap

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn upload_resource_enforces_the_cap`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn upload_resource_enforces_the_cap
    #[tokio::test]
    async fn upload_resource_enforces_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let backend = Arc::new(FsBackend::new(&path).await.unwrap());
        let srv = KeeplinServer::from_shared(backend, 0, 0, 8);
        let frames = vec![
            Ok(meta_frame("big", "application/octet-stream", "big.bin")),
            Ok(chunk_frame(b"0123456789")),
        ];
        let err = srv
            .assemble_upload(tokio_stream::iter(frames))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::ResourceExhausted);
    }
```

**What it does** — With an explicit tiny 8-byte cap, a 10-byte payload is
refused with `RESOURCE_EXHAUSTED`.

### fn update_rpcs_reject_soft_deleted_entities

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn update_rpcs_reject_soft_deleted_entities`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn update_rpcs_reject_soft_deleted_entities
    #[tokio::test]
    async fn update_rpcs_reject_soft_deleted_entities() {
        let (srv, backend) = server().await;

        let note = backend.create_note(CoreNote::new("t", "b")).await.unwrap();
        backend.delete_note(note.id).await.unwrap();
        let mut proto = note_to_proto(note.clone());
        proto.deleted_at = None;
        let err = srv
            .update_note(Request::new(UpdateNoteRequest { note: Some(proto) }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);
        let got = srv
            .get_note(Request::new(GetNoteRequest {
                id: note.id.to_string(),
            }))
            .await
            .unwrap();
        assert!(got.into_inner().note.unwrap().deleted_at.is_some());

        let nb = backend
            .create_notebook(CoreNotebook::new("nb"))
            .await
            .unwrap();
        backend.delete_notebook(nb.id).await.unwrap();
        let mut proto = notebook_to_proto(nb);
        proto.deleted_at = None;
        let err = srv
            .update_notebook(Request::new(UpdateNotebookRequest {
                notebook: Some(proto),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);

        let tag = backend.create_tag(CoreTag::new("label")).await.unwrap();
        backend.delete_tag(tag.id).await.unwrap();
        let mut proto = tag_to_proto(tag);
        proto.deleted_at = None;
        let err = srv
            .update_tag(Request::new(UpdateTagRequest { tag: Some(proto) }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);
    }
```

**What it does** — For note, notebook, and tag: create → delete → update with a
proto carrying `deleted_at: None` must be `NOT_FOUND`, not a silent revival; and
`GetNote` still serves the tombstone (sync reads it) — unchanged by the
rejection.

### fn update_notebook_and_tag_refresh_updated_at_server_side

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn update_notebook_and_tag_refresh_updated_at_server_side`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn update_notebook_and_tag_refresh_updated_at_server_side
    #[tokio::test]
    async fn update_notebook_and_tag_refresh_updated_at_server_side() {
        let (srv, backend) = server().await;
        let stale = "2000-01-01T00:00:00Z";

        let nb = backend
            .create_notebook(CoreNotebook::new("nb"))
            .await
            .unwrap();
        let mut proto = notebook_to_proto(nb.clone());
        proto.title = "renamed".into();
        proto.updated_at = stale.into();
        let out = srv
            .update_notebook(Request::new(UpdateNotebookRequest {
                notebook: Some(proto),
            }))
            .await
            .unwrap()
            .into_inner()
            .notebook
            .unwrap();
        assert_eq!(out.title, "renamed");
        assert_ne!(out.updated_at, stale, "client updated_at must be ignored");
        assert!(
            out.updated_at
                .parse::<chrono::DateTime<chrono::Utc>>()
                .unwrap()
                > nb.updated_at,
            "updated_at must advance to server time"
        );

        let tag = backend.create_tag(CoreTag::new("label")).await.unwrap();
        let mut proto = tag_to_proto(tag.clone());
        proto.title = "renamed".into();
        proto.updated_at = stale.into();
        let out = srv
            .update_tag(Request::new(UpdateTagRequest { tag: Some(proto) }))
            .await
            .unwrap()
            .into_inner()
            .tag
            .unwrap();
        assert_eq!(out.title, "renamed");
        assert_ne!(out.updated_at, stale, "client updated_at must be ignored");
        assert!(
            out.updated_at
                .parse::<chrono::DateTime<chrono::Utc>>()
                .unwrap()
                > tag.updated_at
        );
    }
```

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
| 1 | `Overview` | `// md:Overview` |
| 2 | `fn bookmark_to_proto` | `// md:fn bookmark_to_proto` |
| 3 | `fn link_source_str` | `// md:fn link_source_str` |
| 4 | `fn notelink_to_proto` | `// md:fn notelink_to_proto` |
| 5 | `fn note_to_proto` | `// md:fn note_to_proto` |
| 6 | `fn notebook_to_proto` | `// md:fn notebook_to_proto` |
| 7 | `fn resource_to_proto` | `// md:fn resource_to_proto` |
| 8 | `fn tag_to_proto` | `// md:fn tag_to_proto` |
| 9 | `fn storage_err` | `// md:fn storage_err` |
| 10 | `fn parse_uuid` | `// md:fn parse_uuid` |
| 11 | `fn ensure_not_deleted` | `// md:fn ensure_not_deleted` |
| 12 | `fn parse_optional_dt` | `// md:fn parse_optional_dt` |
| 13 | `fn proto_to_note` | `// md:fn proto_to_note` |
| 14 | `fn proto_to_bookmark` | `// md:fn proto_to_bookmark` |
| 15 | `fn proto_to_notelink` | `// md:fn proto_to_notelink` |
| 16 | `KeeplinServer` | `// md:KeeplinServer` |
| 17 | `impl KeeplinServer` (container) | `// md:impl KeeplinServer` |
| 18 | `fn from_shared` | `// md:impl KeeplinServer > fn from_shared` |
| 19 | `fn assemble_upload` | `// md:impl KeeplinServer > fn assemble_upload` |
| 20 | `SyncStreamItem` | `// md:SyncStreamItem` |
| 21 | `SyncStreamPin` | `// md:SyncStreamPin` |
| 22 | `impl KeeplinService for KeeplinServer` (container) | `// md:impl KeeplinService for KeeplinServer` |
| 23 | `fn list_notes` | `// md:impl KeeplinService for KeeplinServer > fn list_notes` |
| 24 | `fn create_note` | `// md:impl KeeplinService for KeeplinServer > fn create_note` |
| 25 | `fn get_note` | `// md:impl KeeplinService for KeeplinServer > fn get_note` |
| 26 | `fn update_note` | `// md:impl KeeplinService for KeeplinServer > fn update_note` |
| 27 | `fn delete_note` | `// md:impl KeeplinService for KeeplinServer > fn delete_note` |
| 28 | `fn list_notes_in_notebook` | `// md:impl KeeplinService for KeeplinServer > fn list_notes_in_notebook` |
| 29 | `fn list_starred_notes` | `// md:impl KeeplinService for KeeplinServer > fn list_starred_notes` |
| 30 | `fn pin_note` | `// md:impl KeeplinService for KeeplinServer > fn pin_note` |
| 31 | `fn unpin_note` | `// md:impl KeeplinService for KeeplinServer > fn unpin_note` |
| 32 | `fn star_note` | `// md:impl KeeplinService for KeeplinServer > fn star_note` |
| 33 | `fn unstar_note` | `// md:impl KeeplinService for KeeplinServer > fn unstar_note` |
| 34 | `fn reorder_notes` | `// md:impl KeeplinService for KeeplinServer > fn reorder_notes` |
| 35 | `fn list_notebooks` | `// md:impl KeeplinService for KeeplinServer > fn list_notebooks` |
| 36 | `fn create_notebook` | `// md:impl KeeplinService for KeeplinServer > fn create_notebook` |
| 37 | `fn get_notebook` | `// md:impl KeeplinService for KeeplinServer > fn get_notebook` |
| 38 | `fn update_notebook` | `// md:impl KeeplinService for KeeplinServer > fn update_notebook` |
| 39 | `fn delete_notebook` | `// md:impl KeeplinService for KeeplinServer > fn delete_notebook` |
| 40 | `fn list_tags` | `// md:impl KeeplinService for KeeplinServer > fn list_tags` |
| 41 | `fn create_tag` | `// md:impl KeeplinService for KeeplinServer > fn create_tag` |
| 42 | `fn add_note_tag` | `// md:impl KeeplinService for KeeplinServer > fn add_note_tag` |
| 43 | `fn remove_note_tag` | `// md:impl KeeplinService for KeeplinServer > fn remove_note_tag` |
| 44 | `fn get_tag` | `// md:impl KeeplinService for KeeplinServer > fn get_tag` |
| 45 | `fn update_tag` | `// md:impl KeeplinService for KeeplinServer > fn update_tag` |
| 46 | `fn delete_tag` | `// md:impl KeeplinService for KeeplinServer > fn delete_tag` |
| 47 | `fn list_note_tags` | `// md:impl KeeplinService for KeeplinServer > fn list_note_tags` |
| 48 | `fn list_resources` | `// md:impl KeeplinService for KeeplinServer > fn list_resources` |
| 49 | `fn create_resource` | `// md:impl KeeplinService for KeeplinServer > fn create_resource` |
| 50 | `fn upload_resource` | `// md:impl KeeplinService for KeeplinServer > fn upload_resource` |
| 51 | `fn get_resource` | `// md:impl KeeplinService for KeeplinServer > fn get_resource` |
| 52 | `fn delete_resource` | `// md:impl KeeplinService for KeeplinServer > fn delete_resource` |
| 53 | `fn set_note_alias` | `// md:impl KeeplinService for KeeplinServer > fn set_note_alias` |
| 54 | `fn set_notebook_alias` | `// md:impl KeeplinService for KeeplinServer > fn set_notebook_alias` |
| 55 | `fn add_note_link` | `// md:impl KeeplinService for KeeplinServer > fn add_note_link` |
| 56 | `fn remove_note_link` | `// md:impl KeeplinService for KeeplinServer > fn remove_note_link` |
| 57 | `fn list_backlinks` | `// md:impl KeeplinService for KeeplinServer > fn list_backlinks` |
| 58 | `fn resolve_reference` | `// md:impl KeeplinService for KeeplinServer > fn resolve_reference` |
| 59 | `fn list_alias_conflicts` | `// md:impl KeeplinService for KeeplinServer > fn list_alias_conflicts` |
| 60 | `type SyncStream` | `// md:impl KeeplinService for KeeplinServer > type SyncStream` |
| 61 | `fn sync` | `// md:impl KeeplinService for KeeplinServer > fn sync` |
| 62 | `fn prune_journal_after_sync` | `// md:fn prune_journal_after_sync` |
| 63 | `fn purge_resources_after_sync` | `// md:fn purge_resources_after_sync` |
| 64 | `fn stage_to_proto` | `// md:fn stage_to_proto` |
| 65 | `mod tests` (container) | `// md:mod tests` |
| 66 | `imports` | `// md:mod tests > imports` |
| 67 | `fn server` | `// md:mod tests > fn server` |
| 68 | `fn meta_frame` | `// md:mod tests > fn meta_frame` |
| 69 | `fn chunk_frame` | `// md:mod tests > fn chunk_frame` |
| 70 | `fn upload_resource_assembles_chunks_in_order` | `// md:mod tests > fn upload_resource_assembles_chunks_in_order` |
| 71 | `fn upload_resource_requires_metadata_first` | `// md:mod tests > fn upload_resource_requires_metadata_first` |
| 72 | `fn upload_resource_enforces_the_cap` | `// md:mod tests > fn upload_resource_enforces_the_cap` |
| 73 | `fn update_rpcs_reject_soft_deleted_entities` | `// md:mod tests > fn update_rpcs_reject_soft_deleted_entities` |
| 74 | `fn update_notebook_and_tag_refresh_updated_at_server_side` | `// md:mod tests > fn update_notebook_and_tag_refresh_updated_at_server_side` |
