// md:Overview
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::{
    error::StorageError,
    links::{self, Bookmark, LinkSource, NoteLink},
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
    ordering::is_inbox,
    storage::{
        NoteRepository, NotebookRepository, ResourceRepository, StorageBackend, SyncBackend,
        TagRepository,
    },
};

// md:SCAN_PAGE
const SCAN_PAGE: u32 = 500;

// md:ResolvedReference
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedReference {
    pub note_id: Uuid,
    pub bookmark_number: Option<u32>,
}

// md:AliasConflict
#[derive(Debug, Clone, Serialize)]
pub struct AliasConflict<T> {
    pub alias: String,
    pub entities: Vec<T>,
}

// md:AliasConflicts
#[derive(Debug, Clone, Serialize)]
pub struct AliasConflicts {
    pub notes: Vec<AliasConflict<Note>>,
    pub notebooks: Vec<AliasConflict<Notebook>>,
}

// md:AliasIndex
#[derive(Debug, Default)]
struct AliasIndex {
    note_aliases: BTreeMap<String, BTreeSet<(Uuid, Uuid)>>,
    aliased_notes: HashMap<Uuid, (String, Uuid)>,
    notebook_aliases: BTreeMap<String, BTreeSet<Uuid>>,
    aliased_notebooks: HashMap<Uuid, String>,
}

// md:impl AliasIndex
impl AliasIndex {
    // md:impl AliasIndex > fn from_snapshots
    fn from_snapshots(notes: &[Note], notebooks: &[Notebook]) -> Self {
        let mut idx = Self::default();
        for note in notes {
            idx.upsert_note(note);
        }
        for nb in notebooks {
            idx.upsert_notebook(nb);
        }
        idx
    }

    // md:impl AliasIndex > fn upsert_note
    fn upsert_note(&mut self, note: &Note) {
        self.remove_note(note.id);
        if note.deleted_at.is_some() {
            return;
        }
        if is_inbox(note.notebook_id) {
            return;
        }
        if let Some(alias) = &note.alias {
            self.note_aliases
                .entry(alias.clone())
                .or_default()
                .insert((note.id, note.notebook_id));
            self.aliased_notes
                .insert(note.id, (alias.clone(), note.notebook_id));
        }
    }

    // md:impl AliasIndex > fn remove_note
    fn remove_note(&mut self, id: Uuid) {
        if let Some((alias, notebook_id)) = self.aliased_notes.remove(&id) {
            if let Some(set) = self.note_aliases.get_mut(&alias) {
                set.remove(&(id, notebook_id));
                if set.is_empty() {
                    self.note_aliases.remove(&alias);
                }
            }
        }
    }

    // md:impl AliasIndex > fn upsert_notebook
    fn upsert_notebook(&mut self, notebook: &Notebook) {
        self.remove_notebook(notebook.id);
        if notebook.deleted_at.is_some() {
            return;
        }
        if let Some(alias) = &notebook.alias {
            self.notebook_aliases
                .entry(alias.clone())
                .or_default()
                .insert(notebook.id);
            self.aliased_notebooks.insert(notebook.id, alias.clone());
        }
    }

    // md:impl AliasIndex > fn remove_notebook
    fn remove_notebook(&mut self, id: Uuid) {
        if let Some(alias) = self.aliased_notebooks.remove(&id) {
            if let Some(set) = self.notebook_aliases.get_mut(&alias) {
                set.remove(&id);
                if set.is_empty() {
                    self.notebook_aliases.remove(&alias);
                }
            }
        }
    }

    // md:impl AliasIndex > fn note_alias_taken
    fn note_alias_taken(&self, alias: &str, self_id: Uuid, notebook_id: Uuid) -> bool {
        self.note_aliases.get(alias).is_some_and(|set| {
            set.iter()
                .any(|(id, nb)| *id != self_id && *nb == notebook_id)
        })
    }

    // md:impl AliasIndex > fn notebook_alias_taken
    fn notebook_alias_taken(&self, alias: &str, self_id: Uuid) -> bool {
        self.notebook_aliases
            .get(alias)
            .is_some_and(|set| set.iter().any(|id| *id != self_id))
    }

    // md:impl AliasIndex > fn resolve_notebook_seg
    fn resolve_notebook_seg(&self, seg: &str) -> Option<Uuid> {
        if let Ok(id) = Uuid::parse_str(seg) {
            return Some(id);
        }
        self.notebook_aliases
            .get(seg)
            .and_then(|set| set.iter().next().copied())
    }

    // md:impl AliasIndex > fn resolve_note_seg
    fn resolve_note_seg(
        &self,
        seg: &str,
        notebook_seg: Option<&str>,
        source_notebook: Option<Uuid>,
    ) -> Option<Uuid> {
        if let Ok(id) = Uuid::parse_str(seg) {
            return Some(id);
        }
        let candidates: Vec<(Uuid, Uuid)> = self
            .note_aliases
            .get(seg)?
            .iter()
            .copied()
            .filter(|(_, nb)| !is_inbox(*nb))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        if let Some(ns) = notebook_seg {
            let nb = self.resolve_notebook_seg(ns)?;
            if is_inbox(nb) {
                return None;
            }
            return candidates
                .iter()
                .filter(|(_, n)| *n == nb)
                .map(|(id, _)| *id)
                .min();
        }
        if candidates.len() == 1 {
            return Some(candidates[0].0);
        }
        if let Some(src) = source_notebook {
            return candidates
                .iter()
                .filter(|(_, n)| *n == src)
                .map(|(id, _)| *id)
                .min();
        }
        tracing::warn!(alias = %seg, "ambiguous note alias; resolving to smallest uuid");
        candidates.into_iter().map(|(id, _)| id).min()
    }

    // md:impl AliasIndex > fn resolve_target
    fn resolve_target<'a>(
        &self,
        raw: &'a str,
        source_notebook: Option<Uuid>,
    ) -> Option<(Uuid, Option<&'a str>)> {
        let body = raw.strip_prefix('#')?;
        let segments: Vec<&str> = body.split('#').collect();
        if segments.iter().any(|s| s.is_empty()) {
            return None;
        }
        match segments.as_slice() {
            [note] => Some((self.resolve_note_seg(note, None, source_notebook)?, None)),
            [first, second] => {
                if let Some(id) = self.resolve_note_seg(second, Some(first), source_notebook) {
                    Some((id, None))
                } else {
                    Some((
                        self.resolve_note_seg(first, None, source_notebook)?,
                        Some(second),
                    ))
                }
            }
            [notebook, note, bookmark] => Some((
                self.resolve_note_seg(note, Some(notebook), source_notebook)?,
                Some(bookmark),
            )),
            _ => None,
        }
    }
}

// md:fn change_affects_aliases
fn change_affects_aliases(change: &Change) -> bool {
    matches!(
        change,
        Change::NoteCreate { .. }
            | Change::NoteUpdate { .. }
            | Change::NoteDelete { .. }
            | Change::NotebookCreate { .. }
            | Change::NotebookUpdate { .. }
            | Change::NotebookDelete { .. }
    )
}

// md:LinkingBackend
pub struct LinkingBackend<B> {
    inner: B,
    alias_write_lock: Arc<Mutex<()>>,
    alias_index: Arc<RwLock<Option<AliasIndex>>>,
}

// md:impl LinkingBackend
impl<B: StorageBackend> LinkingBackend<B> {
    // md:impl LinkingBackend > fn new
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            alias_write_lock: Arc::new(Mutex::new(())),
            alias_index: Arc::new(RwLock::new(None)),
        }
    }

    // md:impl LinkingBackend > fn with_index
    async fn with_index<R>(&self, f: impl FnOnce(&AliasIndex) -> R) -> Result<R, StorageError> {
        {
            let guard = self.alias_index.read().await;
            if let Some(idx) = guard.as_ref() {
                return Ok(f(idx));
            }
        }
        let mut guard = self.alias_index.write().await;
        if guard.is_none() {
            let notes = collect_notes(&self.inner).await?;
            let notebooks = collect_notebooks(&self.inner).await?;
            *guard = Some(AliasIndex::from_snapshots(&notes, &notebooks));
        }
        Ok(f(guard.as_ref().expect("index was just built")))
    }

    // md:impl LinkingBackend > fn index_upsert_note
    async fn index_upsert_note(&self, note: &Note) {
        if let Some(idx) = self.alias_index.write().await.as_mut() {
            idx.upsert_note(note);
        }
    }

    // md:impl LinkingBackend > fn index_remove_note
    async fn index_remove_note(&self, id: Uuid) {
        if let Some(idx) = self.alias_index.write().await.as_mut() {
            idx.remove_note(id);
        }
    }

    // md:impl LinkingBackend > fn index_upsert_notebook
    async fn index_upsert_notebook(&self, notebook: &Notebook) {
        if let Some(idx) = self.alias_index.write().await.as_mut() {
            idx.upsert_notebook(notebook);
        }
    }

    // md:impl LinkingBackend > fn index_remove_notebook
    async fn index_remove_notebook(&self, id: Uuid) {
        if let Some(idx) = self.alias_index.write().await.as_mut() {
            idx.remove_notebook(id);
        }
    }

    // md:impl LinkingBackend > fn index_invalidate
    async fn index_invalidate(&self) {
        *self.alias_index.write().await = None;
    }

    // md:impl LinkingBackend > fn refresh
    fn refresh(note: &mut Note) {
        note.bookmarks = links::parse_bookmarks(&note.body)
            .into_iter()
            .enumerate()
            .map(|(i, b)| {
                let alias = b
                    .alias
                    .filter(|a| !a.is_empty())
                    .unwrap_or_else(|| b.text.clone());
                Bookmark {
                    number: (i + 1) as u32,
                    text: b.text,
                    alias,
                }
            })
            .collect();

        let mut links: Vec<NoteLink> = note
            .links
            .iter()
            .filter(|l| l.source == LinkSource::Manual)
            .cloned()
            .collect();
        for raw in links::parse_content_links(&note.body) {
            if let Some(link) = NoteLink::from_raw(&raw, LinkSource::Content) {
                links.push(link);
            }
        }
        note.links = links;
    }

    // md:impl LinkingBackend > fn prepare
    async fn prepare(&self, note: &mut Note) -> Result<(), StorageError> {
        Self::refresh(note);
        if is_inbox(note.notebook_id) {
            note.alias = None;
            for link in note.links.iter_mut() {
                link.target_note_id = None;
            }
            return Ok(());
        }
        if note.alias.is_none() && note.links.is_empty() {
            return Ok(());
        }
        let notebook_id = note.notebook_id;
        let (alias_taken, targets, unverified) = self
            .with_index(|idx| {
                let taken = note
                    .alias
                    .as_deref()
                    .is_some_and(|alias| idx.note_alias_taken(alias, note.id, notebook_id));
                let targets: Vec<Option<Uuid>> = note
                    .links
                    .iter()
                    .map(|link| {
                        idx.resolve_target(&link.raw, Some(notebook_id))
                            .map(|(id, _)| id)
                    })
                    .collect();
                let unverified: Vec<Uuid> = targets
                    .iter()
                    .flatten()
                    .filter(|id| !idx.aliased_notes.contains_key(id))
                    .copied()
                    .collect();
                (taken, targets, unverified)
            })
            .await?;
        if alias_taken {
            let alias = note.alias.as_deref().unwrap_or_default();
            return Err(StorageError::Conflict(format!(
                "note alias '{alias}' is already in use"
            )));
        }
        let mut inbox_targets: HashSet<Uuid> = HashSet::new();
        for id in &unverified {
            if inbox_targets.contains(id) {
                continue;
            }
            if let Ok(target) = self.inner.read_note(*id).await {
                if target.deleted_at.is_none() && is_inbox(target.notebook_id) {
                    inbox_targets.insert(*id);
                }
            }
        }
        for (link, target) in note.links.iter_mut().zip(targets) {
            link.target_note_id = target.filter(|t| !inbox_targets.contains(t));
        }
        Ok(())
    }

    // md:impl LinkingBackend > fn ensure_notebook_alias_free
    async fn ensure_notebook_alias_free(&self, notebook: &Notebook) -> Result<(), StorageError> {
        let Some(alias) = notebook.alias.clone() else {
            return Ok(());
        };
        let taken = self
            .with_index(|idx| idx.notebook_alias_taken(&alias, notebook.id))
            .await?;
        if taken {
            return Err(StorageError::Conflict(format!(
                "notebook alias '{alias}' is already in use"
            )));
        }
        Ok(())
    }
}

// md:fn collect_notes
pub async fn collect_notes(backend: &dyn StorageBackend) -> Result<Vec<Note>, StorageError> {
    let mut out = Vec::new();
    let mut token = None;
    loop {
        let (page, next) = backend.list_notes(SCAN_PAGE, token).await?;
        out.extend(page);
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    Ok(out)
}

// md:fn collect_notebooks
pub async fn collect_notebooks(
    backend: &dyn StorageBackend,
) -> Result<Vec<Notebook>, StorageError> {
    let mut out = Vec::new();
    let mut token = None;
    loop {
        let (page, next) = backend.list_notebooks(SCAN_PAGE, token).await?;
        out.extend(page);
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    Ok(out)
}

// md:fn resolve_bookmark_seg
fn resolve_bookmark_seg(seg: &str, note_id: Uuid, notes: &[Note]) -> Option<u32> {
    let note = notes.iter().find(|n| n.id == note_id)?;
    match links::BookmarkRef::parse(seg) {
        links::BookmarkRef::Number(n) => note
            .bookmarks
            .iter()
            .find(|b| b.number == n)
            .map(|b| b.number),
        links::BookmarkRef::Alias(a) => note
            .bookmarks
            .iter()
            .find(|b| b.alias == a)
            .map(|b| b.number),
    }
}

// md:fn resolve_ref
fn resolve_ref(raw: &str, notes: &[Note], notebooks: &[Notebook]) -> Option<ResolvedReference> {
    let idx = AliasIndex::from_snapshots(notes, notebooks);
    let (note_id, bookmark_seg) = idx.resolve_target(raw, None)?;
    if notes
        .iter()
        .any(|n| n.id == note_id && is_inbox(n.notebook_id))
    {
        return None;
    }
    Some(ResolvedReference {
        note_id,
        bookmark_number: bookmark_seg.and_then(|seg| resolve_bookmark_seg(seg, note_id, notes)),
    })
}

// md:fn resolve
pub async fn resolve(
    backend: &dyn StorageBackend,
    raw: &str,
) -> Result<Option<ResolvedReference>, StorageError> {
    let notes = collect_notes(backend).await?;
    let notebooks = collect_notebooks(backend).await?;
    Ok(resolve_ref(raw, &notes, &notebooks))
}

// md:fn backlinks
pub async fn backlinks(
    backend: &dyn StorageBackend,
    target_id: Uuid,
    page_size: u32,
    page_token: Option<String>,
) -> Result<(Vec<Note>, Option<String>), StorageError> {
    backend
        .note_backlinks(target_id, page_size, page_token)
        .await
}

// md:fn group_conflicts
fn group_conflicts<T>(
    items: Vec<T>,
    alias_of: impl Fn(&T) -> Option<String>,
    id_of: impl Fn(&T) -> Uuid,
) -> Vec<AliasConflict<T>> {
    let mut by_alias: std::collections::BTreeMap<String, Vec<T>> =
        std::collections::BTreeMap::new();
    for item in items {
        if let Some(alias) = alias_of(&item) {
            by_alias.entry(alias).or_default().push(item);
        }
    }
    by_alias
        .into_iter()
        .filter(|(_, group)| group.len() >= 2)
        .map(|(alias, mut entities)| {
            entities.sort_by_key(&id_of);
            AliasConflict { alias, entities }
        })
        .collect()
}

// md:fn alias_conflicts
pub async fn alias_conflicts(backend: &dyn StorageBackend) -> Result<AliasConflicts, StorageError> {
    let notes = collect_notes(backend).await?;
    let notebooks = collect_notebooks(backend).await?;
    Ok(AliasConflicts {
        notes: group_note_conflicts(notes),
        notebooks: group_conflicts(notebooks, |nb| nb.alias.clone(), |nb| nb.id),
    })
}

// md:fn group_note_conflicts
fn group_note_conflicts(notes: Vec<Note>) -> Vec<AliasConflict<Note>> {
    let mut by_key: BTreeMap<(String, Uuid), Vec<Note>> = BTreeMap::new();
    for note in notes {
        if is_inbox(note.notebook_id) {
            continue;
        }
        if let Some(alias) = note.alias.clone() {
            by_key
                .entry((alias, note.notebook_id))
                .or_default()
                .push(note);
        }
    }
    by_key
        .into_iter()
        .filter(|(_, group)| group.len() >= 2)
        .map(|((alias, _notebook), mut entities)| {
            entities.sort_by_key(|n| n.id);
            AliasConflict { alias, entities }
        })
        .collect()
}

// md:fn read_live_note
async fn read_live_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    let note = backend.read_note(id).await?;
    if note.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()));
    }
    Ok(note)
}

// md:fn read_live_notebook
async fn read_live_notebook(
    backend: &dyn StorageBackend,
    id: Uuid,
) -> Result<Notebook, StorageError> {
    let notebook = backend.read_notebook(id).await?;
    if notebook.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()));
    }
    Ok(notebook)
}

// md:fn set_note_alias
pub async fn set_note_alias(
    backend: &dyn StorageBackend,
    note_id: Uuid,
    alias: Option<String>,
) -> Result<Note, StorageError> {
    let mut note = read_live_note(backend, note_id).await?;
    if alias.is_some() && is_inbox(note.notebook_id) {
        return Err(StorageError::InvalidInput(
            "an Inbox note cannot carry an alias".into(),
        ));
    }
    note.alias = alias;
    backend.update_note(note).await
}

// md:fn set_notebook_alias
pub async fn set_notebook_alias(
    backend: &dyn StorageBackend,
    notebook_id: Uuid,
    alias: Option<String>,
) -> Result<Notebook, StorageError> {
    let mut notebook = read_live_notebook(backend, notebook_id).await?;
    notebook.alias = alias;
    backend.update_notebook(notebook).await
}

// md:fn add_manual_link
pub async fn add_manual_link(
    backend: &dyn StorageBackend,
    note_id: Uuid,
    raw: &str,
) -> Result<Note, StorageError> {
    let link = NoteLink::from_raw(raw, LinkSource::Manual)
        .ok_or_else(|| StorageError::InvalidState(format!("invalid link reference '{raw}'")))?;
    let mut note = read_live_note(backend, note_id).await?;
    note.links.push(link);
    backend.update_note(note).await
}

// md:fn remove_link
pub async fn remove_link(
    backend: &dyn StorageBackend,
    note_id: Uuid,
    index: usize,
) -> Result<Note, StorageError> {
    let mut note = read_live_note(backend, note_id).await?;
    if index >= note.links.len() {
        return Err(StorageError::NotFound(format!(
            "link {index} in note {note_id}"
        )));
    }
    note.links.remove(index);
    backend.update_note(note).await
}

// md:impl NoteRepository for LinkingBackend
#[async_trait]
impl<B: StorageBackend> NoteRepository for LinkingBackend<B> {
    async fn create_note(&self, mut note: Note) -> Result<Note, StorageError> {
        let _guard = if note.alias.is_some() {
            Some(self.alias_write_lock.lock().await)
        } else {
            None
        };
        self.prepare(&mut note).await?;
        let stored = self.inner.create_note(note).await?;
        self.index_upsert_note(&stored).await;
        Ok(stored)
    }

    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        self.inner.read_note(id).await
    }

    async fn update_note(&self, mut note: Note) -> Result<Note, StorageError> {
        let _guard = if note.alias.is_some() {
            Some(self.alias_write_lock.lock().await)
        } else {
            None
        };
        self.prepare(&mut note).await?;
        let stored = self.inner.update_note(note).await?;
        self.index_upsert_note(&stored).await;
        Ok(stored)
    }

    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_note(id).await?;
        self.index_remove_note(id).await;
        Ok(())
    }

    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        self.inner.list_notes(page_size, page_token).await
    }

    async fn note_backlinks(
        &self,
        target_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        if let Ok(note) = self.inner.read_note(target_id).await {
            if is_inbox(note.notebook_id) {
                return Ok((Vec::new(), None));
            }
        }
        self.inner
            .note_backlinks(target_id, page_size, page_token)
            .await
    }

    async fn list_notes_in_notebook(
        &self,
        notebook_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        self.inner
            .list_notes_in_notebook(notebook_id, page_size, page_token)
            .await
    }

    async fn list_starred_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        self.inner.list_starred_notes(page_size, page_token).await
    }

    async fn notebook_sort_profile(
        &self,
        notebook_id: Uuid,
    ) -> Result<crate::storage::NotebookSortProfile, StorageError> {
        self.inner.notebook_sort_profile(notebook_id).await
    }
}

// md:impl NotebookRepository for LinkingBackend
#[async_trait]
impl<B: StorageBackend> NotebookRepository for LinkingBackend<B> {
    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        let _guard = if notebook.alias.is_some() {
            Some(self.alias_write_lock.lock().await)
        } else {
            None
        };
        self.ensure_notebook_alias_free(&notebook).await?;
        let stored = self.inner.create_notebook(notebook).await?;
        self.index_upsert_notebook(&stored).await;
        Ok(stored)
    }

    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        self.inner.read_notebook(id).await
    }

    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        let _guard = if notebook.alias.is_some() {
            Some(self.alias_write_lock.lock().await)
        } else {
            None
        };
        self.ensure_notebook_alias_free(&notebook).await?;
        let stored = self.inner.update_notebook(notebook).await?;
        self.index_upsert_notebook(&stored).await;
        Ok(stored)
    }

    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_notebook(id).await?;
        self.index_remove_notebook(id).await;
        Ok(())
    }

    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        self.inner.list_notebooks(page_size, page_token).await
    }
}

// md:impl TagRepository for LinkingBackend
#[async_trait]
impl<B: StorageBackend> TagRepository for LinkingBackend<B> {
    async fn create_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        self.inner.create_tag(tag).await
    }

    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        self.inner.read_tag(id).await
    }

    async fn update_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        self.inner.update_tag(tag).await
    }

    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_tag(id).await
    }

    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        self.inner.list_tags(page_size, page_token).await
    }

    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        self.inner.add_note_tag(note_tag).await
    }

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        self.inner.remove_note_tag(note_id, tag_id).await
    }

    async fn list_note_tags(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        self.inner
            .list_note_tags(note_id, page_size, page_token)
            .await
    }
}

// md:impl ResourceRepository for LinkingBackend
#[async_trait]
impl<B: StorageBackend> ResourceRepository for LinkingBackend<B> {
    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        self.inner.create_resource(resource, data).await
    }

    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        self.inner.read_resource(id).await
    }

    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_resource(id).await
    }

    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        self.inner.list_resources(page_size, page_token).await
    }

    async fn purge_deleted_resources(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, StorageError> {
        self.inner.purge_deleted_resources(older_than).await
    }
}

// md:impl SyncBackend for LinkingBackend
#[async_trait]
impl<B: StorageBackend> SyncBackend for LinkingBackend<B> {
    async fn get_device_id(&self) -> Result<String, StorageError> {
        self.inner.get_device_id().await
    }

    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
        self.inner.get_last_sync_time().await
    }

    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        self.inner.update_sync_time(ts).await
    }

    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError> {
        self.inner.get_changes_since(since).await
    }

    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        let invalidates = change_affects_aliases(&change);
        self.inner.apply_change(change).await?;
        if invalidates {
            self.index_invalidate().await;
        }
        Ok(())
    }

    async fn send_changes(&self, changes: Vec<Change>) -> Result<(), StorageError> {
        self.inner.send_changes(changes).await
    }

    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        let changes = self.inner.receive_changes().await?;
        if changes.iter().any(change_affects_aliases) {
            self.index_invalidate().await;
        }
        Ok(changes)
    }

    async fn prune_change_journal(&self, older_than: DateTime<Utc>) -> Result<u64, StorageError> {
        self.inner.prune_change_journal(older_than).await
    }
}

// md:impl HistoryRepository for LinkingBackend
#[async_trait]
impl<B: StorageBackend> crate::storage::HistoryRepository for LinkingBackend<B> {
    async fn note_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<crate::storage::EntityVersion<Note>>, StorageError> {
        self.inner.note_history(id, limit).await
    }

    async fn notebook_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<crate::storage::EntityVersion<Notebook>>, StorageError> {
        self.inner.notebook_history(id, limit).await
    }
}

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::fs::FsBackend;

    // md:mod tests > fn backend
    async fn backend() -> LinkingBackend<FsBackend> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        LinkingBackend::new(FsBackend::new(&path).await.unwrap())
    }

    // md:mod tests > fn nb
    fn nb() -> Uuid {
        Uuid::from_u128(0x00b0_000c)
    }

    // md:mod tests > fn aliased
    fn aliased(title: &str, alias: &str) -> Note {
        let mut n = Note::new(title, "");
        n.alias = Some(alias.to_string());
        n.notebook_id = nb();
        n
    }

    // md:mod tests > fn derives_bookmarks_and_content_links
    #[tokio::test]
    async fn derives_bookmarks_and_content_links() {
        let be = backend().await;
        let body =
            "Intro [Bookmark1](###) and [Other](### \"Alias2\") and a [link](#notebook1#note3#1)";
        let stored = be.create_note(Note::new("t", body)).await.unwrap();

        assert_eq!(stored.bookmarks.len(), 2);
        assert_eq!(stored.bookmarks[0].number, 1);
        assert_eq!(stored.bookmarks[0].text, "Bookmark1");
        assert_eq!(stored.bookmarks[0].alias, "Bookmark1");
        assert_eq!(stored.bookmarks[1].number, 2);
        assert_eq!(stored.bookmarks[1].text, "Other");
        assert_eq!(stored.bookmarks[1].alias, "Alias2");

        assert_eq!(stored.links.len(), 1);
        assert_eq!(stored.links[0].source, LinkSource::Content);
        assert_eq!(stored.links[0].raw, "#notebook1#note3#1");
    }

    // md:mod tests > fn bookmark_alias_comes_from_the_body_title
    #[tokio::test]
    async fn bookmark_alias_comes_from_the_body_title() {
        let be = backend().await;
        let note = be
            .create_note(Note::new("t", "[Bookmark1](### \"Custom\") hi"))
            .await
            .unwrap();
        assert_eq!(note.bookmarks[0].text, "Bookmark1");
        assert_eq!(note.bookmarks[0].alias, "Custom");

        let mut note = note;
        note.body = "[Bookmark1](### \"Renamed\") hi, edited".to_string();
        let note = be.update_note(note).await.unwrap();
        assert_eq!(note.bookmarks[0].alias, "Renamed");
    }

    // md:mod tests > fn resolves_link_by_alias_and_uuid
    #[tokio::test]
    async fn resolves_link_by_alias_and_uuid() {
        let be = backend().await;
        let mut target = Note::new("target", "[Anchor](###) body");
        target.alias = Some("note3".to_string());
        target.notebook_id = nb();
        let target = be.create_note(target).await.unwrap();

        let mut src = Note::new("src", "go [here](#note3)");
        src.notebook_id = nb();
        let src = be.create_note(src).await.unwrap();
        assert_eq!(src.links[0].target_note_id, Some(target.id));

        let resolved = resolve(&be, &format!("#whatever#{}#Anchor", target.id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.note_id, target.id);
        assert_eq!(resolved.bookmark_number, Some(1));

        let (back, next) = backlinks(&be, target.id, 0, None).await.unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].id, src.id);
        assert!(next.is_none());
    }

    // md:mod tests > fn rejects_duplicate_note_alias
    #[tokio::test]
    async fn rejects_duplicate_note_alias() {
        let be = backend().await;
        be.create_note(aliased("a", "dup")).await.unwrap();
        let err = be.create_note(aliased("b", "dup")).await.unwrap_err();
        assert!(matches!(err, StorageError::Conflict(_)));
    }

    // md:mod tests > fn same_alias_in_different_notebooks_is_allowed
    #[tokio::test]
    async fn same_alias_in_different_notebooks_is_allowed() {
        let be = backend().await;
        let mut a = aliased("a", "shared");
        a.notebook_id = Uuid::from_u128(1);
        be.create_note(a).await.unwrap();
        let mut b = aliased("b", "shared");
        b.notebook_id = Uuid::from_u128(2);
        be.create_note(b).await.unwrap();
    }

    // md:mod tests > fn inbox_note_cannot_carry_an_alias
    #[tokio::test]
    async fn inbox_note_cannot_carry_an_alias() {
        let be = backend().await;
        let mut n = Note::new("n", "");
        n.alias = Some("x".to_string());
        let stored = be.create_note(n).await.unwrap();
        assert!(stored.alias.is_none(), "Inbox notes carry no alias");
    }

    // md:mod tests > fn set_note_alias_rejects_inbox_notes
    #[tokio::test]
    async fn set_note_alias_rejects_inbox_notes() {
        let be = backend().await;
        let inbox_note = be.create_note(Note::new("i", "")).await.unwrap();
        let err = set_note_alias(&be, inbox_note.id, Some("x".into()))
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)), "got: {err}");
        let cleared = set_note_alias(&be, inbox_note.id, None).await.unwrap();
        assert!(cleared.alias.is_none());
    }

    // md:mod tests > fn moving_a_note_to_inbox_clears_its_alias
    #[tokio::test]
    async fn moving_a_note_to_inbox_clears_its_alias() {
        let be = backend().await;
        let mut n = be.create_note(aliased("n", "keep")).await.unwrap();
        assert_eq!(n.alias.as_deref(), Some("keep"));
        n.notebook_id = crate::ordering::INBOX_ID;
        let moved = be.update_note(n).await.unwrap();
        assert!(moved.alias.is_none());
    }

    // md:mod tests > fn inbox_note_does_not_link_out
    #[tokio::test]
    async fn inbox_note_does_not_link_out() {
        let be = backend().await;
        let target = be.create_note(aliased("t", "tgt")).await.unwrap();
        let src = be
            .create_note(Note::new("s", "see [t](#tgt)"))
            .await
            .unwrap();
        assert_eq!(
            src.links[0].target_note_id, None,
            "Inbox notes do not link out"
        );
        let (back, _) = backlinks(&be, target.id, 0, None).await.unwrap();
        assert!(back.is_empty());
    }

    // md:mod tests > fn nothing_links_to_an_inbox_note
    #[tokio::test]
    async fn nothing_links_to_an_inbox_note() {
        let be = backend().await;
        let inbox_note = be.create_note(Note::new("i", "")).await.unwrap();
        let mut src = Note::new("s", format!("go [x](#{})", inbox_note.id));
        src.notebook_id = nb();
        let src = be.create_note(src).await.unwrap();
        assert_eq!(src.links[0].target_note_id, None);
        let (back, _) = backlinks(&be, inbox_note.id, 0, None).await.unwrap();
        assert!(back.is_empty(), "Inbox notes have no backlinks");
    }

    // md:mod tests > fn bare_alias_resolves_globally_when_unique_else_scoped
    #[tokio::test]
    async fn bare_alias_resolves_globally_when_unique_else_scoped() {
        let be = backend().await;
        let nb1 = Uuid::from_u128(1);
        let nb2 = Uuid::from_u128(2);
        let mut only = aliased("only", "only");
        only.notebook_id = nb1;
        let only = be.create_note(only).await.unwrap();
        let mut elsewhere = Note::new("e", "go [x](#only)");
        elsewhere.notebook_id = nb2;
        let elsewhere = be.create_note(elsewhere).await.unwrap();
        assert_eq!(elsewhere.links[0].target_note_id, Some(only.id));

        let mut dup1 = aliased("d1", "dup");
        dup1.notebook_id = nb1;
        let dup1 = be.create_note(dup1).await.unwrap();
        let mut dup2 = aliased("d2", "dup");
        dup2.notebook_id = nb2;
        be.create_note(dup2).await.unwrap();

        let mut src = Note::new("src", "go [x](#dup)");
        src.notebook_id = nb1;
        let src = be.create_note(src).await.unwrap();
        assert_eq!(
            src.links[0].target_note_id,
            Some(dup1.id),
            "bare ambiguous alias scopes to the source notebook"
        );
    }

    // md:mod tests > fn alias_and_link_edits_reject_deleted_entities
    #[tokio::test]
    async fn alias_and_link_edits_reject_deleted_entities() {
        let be = backend().await;

        let note = be.create_note(Note::new("n", "")).await.unwrap();
        be.delete_note(note.id).await.unwrap();
        for err in [
            set_note_alias(&be, note.id, Some("ghost".into()))
                .await
                .unwrap_err(),
            add_manual_link(&be, note.id, "#target").await.unwrap_err(),
            remove_link(&be, note.id, 0).await.unwrap_err(),
        ] {
            assert!(matches!(err, StorageError::NotFound(_)), "got: {err}");
        }
        let read = be.read_note(note.id).await.unwrap();
        assert!(read.deleted_at.is_some(), "note must remain tombstoned");

        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        be.delete_notebook(nb.id).await.unwrap();
        let err = set_notebook_alias(&be, nb.id, Some("ghost".into()))
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)), "got: {err}");
    }

    // md:mod tests > fn add_and_remove_manual_link
    #[tokio::test]
    async fn add_and_remove_manual_link() {
        let be = backend().await;
        let note = be
            .create_note(Note::new("a", "no links here"))
            .await
            .unwrap();
        let note = add_manual_link(&be, note.id, "#somealias").await.unwrap();
        assert_eq!(note.links.len(), 1);
        assert_eq!(note.links[0].source, LinkSource::Manual);

        let note = remove_link(&be, note.id, 0).await.unwrap();
        assert!(note.links.is_empty());
    }

    // md:mod tests > fn resolves_two_segment_note_bookmark_shorthand
    #[tokio::test]
    async fn resolves_two_segment_note_bookmark_shorthand() {
        let be = backend().await;
        let mut target = Note::new("target", "[Anchor](###) body");
        target.alias = Some("note3".to_string());
        target.notebook_id = nb();
        let target = be.create_note(target).await.unwrap();

        let r = resolve(&be, "#note3#Anchor").await.unwrap().unwrap();
        assert_eq!(r.note_id, target.id);
        assert_eq!(r.bookmark_number, Some(1));

        let r = resolve(&be, "#note3#1").await.unwrap().unwrap();
        assert_eq!(r.note_id, target.id);
        assert_eq!(r.bookmark_number, Some(1));
    }

    // md:mod tests > fn two_segment_prefers_notebook_note
    #[tokio::test]
    async fn two_segment_prefers_notebook_note() {
        let be = backend().await;
        let mut nb = Notebook::new("lib");
        nb.alias = Some("lib1".to_string());
        let nb = be.create_notebook(nb).await.unwrap();

        let mut note = Note::new("n", "");
        note.alias = Some("nA".to_string());
        note.notebook_id = nb.id;
        let note = be.create_note(note).await.unwrap();

        let r = resolve(&be, "#lib1#nA").await.unwrap().unwrap();
        assert_eq!(r.note_id, note.id);
        assert_eq!(r.bookmark_number, None);
    }

    // md:mod tests > fn alias_conflicts_lists_duplicates
    #[tokio::test]
    async fn alias_conflicts_lists_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let fs = FsBackend::new(&path).await.unwrap();

        for title in ["a", "b"] {
            let mut n = Note::new(title, "");
            n.alias = Some("dup".to_string());
            n.notebook_id = nb();
            fs.create_note(n).await.unwrap();
        }
        let mut unique = Note::new("c", "");
        unique.alias = Some("unique".to_string());
        unique.notebook_id = nb();
        fs.create_note(unique).await.unwrap();

        let conflicts = alias_conflicts(&fs).await.unwrap();
        assert_eq!(conflicts.notes.len(), 1);
        assert_eq!(conflicts.notes[0].alias, "dup");
        assert_eq!(conflicts.notes[0].entities.len(), 2);
        assert!(conflicts.notebooks.is_empty());
    }

    // md:mod tests > fn alias_index_tracks_deletes_and_renames
    #[tokio::test]
    async fn alias_index_tracks_deletes_and_renames() {
        let be = backend().await;

        let a = be.create_note(aliased("a", "freed")).await.unwrap();
        be.delete_note(a.id).await.unwrap();
        be.create_note(aliased("b", "freed")).await.unwrap();

        let mut c = be.create_note(aliased("c", "old")).await.unwrap();
        c.alias = Some("new".to_string());
        be.update_note(c).await.unwrap();

        be.create_note(aliased("d", "old")).await.unwrap();

        let err = be.create_note(aliased("e", "new")).await.unwrap_err();
        assert!(matches!(err, StorageError::Conflict(_)));
    }

    // md:mod tests > fn sync_applied_change_invalidates_alias_index
    #[tokio::test]
    async fn sync_applied_change_invalidates_alias_index() {
        let be = backend().await;

        let mut warm = Note::new("warm", "");
        warm.alias = Some("warm".to_string());
        be.create_note(warm).await.unwrap();

        let mut nb = Notebook::new("remote");
        nb.alias = Some("synced".to_string());
        be.apply_change(Change::NotebookCreate { notebook: nb })
            .await
            .unwrap();

        let mut local = Notebook::new("local");
        local.alias = Some("synced".to_string());
        let err = be.create_notebook(local).await.unwrap_err();
        assert!(matches!(err, StorageError::Conflict(_)), "got: {err:?}");
    }

    // md:mod tests > fn concurrent_duplicate_alias_yields_exactly_one_winner
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_duplicate_alias_yields_exactly_one_winner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let be = Arc::new(LinkingBackend::new(FsBackend::new(&path).await.unwrap()));

        let mut handles = Vec::new();
        for i in 0..8 {
            let b = Arc::clone(&be);
            handles.push(tokio::spawn(async move {
                let mut note = Note::new(format!("n{i}"), "");
                note.alias = Some("dup".to_string());
                note.notebook_id = nb();
                b.create_note(note).await
            }));
        }

        let (mut ok, mut conflict) = (0, 0);
        for h in handles {
            match h.await.unwrap() {
                Ok(_) => ok += 1,
                Err(StorageError::Conflict(_)) => conflict += 1,
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        assert_eq!(ok, 1, "exactly one create wins the alias");
        assert_eq!(conflict, 7, "the rest are rejected");
    }
}
