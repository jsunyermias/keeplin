# `src/server.rs` — gRPC service implementation

## Purpose

This module implements `KeeplinServer<B>`, which satisfies the `KeeplinService` trait
generated from `keeplin.proto`. It bridges between the protobuf wire types
(`proto::keeplin::Note`, etc.) and the domain types in `keeplin-core` (`models::Note`,
etc.), and delegates all persistence to a generic `StorageBackend`.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `KeeplinServer<B>` | struct | Holds a shared reference to the backend; implements `KeeplinService` |
| `SyncStreamItem` | type alias | `Result<SyncProgress, Status>` — items yielded by the sync stream |
| `SyncStreamPin` | type alias | `Pin<Box<dyn Stream<Item = SyncStreamItem> + Send>>` — the opaque stream type |

## Conversion helpers (module-private)

These functions are stateless and have no error path other than parsing.

| Function | Description |
|----------|-------------|
| `note_to_proto(CoreNote) -> Note` | Converts a domain `Note` to a protobuf `Note`; all `Option` fields become empty strings when absent |
| `notebook_to_proto(CoreNotebook) -> Notebook` | Same pattern for notebooks |
| `resource_to_proto(CoreResource) -> Resource` | Same pattern for resources; `size: u64` becomes `size: i64` (proto3 has no unsigned integers) |
| `tag_to_proto(CoreTag) -> Tag` | Same pattern for tags |
| `storage_err(StorageError) -> Status` | Maps `StorageError::NotFound` to `Status::not_found` and everything else to `Status::internal` |
| `parse_uuid(&str, field_name) -> Result<Uuid, Status>` | Parses a UUID string from a proto field; returns `Status::invalid_argument` if malformed |
| `parse_optional_dt(&str) -> Result<Option<DateTime<Utc>>, Status>` | Parses an RFC-3339 timestamp; returns `None` for empty strings |
| `proto_to_note(Note) -> Result<CoreNote, Status>` | Full conversion from protobuf to domain `Note`; validates all timestamp and UUID fields |

## Public API

### `KeeplinServer::new(backend: B) -> Self`
**What it does:** Wraps the backend in an `Arc` so it can be shared across concurrent
gRPC handler tasks (tonic calls handlers from a thread pool).  
**Parameters:** `backend` — any value implementing `StorageBackend`.  
**Returns:** A server instance ready to be registered with a tonic `Server`.

### gRPC methods

All 22 RPC methods are implemented. They follow this pattern:
1. Extract the request payload from `tonic::Request<T>`.
2. Parse and validate fields (UUIDs, timestamps) using the helper functions above.
3. Call the corresponding `StorageBackend` method.
4. Map the result to the protobuf response type.

#### Notes RPCs
`ListNotes`, `CreateNote`, `GetNote`, `UpdateNote`, `DeleteNote`

#### Notebooks RPCs
`ListNotebooks`, `CreateNotebook`, `GetNotebook`, `UpdateNotebook`, `DeleteNotebook`

#### Tags RPCs
`ListTags`, `CreateTag`, `GetTag`, `UpdateTag`, `DeleteTag`,
`AddNoteTag`, `RemoveNoteTag`, `ListNoteTags`

#### Resources RPCs
`ListResources`, `CreateResource`, `GetResource`, `DeleteResource`

#### Sync RPC — server-streaming

`Sync` is a server-streaming RPC that reports progress through a `tokio::sync::mpsc`
channel with a capacity of 16. A `tokio::spawn` task drives the sync cycle and sends
`SyncProgress` messages at each stage:

| Stage | `Stage` enum value | What it means |
|-------|--------------------|---------------|
| `COLLECTING` | 0 | Retrieving the last-sync timestamp and collecting local changes |
| `SENDING` | 1 | Pushing local changes to the remote peer |
| `RECEIVING` | 2 | Pulling changes from the remote peer |
| `APPLYING` | 3 | Applying each remote change to the local store |
| `DONE` | 4 | Sync complete; `changes_count` reports how many remote changes were applied |

If any step fails, an error `Status::internal` is sent and the task exits.

## Data flow (example: `CreateNote`)

```
gRPC client → CreateNoteRequest (proto)
  → parse_optional_dt / parse_uuid
  → CoreNote::new(title, body) + set optional fields
  → backend.create_note(note)     ← may encrypt + persist
  → note_to_proto(stored_note)
  → CreateNoteResponse (proto) → gRPC client
```

## Design notes

- `UpdateNote` explicitly overwrites `note.updated_at = now()` before calling the
  backend. This ensures the timestamp reflects when the gRPC call was received, not the
  value supplied by the client, which prevents clients from supplying arbitrary
  timestamps.
- `parse_uuid` and `parse_optional_dt` return `tonic::Status` errors (not
  `StorageError`) because they validate client input at the RPC boundary; `StorageError`
  is reserved for backend-layer failures.
- `#[allow(clippy::result_large_err)]` is on the helper functions because
  `tonic::Status` is 176 bytes. The functions are called in every RPC handler, so
  suppressing the warning is preferable to boxing the return value.

## Related files

- `keeplin-daemon/src/proto.rs` — includes the generated `KeeplinService` trait
- `keeplin-daemon/proto/keeplin.proto` — the API contract
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` trait `B` must satisfy
- `keeplin-daemon/src/main.rs` — constructs `KeeplinServer` and registers it with tonic
