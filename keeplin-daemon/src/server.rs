//! gRPC service implementation for the Keeplin daemon.
//!
//! This module defines [`KeeplinServer<B>`], which implements the `KeeplinService`
//! trait generated from `proto/keeplin.proto`. It bridges between the protobuf wire
//! types (e.g. `proto::keeplin::Note`) and the domain types in `keeplin-core`
//! (e.g. `models::Note`), delegating all persistence to a generic [`StorageBackend`].

use std::{pin::Pin, sync::Arc};

use keeplin_core::{
    error::{StorageError, SyncError},
    models::{
        now, Note as CoreNote, NoteTag, Notebook as CoreNotebook, Resource as CoreResource,
        Tag as CoreTag,
    },
    storage::StorageBackend,
    sync::{run_sync, SyncStage},
};
use tokio_stream::{wrappers::UnboundedReceiverStream, Stream};
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::proto::keeplin::{
    keeplin_service_server::KeeplinService, sync_progress::Stage, AddNoteTagRequest,
    AddNoteTagResponse, CreateNoteRequest, CreateNoteResponse, CreateNotebookRequest,
    CreateNotebookResponse, CreateResourceRequest, CreateResourceResponse, CreateTagRequest,
    CreateTagResponse, DeleteNoteRequest, DeleteNoteResponse, DeleteNotebookRequest,
    DeleteNotebookResponse, DeleteResourceRequest, DeleteResourceResponse, DeleteTagRequest,
    DeleteTagResponse, GetNoteRequest, GetNoteResponse, GetNotebookRequest, GetNotebookResponse,
    GetResourceRequest, GetResourceResponse, GetTagRequest, GetTagResponse, ListNoteTagsRequest,
    ListNoteTagsResponse, ListNotebooksRequest, ListNotebooksResponse, ListNotesRequest,
    ListNotesResponse, ListResourcesRequest, ListResourcesResponse, ListTagsRequest,
    ListTagsResponse, Note, Notebook, RemoveNoteTagRequest, RemoveNoteTagResponse, Resource,
    SyncProgress, SyncRequest, Tag, UpdateNoteRequest, UpdateNoteResponse, UpdateNotebookRequest,
    UpdateNotebookResponse, UpdateTagRequest, UpdateTagResponse,
};

// ── Conversion helpers ────────────────────────────────────────────────────────
// These functions are stateless and infallible (they only map known fields). They
// are kept as free functions rather than `From` impls because the proto and domain
// types live in separate crates and the orphan rule would prevent implementing
// `From<CoreNote> for proto::Note` here.

fn note_to_proto(n: CoreNote) -> Note {
    Note {
        id: n.id.to_string(),
        title: n.title,
        body: n.body,
        notebook_id: n.notebook_id.map(|u| u.to_string()),
        is_todo: n.is_todo,
        todo_due: n.todo_due.map(|d| d.to_rfc3339()),
        todo_completed: n.todo_completed.map(|d| d.to_rfc3339()),
        created_at: n.created_at.to_rfc3339(),
        updated_at: n.updated_at.to_rfc3339(),
        deleted_at: n.deleted_at.map(|d| d.to_rfc3339()),
    }
}

fn notebook_to_proto(nb: CoreNotebook) -> Notebook {
    Notebook {
        id: nb.id.to_string(),
        title: nb.title,
        created_at: nb.created_at.to_rfc3339(),
        updated_at: nb.updated_at.to_rfc3339(),
        deleted_at: nb.deleted_at.map(|d| d.to_rfc3339()),
    }
}

fn resource_to_proto(r: CoreResource) -> Resource {
    Resource {
        id: r.id.to_string(),
        title: r.title,
        mime_type: r.mime_type,
        file_name: r.file_name,
        size: r.size as i64,
        created_at: r.created_at.to_rfc3339(),
    }
}

fn tag_to_proto(t: CoreTag) -> Tag {
    Tag {
        id: t.id.to_string(),
        title: t.title,
        created_at: t.created_at.to_rfc3339(),
        updated_at: t.updated_at.to_rfc3339(),
        deleted_at: t.deleted_at.map(|d| d.to_rfc3339()),
    }
}

/// Maps a `StorageError` to the appropriate gRPC `Status` code.
///
/// `NotFound` errors map to `status::not_found` (HTTP 404 equivalent) so that clients
/// can distinguish "entity does not exist" from general server failures. All other
/// errors map to `status::internal` (HTTP 500 equivalent).
fn storage_err(e: StorageError) -> Status {
    match &e {
        StorageError::NotFound(_) => Status::not_found(e.to_string()),
        // AES-GCM authentication tag failure caused by a wrong key or tampered ciphertext.
        // gRPC DATA_LOSS is the closest code: the data exists but cannot be recovered
        // in a trustworthy form.
        StorageError::CorruptedData(_) => Status::data_loss(e.to_string()),
        _ => Status::internal(e.to_string()),
    }
}

/// Parses a UUID string received in a protobuf field.
///
/// Returns `Status::invalid_argument` if the string is not a valid UUID, including the
/// field name in the error message so the client knows which field failed.
#[allow(clippy::result_large_err)]
fn parse_uuid(s: &str, field: &str) -> Result<Uuid, Status> {
    s.parse::<Uuid>()
        .map_err(|_| Status::invalid_argument(format!("{field} is not a valid UUID")))
}

/// Parses an optional RFC-3339 timestamp from a proto3 `optional string` field.
///
/// Returns `None` when the option is absent. Returns `Status::invalid_argument`
/// if the string is present but not a valid RFC-3339 timestamp.
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

#[allow(clippy::result_large_err)]
fn proto_to_note(n: Note) -> Result<CoreNote, Status> {
    Ok(CoreNote {
        id: parse_uuid(&n.id, "id")?,
        title: n.title,
        body: n.body,
        notebook_id: n
            .notebook_id
            .map(|s| parse_uuid(&s, "notebook_id"))
            .transpose()?,
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
    })
}

// ── Server ────────────────────────────────────────────────────────────────────

/// The gRPC service handler.
///
/// `KeeplinServer<B>` is generic over the storage backend so the compiler can
/// monomorphise a single copy for the backend type chosen at startup (e.g.
/// `EncryptedBackend<FsBackend>` or `DbBackend`). The backend is wrapped in `Arc`
/// so it can be shared across the concurrent async tasks that tonic spawns for each
/// incoming RPC call.
pub struct KeeplinServer<B: StorageBackend> {
    /// Reference-counted handle to the backend shared across all handler tasks.
    backend: Arc<B>,
}

impl<B: StorageBackend> KeeplinServer<B> {
    /// Wraps `backend` in an `Arc` and returns a new server.
    ///
    /// The resulting server should be passed to
    /// `KeeplinServiceServer::new(server)` before being registered with tonic.
    pub fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
        }
    }
}

type SyncStreamItem = Result<SyncProgress, Status>;
type SyncStreamPin = Pin<Box<dyn Stream<Item = SyncStreamItem> + Send>>;

#[tonic::async_trait]
impl<B: StorageBackend> KeeplinService for KeeplinServer<B> {
    // ── Notes ─────────────────────────────────────────────────────────────────

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
            note.notebook_id = Some(parse_uuid(&r.notebook_id, "notebook_id")?);
        }
        let created = self.backend.create_note(note).await.map_err(storage_err)?;
        Ok(Response::new(CreateNoteResponse {
            note: Some(note_to_proto(created)),
        }))
    }

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

    async fn update_note(
        &self,
        req: Request<UpdateNoteRequest>,
    ) -> Result<Response<UpdateNoteResponse>, Status> {
        let note_proto = req
            .into_inner()
            .note
            .ok_or_else(|| Status::invalid_argument("note is required"))?;
        let mut note = proto_to_note(note_proto)?;
        note.updated_at = now();
        let updated = self.backend.update_note(note).await.map_err(storage_err)?;
        Ok(Response::new(UpdateNoteResponse {
            note: Some(note_to_proto(updated)),
        }))
    }

    async fn delete_note(
        &self,
        req: Request<DeleteNoteRequest>,
    ) -> Result<Response<DeleteNoteResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        self.backend.delete_note(id).await.map_err(storage_err)?;
        Ok(Response::new(DeleteNoteResponse {}))
    }

    // ── Notebooks ─────────────────────────────────────────────────────────────

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
            updated_at: nb
                .updated_at
                .parse()
                .map_err(|_| Status::invalid_argument("updated_at is invalid"))?,
            deleted_at: parse_optional_dt(nb.deleted_at)?,
        };
        let updated = self
            .backend
            .update_notebook(notebook)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(UpdateNotebookResponse {
            notebook: Some(notebook_to_proto(updated)),
        }))
    }

    async fn delete_notebook(
        &self,
        req: Request<DeleteNotebookRequest>,
    ) -> Result<Response<DeleteNotebookResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        self.backend
            .delete_notebook(id)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(DeleteNotebookResponse {}))
    }

    // ── Tags ──────────────────────────────────────────────────────────────────

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
            updated_at: t
                .updated_at
                .parse()
                .map_err(|_| Status::invalid_argument("updated_at is invalid"))?,
            deleted_at: parse_optional_dt(t.deleted_at)?,
        };
        let updated = self.backend.update_tag(tag).await.map_err(storage_err)?;
        Ok(Response::new(UpdateTagResponse {
            tag: Some(tag_to_proto(updated)),
        }))
    }

    async fn delete_tag(
        &self,
        req: Request<DeleteTagRequest>,
    ) -> Result<Response<DeleteTagResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        self.backend.delete_tag(id).await.map_err(storage_err)?;
        Ok(Response::new(DeleteTagResponse {}))
    }

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

    // ── Resources ─────────────────────────────────────────────────────────────

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
        let (resources, next_page_token) = self
            .backend
            .list_resources(r.page_size, token)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(ListResourcesResponse {
            resources: resources.into_iter().map(resource_to_proto).collect(),
            next_page_token: next_page_token.unwrap_or_default(),
        }))
    }

    async fn create_resource(
        &self,
        req: Request<CreateResourceRequest>,
    ) -> Result<Response<CreateResourceResponse>, Status> {
        let r = req.into_inner();
        let size = r.data.len() as u64;
        let resource = CoreResource::new(r.title, r.mime_type, r.file_name, size);
        let created = self
            .backend
            .create_resource(resource, r.data)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(CreateResourceResponse {
            resource: Some(resource_to_proto(created)),
        }))
    }

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

    // ── Sync (server-streaming) ───────────────────────────────────────────────

    type SyncStream = SyncStreamPin;

    async fn sync(&self, _req: Request<SyncRequest>) -> Result<Response<Self::SyncStream>, Status> {
        let backend = Arc::clone(&self.backend);
        // An unbounded channel lets the synchronous progress callback in `run_sync` emit
        // updates without awaiting; a sync cycle produces only a handful of messages.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<SyncStreamItem>();

        tokio::spawn(async move {
            // Forward each core `SyncStage` to the client as a `SyncProgress` message.
            let progress_tx = tx.clone();
            let report = move |stage: SyncStage, count: usize| {
                let (proto_stage, message) = stage_to_proto(stage);
                let _ = progress_tx.send(Ok(SyncProgress {
                    stage: proto_stage as i32,
                    changes_count: count as i32,
                    message: message.to_string(),
                }));
            };

            // The whole cycle (including the watermark fix) lives in `run_sync`; the
            // daemon only adapts progress and error reporting to the gRPC stream.
            if let Err(e) = run_sync(&*backend, report).await {
                let status = match e {
                    SyncError::Storage(se) => storage_err(se),
                    other => Status::internal(other.to_string()),
                };
                let _ = tx.send(Err(status));
            }
        });

        Ok(Response::new(
            Box::pin(UnboundedReceiverStream::new(rx)) as SyncStreamPin
        ))
    }
}

/// Maps a core [`SyncStage`] to its protobuf [`Stage`] code and a human-readable
/// progress message for the streaming `Sync` RPC.
fn stage_to_proto(stage: SyncStage) -> (Stage, &'static str) {
    match stage {
        SyncStage::Collecting => (Stage::Collecting, "Collecting local changes"),
        SyncStage::Sending => (Stage::Sending, "Sending local changes"),
        SyncStage::Receiving => (Stage::Receiving, "Receiving remote changes"),
        SyncStage::Applying => (Stage::Applying, "Applying remote changes"),
        SyncStage::Done => (Stage::Done, "Sync complete"),
    }
}
