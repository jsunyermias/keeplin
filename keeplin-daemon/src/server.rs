use std::{pin::Pin, sync::Arc};

use keeplin_core::{
    error::StorageError,
    models::{now, Note as CoreNote, NoteTag, Notebook as CoreNotebook, Resource as CoreResource, Tag as CoreTag},
    storage::StorageBackend,
};
use tokio_stream::{wrappers::ReceiverStream, Stream};
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::proto::keeplin::{
    keeplin_service_server::KeeplinService,
    sync_progress::Stage,
    AddNoteTagRequest, AddNoteTagResponse,
    CreateNoteRequest, CreateNoteResponse,
    CreateNotebookRequest, CreateNotebookResponse,
    CreateResourceRequest, CreateResourceResponse,
    CreateTagRequest, CreateTagResponse,
    DeleteNoteRequest, DeleteNoteResponse,
    DeleteNotebookRequest, DeleteNotebookResponse,
    DeleteResourceRequest, DeleteResourceResponse,
    DeleteTagRequest, DeleteTagResponse,
    GetNoteRequest, GetNoteResponse,
    GetNotebookRequest, GetNotebookResponse,
    GetResourceRequest, GetResourceResponse,
    GetTagRequest, GetTagResponse,
    ListNotebooksRequest, ListNotebooksResponse,
    ListNoteTagsRequest, ListNoteTagsResponse,
    ListNotesRequest, ListNotesResponse,
    ListResourcesRequest, ListResourcesResponse,
    ListTagsRequest, ListTagsResponse,
    Note, Notebook, RemoveNoteTagRequest, RemoveNoteTagResponse,
    Resource, SyncProgress, SyncRequest, Tag,
    UpdateNoteRequest, UpdateNoteResponse,
    UpdateNotebookRequest, UpdateNotebookResponse,
    UpdateTagRequest, UpdateTagResponse,
};

// ── Conversion helpers ────────────────────────────────────────────────────────

fn note_to_proto(n: CoreNote) -> Note {
    Note {
        id: n.id.to_string(),
        title: n.title,
        body: n.body,
        notebook_id: n.notebook_id.map(|u| u.to_string()).unwrap_or_default(),
        is_todo: n.is_todo,
        todo_due: n.todo_due.map(|d| d.to_rfc3339()).unwrap_or_default(),
        todo_completed: n
            .todo_completed
            .map(|d| d.to_rfc3339())
            .unwrap_or_default(),
        created_at: n.created_at.to_rfc3339(),
        updated_at: n.updated_at.to_rfc3339(),
        deleted_at: n.deleted_at.map(|d| d.to_rfc3339()).unwrap_or_default(),
    }
}

fn notebook_to_proto(nb: CoreNotebook) -> Notebook {
    Notebook {
        id: nb.id.to_string(),
        title: nb.title,
        created_at: nb.created_at.to_rfc3339(),
        updated_at: nb.updated_at.to_rfc3339(),
        deleted_at: nb.deleted_at.map(|d| d.to_rfc3339()).unwrap_or_default(),
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
        deleted_at: t.deleted_at.map(|d| d.to_rfc3339()).unwrap_or_default(),
    }
}

fn storage_err(e: StorageError) -> Status {
    match &e {
        StorageError::NotFound(_) => Status::not_found(e.to_string()),
        _ => Status::internal(e.to_string()),
    }
}

#[allow(clippy::result_large_err)]
fn parse_uuid(s: &str, field: &str) -> Result<Uuid, Status> {
    s.parse::<Uuid>()
        .map_err(|_| Status::invalid_argument(format!("{field} is not a valid UUID")))
}

#[allow(clippy::result_large_err)]
fn parse_optional_dt(s: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>, Status> {
    if s.is_empty() {
        return Ok(None);
    }
    s.parse::<chrono::DateTime<chrono::Utc>>()
        .map(Some)
        .map_err(|_| Status::invalid_argument(format!("{s} is not a valid RFC-3339 timestamp")))
}

#[allow(clippy::result_large_err)]
fn proto_to_note(n: Note) -> Result<CoreNote, Status> {
    Ok(CoreNote {
        id: parse_uuid(&n.id, "id")?,
        title: n.title,
        body: n.body,
        notebook_id: if n.notebook_id.is_empty() {
            None
        } else {
            Some(parse_uuid(&n.notebook_id, "notebook_id")?)
        },
        is_todo: n.is_todo,
        todo_due: parse_optional_dt(&n.todo_due)?,
        todo_completed: parse_optional_dt(&n.todo_completed)?,
        created_at: n
            .created_at
            .parse::<chrono::DateTime<chrono::Utc>>()
            .map_err(|_| Status::invalid_argument("created_at is invalid"))?,
        updated_at: n
            .updated_at
            .parse::<chrono::DateTime<chrono::Utc>>()
            .map_err(|_| Status::invalid_argument("updated_at is invalid"))?,
        deleted_at: parse_optional_dt(&n.deleted_at)?,
    })
}

// ── Server ────────────────────────────────────────────────────────────────────

pub struct KeeplinServer<B: StorageBackend> {
    backend: Arc<B>,
}

impl<B: StorageBackend> KeeplinServer<B> {
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
        _req: Request<ListNotesRequest>,
    ) -> Result<Response<ListNotesResponse>, Status> {
        let notes = self
            .backend
            .list_notes()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(ListNotesResponse {
            notes: notes.into_iter().map(note_to_proto).collect(),
        }))
    }

    async fn create_note(
        &self,
        req: Request<CreateNoteRequest>,
    ) -> Result<Response<CreateNoteResponse>, Status> {
        let r = req.into_inner();
        let mut note = CoreNote::new(r.title, r.body);
        note.is_todo = r.is_todo;
        note.todo_due = parse_optional_dt(&r.todo_due)?;
        if !r.notebook_id.is_empty() {
            note.notebook_id = Some(parse_uuid(&r.notebook_id, "notebook_id")?);
        }
        let created = self
            .backend
            .create_note(note)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(CreateNoteResponse {
            note: Some(note_to_proto(created)),
        }))
    }

    async fn get_note(
        &self,
        req: Request<GetNoteRequest>,
    ) -> Result<Response<GetNoteResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        let note = self
            .backend
            .read_note(id)
            .await
            .map_err(|e| match &e {
                keeplin_core::error::StorageError::NotFound(_) => {
                    Status::not_found(e.to_string())
                }
                _ => Status::internal(e.to_string()),
            })?;
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
        let note = proto_to_note(note_proto)?;
        let updated = self
            .backend
            .update_note(note)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(UpdateNoteResponse {
            note: Some(note_to_proto(updated)),
        }))
    }

    async fn delete_note(
        &self,
        req: Request<DeleteNoteRequest>,
    ) -> Result<Response<DeleteNoteResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        self.backend
            .delete_note(id)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(DeleteNoteResponse {}))
    }

    // ── Notebooks ─────────────────────────────────────────────────────────────

    async fn list_notebooks(
        &self,
        _req: Request<ListNotebooksRequest>,
    ) -> Result<Response<ListNotebooksResponse>, Status> {
        let notebooks = self
            .backend
            .list_notebooks()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(ListNotebooksResponse {
            notebooks: notebooks.into_iter().map(notebook_to_proto).collect(),
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
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(CreateNotebookResponse {
            notebook: Some(notebook_to_proto(created)),
        }))
    }

    async fn get_notebook(
        &self,
        req: Request<GetNotebookRequest>,
    ) -> Result<Response<GetNotebookResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        let notebook = self
            .backend
            .read_notebook(id)
            .await
            .map_err(storage_err)?;
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
            deleted_at: parse_optional_dt(&nb.deleted_at)?,
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
        _req: Request<ListTagsRequest>,
    ) -> Result<Response<ListTagsResponse>, Status> {
        let tags = self
            .backend
            .list_tags()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(ListTagsResponse {
            tags: tags.into_iter().map(tag_to_proto).collect(),
        }))
    }

    async fn create_tag(
        &self,
        req: Request<CreateTagRequest>,
    ) -> Result<Response<CreateTagResponse>, Status> {
        let tag = CoreTag::new(req.into_inner().title);
        let created = self
            .backend
            .create_tag(tag)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
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
        let tag = self
            .backend
            .read_tag(id)
            .await
            .map_err(|e| match &e {
                keeplin_core::error::StorageError::NotFound(_) => Status::not_found(e.to_string()),
                _ => Status::internal(e.to_string()),
            })?;
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
            deleted_at: parse_optional_dt(&t.deleted_at)?,
        };
        let updated = self
            .backend
            .update_tag(tag)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(UpdateTagResponse {
            tag: Some(tag_to_proto(updated)),
        }))
    }

    async fn delete_tag(
        &self,
        req: Request<DeleteTagRequest>,
    ) -> Result<Response<DeleteTagResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        self.backend
            .delete_tag(id)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(DeleteTagResponse {}))
    }

    async fn list_note_tags(
        &self,
        req: Request<ListNoteTagsRequest>,
    ) -> Result<Response<ListNoteTagsResponse>, Status> {
        let note_id = parse_uuid(&req.into_inner().note_id, "note_id")?;
        let tags = self
            .backend
            .list_note_tags(note_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(ListNoteTagsResponse {
            tags: tags.into_iter().map(tag_to_proto).collect(),
        }))
    }

    // ── Resources ─────────────────────────────────────────────────────────────

    async fn list_resources(
        &self,
        _req: Request<ListResourcesRequest>,
    ) -> Result<Response<ListResourcesResponse>, Status> {
        let resources = self
            .backend
            .list_resources()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(ListResourcesResponse {
            resources: resources.into_iter().map(resource_to_proto).collect(),
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
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(CreateResourceResponse {
            resource: Some(resource_to_proto(created)),
        }))
    }

    async fn get_resource(
        &self,
        req: Request<GetResourceRequest>,
    ) -> Result<Response<GetResourceResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        let (meta, data) = self
            .backend
            .read_resource(id)
            .await
            .map_err(|e| match &e {
                keeplin_core::error::StorageError::NotFound(_) => Status::not_found(e.to_string()),
                _ => Status::internal(e.to_string()),
            })?;
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

    async fn sync(
        &self,
        _req: Request<SyncRequest>,
    ) -> Result<Response<Self::SyncStream>, Status> {
        let backend = Arc::clone(&self.backend);
        let (tx, rx) = tokio::sync::mpsc::channel::<SyncStreamItem>(16);

        tokio::spawn(async move {
            macro_rules! progress {
                ($stage:expr, $count:expr, $msg:expr) => {
                    let _ = tx
                        .send(Ok(SyncProgress {
                            stage: $stage as i32,
                            changes_count: $count,
                            message: $msg.to_string(),
                        }))
                        .await;
                };
            }
            macro_rules! bail {
                ($e:expr) => {{
                    let _ = tx.send(Err(Status::internal($e.to_string()))).await;
                    return;
                }};
            }

            progress!(Stage::Collecting, 0, "Collecting local changes");
            let last_sync = match backend.get_last_sync_time().await {
                Ok(t) => t,
                Err(e) => bail!(e),
            };

            let local = match backend.get_changes_since(last_sync).await {
                Ok(c) => c,
                Err(e) => bail!(e),
            };

            progress!(Stage::Sending, local.len() as i32, "Sending local changes");
            if let Err(e) = backend.send_changes(local).await {
                bail!(e);
            }

            progress!(Stage::Receiving, 0, "Receiving remote changes");
            let remote = match backend.receive_changes().await {
                Ok(c) => c,
                Err(e) => bail!(e),
            };

            progress!(
                Stage::Applying,
                remote.len() as i32,
                "Applying remote changes"
            );
            for change in &remote {
                if let Err(e) = backend.apply_change(change.clone()).await {
                    bail!(e);
                }
            }

            let _ = backend.update_sync_time(now()).await;
            progress!(Stage::Done, remote.len() as i32, "Sync complete");
        });

        Ok(Response::new(
            Box::pin(ReceiverStream::new(rx)) as SyncStreamPin,
        ))
    }
}
