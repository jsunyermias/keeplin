// md:Overview
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
};

use super::SortableRfc3339;

// md:trait NoteRepository
#[async_trait]
pub trait NoteRepository: Send + Sync + 'static {
    async fn create_note(&self, note: Note) -> Result<Note, StorageError>;

    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError>;

    async fn update_note(&self, note: Note) -> Result<Note, StorageError>;

    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError>;

    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError>;

    async fn list_notes_in_notebook(
        &self,
        notebook_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError>;

    async fn list_starred_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError>;

    async fn notebook_sort_profile(
        &self,
        notebook_id: Uuid,
    ) -> Result<NotebookSortProfile, StorageError>;

    async fn note_backlinks(
        &self,
        target_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let mut matches = Vec::new();
        let mut token = None;
        loop {
            let (page, next) = self.list_notes(0, token).await?;
            for note in page {
                if note
                    .links
                    .iter()
                    .any(|l| l.target_note_id == Some(target_id))
                {
                    matches.push(note);
                }
            }
            match next {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        Ok(paginate_notes(matches, page_size, page_token.as_deref()))
    }
}

// md:NotebookSortProfile
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotebookSortProfile {
    pub pinned_keys: Vec<u32>,
    pub min_key: Option<u32>,
    pub max_normal_key: Option<u32>,
}

// md:impl NotebookSortProfile
impl NotebookSortProfile {
    // md:impl NotebookSortProfile > fn from_effective_keys
    pub fn from_effective_keys(keys: impl IntoIterator<Item = u32>) -> Self {
        let mut profile = Self::default();
        for key in keys {
            profile.min_key = Some(profile.min_key.map_or(key, |min| min.min(key)));
            if (1..1000).contains(&key) {
                profile.pinned_keys.push(key);
            } else {
                profile.max_normal_key =
                    Some(profile.max_normal_key.map_or(key, |max| max.max(key)));
            }
        }
        profile.pinned_keys.sort_unstable();
        profile
    }
}

// md:fn paginate_notes
fn paginate_notes(
    items: Vec<Note>,
    page_size: u32,
    token: Option<&str>,
) -> (Vec<Note>, Option<String>) {
    let limit = super::effective_page_size(page_size) as usize;
    let start = match token.filter(|t| !t.is_empty()) {
        Some(cursor) => match cursor.split_once('|') {
            Some((ts, id_str)) => {
                let cursor_id = Uuid::parse_str(id_str).ok();
                items.partition_point(|n| {
                    let item_ts = n.created_at.to_sortable_rfc3339();
                    item_ts.as_str() < ts
                        || (item_ts.as_str() == ts && cursor_id.is_some_and(|c| n.id <= c))
                })
            }
            None => 0,
        },
        None => 0,
    };
    let remaining: Vec<Note> = items.into_iter().skip(start).collect();
    let has_more = remaining.len() > limit;
    let page: Vec<Note> = remaining.into_iter().take(limit).collect();
    let next = if has_more {
        page.last()
            .map(|n| format!("{}|{}", n.created_at.to_sortable_rfc3339(), n.id))
    } else {
        None
    };
    (page, next)
}

// md:trait NotebookRepository
#[async_trait]
pub trait NotebookRepository: Send + Sync + 'static {
    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError>;

    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError>;

    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError>;

    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError>;

    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError>;
}

// md:trait TagRepository
#[async_trait]
pub trait TagRepository: Send + Sync + 'static {
    async fn create_tag(&self, tag: Tag) -> Result<Tag, StorageError>;

    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError>;

    async fn update_tag(&self, tag: Tag) -> Result<Tag, StorageError>;

    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError>;

    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError>;

    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError>;

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError>;

    async fn list_note_tags(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError>;
}

// md:trait ResourceRepository
#[async_trait]
pub trait ResourceRepository: Send + Sync + 'static {
    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError>;

    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError>;

    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError>;

    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError>;

    async fn list_resources_for_note(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let mut matches = Vec::new();
        let mut token = None;
        loop {
            let (page, next) = self.list_resources(0, token).await?;
            for resource in page {
                if resource.note_id == note_id {
                    matches.push(resource);
                }
            }
            match next {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        Ok(paginate_resources(
            matches,
            page_size,
            page_token.as_deref(),
        ))
    }

    async fn purge_deleted_resources(&self, older_than: DateTime<Utc>)
        -> Result<u64, StorageError>;
}

// md:fn paginate_resources
fn paginate_resources(
    items: Vec<Resource>,
    page_size: u32,
    token: Option<&str>,
) -> (Vec<Resource>, Option<String>) {
    let limit = super::effective_page_size(page_size) as usize;
    let start = match token.filter(|t| !t.is_empty()) {
        Some(cursor) => match cursor.split_once('|') {
            Some((ts, id_str)) => {
                let cursor_id = Uuid::parse_str(id_str).ok();
                items.partition_point(|r| {
                    let item_ts = r.created_at.to_sortable_rfc3339();
                    item_ts.as_str() < ts
                        || (item_ts.as_str() == ts && cursor_id.is_some_and(|c| r.id <= c))
                })
            }
            None => 0,
        },
        None => 0,
    };
    let remaining: Vec<Resource> = items.into_iter().skip(start).collect();
    let has_more = remaining.len() > limit;
    let page: Vec<Resource> = remaining.into_iter().take(limit).collect();
    let next = if has_more {
        page.last()
            .map(|r| format!("{}|{}", r.created_at.to_sortable_rfc3339(), r.id))
    } else {
        None
    };
    (page, next)
}

// md:trait SyncBackend
#[async_trait]
pub trait SyncBackend: Send + Sync + 'static {
    async fn get_device_id(&self) -> Result<String, StorageError>;

    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError>;

    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError>;

    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError>;

    async fn apply_change(&self, change: Change) -> Result<(), StorageError>;

    async fn send_changes(&self, changes: Vec<Change>) -> Result<(), StorageError>;

    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError>;

    async fn prune_change_journal(&self, older_than: DateTime<Utc>) -> Result<u64, StorageError>;
}

// md:DEFAULT_HISTORY_LIMIT
pub const DEFAULT_HISTORY_LIMIT: u32 = 100;

// md:EntityVersion
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityVersion<T> {
    pub timestamp: DateTime<Utc>,
    pub device_id: String,
    pub entity: Option<T>,
}

// md:trait HistoryRepository
#[async_trait]
pub trait HistoryRepository: Send + Sync + 'static {
    async fn note_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Note>>, StorageError>;

    async fn notebook_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Notebook>>, StorageError>;
}

// md:trait StorageBackend
pub trait StorageBackend:
    NoteRepository
    + NotebookRepository
    + TagRepository
    + ResourceRepository
    + SyncBackend
    + HistoryRepository
{
}

// md:impl StorageBackend for T
impl<T: ?Sized> StorageBackend for T where
    T: NoteRepository
        + NotebookRepository
        + TagRepository
        + ResourceRepository
        + SyncBackend
        + HistoryRepository
{
}
