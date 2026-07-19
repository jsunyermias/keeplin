// md:Overview

use std::{pin::Pin, sync::Arc};

use keeplin_core::{
    error::{StorageError, SyncError},
    linking,
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

// md:fn bookmark_to_proto
fn bookmark_to_proto(b: CoreBookmark) -> ProtoBookmark {
    ProtoBookmark {
        number: b.number,
        text: b.text,
        alias: b.alias,
    }
}

// md:fn link_source_str
fn link_source_str(s: LinkSource) -> String {
    match s {
        LinkSource::Content => "content",
        LinkSource::Manual => "manual",
    }
    .to_string()
}

// md:fn notelink_to_proto
fn notelink_to_proto(l: CoreNoteLink) -> ProtoNoteLink {
    ProtoNoteLink {
        source: link_source_str(l.source),
        raw: l.raw,
        target_note_id: l.target_note_id.map(|u| u.to_string()),
    }
}

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

// md:fn resource_to_proto
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

// md:fn tag_to_proto
fn tag_to_proto(t: CoreTag) -> Tag {
    Tag {
        id: t.id.to_string(),
        title: t.title,
        created_at: t.created_at.to_rfc3339(),
        updated_at: t.updated_at.to_rfc3339(),
        deleted_at: t.deleted_at.map(|d| d.to_rfc3339()),
    }
}

// md:fn storage_err
fn storage_err(e: StorageError) -> Status {
    match &e {
        StorageError::NotFound(_) => Status::not_found(e.to_string()),
        StorageError::CorruptedData(_) => Status::data_loss(e.to_string()),
        StorageError::Conflict(_) => Status::already_exists(e.to_string()),
        StorageError::InvalidInput(_) => Status::invalid_argument(e.to_string()),
        _ => Status::internal(e.to_string()),
    }
}

// md:fn parse_uuid
#[allow(clippy::result_large_err)]
fn parse_uuid(s: &str, field: &str) -> Result<Uuid, Status> {
    s.parse::<Uuid>()
        .map_err(|_| Status::invalid_argument(format!("{field} is not a valid UUID")))
}

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

// md:fn proto_to_bookmark
fn proto_to_bookmark(b: ProtoBookmark) -> CoreBookmark {
    CoreBookmark {
        number: b.number,
        text: b.text,
        alias: b.alias,
    }
}

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

// md:KeeplinServer
pub struct KeeplinServer<B: StorageBackend> {
    backend: Arc<B>,
    journal_retention_days: u64,
    resource_purge_days: u64,
    max_upload_bytes: usize,
}

// md:impl KeeplinServer
impl<B: StorageBackend> KeeplinServer<B> {
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
        let resource = CoreResource::new(meta.title, meta.mime_type, meta.file_name, size);
        let created = self
            .backend
            .create_resource(resource, data)
            .await
            .map_err(storage_err)?;
        Ok(Response::new(UploadResourceResponse {
            resource: Some(resource_to_proto(created)),
        }))
    }
}

// md:SyncStreamItem
type SyncStreamItem = Result<SyncProgress, Status>;
// md:SyncStreamPin
type SyncStreamPin = Pin<Box<dyn Stream<Item = SyncStreamItem> + Send>>;

// md:impl KeeplinService for KeeplinServer
#[tonic::async_trait]
impl<B: StorageBackend> KeeplinService for KeeplinServer<B> {
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
        ordering::place_new_note(self.backend.as_ref(), &mut note)
            .await
            .map_err(storage_err)?;
        let created = self.backend.create_note(note).await.map_err(storage_err)?;
        Ok(Response::new(CreateNoteResponse {
            note: Some(note_to_proto(created)),
        }))
    }

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
        ordering::reconcile_notebook_move(self.backend.as_ref(), stored.notebook_id, &mut note)
            .await
            .map_err(storage_err)?;
        note.updated_at = now();
        let updated = self.backend.update_note(note).await.map_err(storage_err)?;
        Ok(Response::new(UpdateNoteResponse {
            note: Some(note_to_proto(updated)),
        }))
    }

    // md:impl KeeplinService for KeeplinServer > fn delete_note
    async fn delete_note(
        &self,
        req: Request<DeleteNoteRequest>,
    ) -> Result<Response<DeleteNoteResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        self.backend.delete_note(id).await.map_err(storage_err)?;
        Ok(Response::new(DeleteNoteResponse {}))
    }

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
        };
        ensure_not_deleted(self.backend.read_tag(tag.id).await, tag.id, |t| {
            t.deleted_at
        })?;
        let updated = self.backend.update_tag(tag).await.map_err(storage_err)?;
        Ok(Response::new(UpdateTagResponse {
            tag: Some(tag_to_proto(updated)),
        }))
    }

    // md:impl KeeplinService for KeeplinServer > fn delete_tag
    async fn delete_tag(
        &self,
        req: Request<DeleteTagRequest>,
    ) -> Result<Response<DeleteTagResponse>, Status> {
        let id = parse_uuid(&req.into_inner().id, "id")?;
        self.backend.delete_tag(id).await.map_err(storage_err)?;
        Ok(Response::new(DeleteTagResponse {}))
    }

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

    // md:impl KeeplinService for KeeplinServer > fn create_resource
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

    // md:impl KeeplinService for KeeplinServer > fn upload_resource
    async fn upload_resource(
        &self,
        req: Request<tonic::Streaming<UploadResourceRequest>>,
    ) -> Result<Response<UploadResourceResponse>, Status> {
        self.assemble_upload(req.into_inner()).await
    }

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

    // md:impl KeeplinService for KeeplinServer > type SyncStream
    type SyncStream = SyncStreamPin;

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
}

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

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::keeplin::{ResourceMeta, UploadResourceRequest};
    use keeplin_core::storage::fs::FsBackend;
    use keeplin_core::storage::{
        NoteRepository, NotebookRepository, ResourceRepository, TagRepository,
    };

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

    // md:mod tests > fn meta_frame
    fn meta_frame(title: &str, mime: &str, file: &str) -> UploadResourceRequest {
        UploadResourceRequest {
            payload: Some(UploadPayload::Meta(ResourceMeta {
                title: title.into(),
                mime_type: mime.into(),
                file_name: file.into(),
            })),
        }
    }

    // md:mod tests > fn chunk_frame
    fn chunk_frame(bytes: &[u8]) -> UploadResourceRequest {
        UploadResourceRequest {
            payload: Some(UploadPayload::Chunk(bytes.to_vec())),
        }
    }

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
}
