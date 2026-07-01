//! `LinkingBackend<B>`: derives bookmarks/links from note bodies and resolves references.
//!
//! This decorator wraps any [`StorageBackend`] and, on every note create/update, rewrites
//! the note's `bookmarks` and `links` from its markdown body before delegating to `inner`,
//! then enforces that note/notebook `alias`es are unique. It mirrors the decorator pattern
//! of [`crate::encryption::EncryptedBackend`].
//!
//! # Placement in the decorator stack
//!
//! `LinkingBackend` must sit **outside** any `EncryptedBackend` (so it parses the
//! **plaintext** body and resolves aliases against decrypted reads) and **inside**
//! `EventBackend` (so the live feed carries the refreshed metadata):
//! `EventBackend( LinkingBackend( [EncryptedBackend]( Fs|Db ) ) )`.
//!
//! # What it does on write
//!
//! 1. **Bookmarks** — `[text](### "alias")` markdown links in the body become numbered
//!    [`Bookmark`]s in order of appearance. The body is the single source of truth: the alias
//!    is the link title (defaulting to the text), edited by editing the body.
//! 2. **Links** — markdown `[t](#…)` destinations become `source = Content` [`NoteLink`]s;
//!    existing `source = Manual` links (added via the API) are preserved.
//! 3. **Resolution** — each link's `target_note_id` is filled best-effort by resolving its
//!    note reference (by uuid or, scanning live notes, by alias).
//! 4. **Alias uniqueness** — a create/update whose `alias` collides with another **live**
//!    entity of the same type is rejected with [`StorageError::Conflict`].
//!
//! Reads, sync (`apply_change`) and the other entities delegate unchanged. Cross-device
//! concurrent edits can still introduce duplicate aliases through sync (which cannot be
//! rejected); resolution then picks the smallest-uuid match deterministically and warns.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    error::StorageError,
    links::{self, Bookmark, LinkSource, NoteLink},
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
    storage::{
        NoteRepository, NotebookRepository, ResourceRepository, StorageBackend, SyncBackend,
        TagRepository,
    },
};

/// Page size used when scanning every live note/notebook for resolution and uniqueness.
const SCAN_PAGE: u32 = 500;

/// A resolved `#…` reference: the concrete target note and, when the reference named a
/// bookmark, its 1-based number within that note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedReference {
    /// The UUID of the target note.
    pub note_id: Uuid,
    /// The 1-based bookmark number, when the reference had a (resolved) bookmark segment.
    pub bookmark_number: Option<u32>,
}

/// One alias shared by two or more live entities of the same type — the residue of a
/// cross-device alias collision that sync could not reject.
#[derive(Debug, Clone, Serialize)]
pub struct AliasConflict<T> {
    /// The duplicated alias.
    pub alias: String,
    /// The colliding entities, ordered by uuid (the smallest is what resolution prefers).
    pub entities: Vec<T>,
}

/// All current alias collisions, grouped by entity type. Empty vectors mean no conflicts.
#[derive(Debug, Clone, Serialize)]
pub struct AliasConflicts {
    pub notes: Vec<AliasConflict<Note>>,
    pub notebooks: Vec<AliasConflict<Notebook>>,
}

/// Decorator that maintains bookmarks/links and enforces alias uniqueness.
pub struct LinkingBackend<B> {
    inner: B,
    /// Serialises alias-bearing writes so the "scan for a duplicate, then write" sequence is
    /// atomic. Without it, two concurrent writes claiming the same alias could each pass the
    /// uniqueness check before either is persisted, creating a local duplicate. Only taken
    /// when the entity actually carries an alias, so plain notes never serialise here.
    alias_write_lock: Arc<Mutex<()>>,
}

impl<B: StorageBackend> LinkingBackend<B> {
    /// Wrap `inner`.
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            alias_write_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Rewrite `note.bookmarks` and `note.links` from its body (pure, no I/O).
    fn refresh(note: &mut Note) {
        // Bookmarks: the body is the single source of truth. Each `[text](### "alias")`
        // declaration becomes a numbered bookmark; the alias is the link title, defaulting to
        // the link text when the title is omitted or empty.
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

        // Links: keep manual ones, re-derive content ones from the body.
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

    /// Fill each link's `target_note_id` from the supplied snapshots of live entities.
    fn resolve_into(note: &mut Note, notes: &[Note], notebooks: &[Notebook]) {
        for link in &mut note.links {
            link.target_note_id = resolve_ref(&link.raw, notes, notebooks).map(|r| r.note_id);
        }
    }

    /// Reject a note whose alias collides with another live note.
    fn ensure_note_alias_unique(note: &Note, notes: &[Note]) -> Result<(), StorageError> {
        if let Some(alias) = &note.alias {
            if notes
                .iter()
                .any(|n| n.id != note.id && n.alias.as_deref() == Some(alias.as_str()))
            {
                return Err(StorageError::Conflict(format!(
                    "note alias '{alias}' is already in use"
                )));
            }
        }
        Ok(())
    }

    /// Reject a notebook whose alias collides with another live notebook.
    fn ensure_notebook_alias_unique(
        notebook: &Notebook,
        notebooks: &[Notebook],
    ) -> Result<(), StorageError> {
        if let Some(alias) = &notebook.alias {
            if notebooks
                .iter()
                .any(|nb| nb.id != notebook.id && nb.alias.as_deref() == Some(alias.as_str()))
            {
                return Err(StorageError::Conflict(format!(
                    "notebook alias '{alias}' is already in use"
                )));
            }
        }
        Ok(())
    }

    /// Prepare a note for a create/update: refresh its derived bookmarks/links, then — only
    /// when needed — scan the corpus to enforce alias uniqueness and resolve link targets.
    ///
    /// The corpus scan (`collect_notes`/`collect_notebooks`) is expensive — on `FsBackend` it
    /// re-materialises every note — so it is skipped entirely for the common case of a note
    /// with no alias and no links. Notes are fetched only when an alias must be checked or
    /// links resolved; notebooks only when there are links to resolve.
    async fn prepare(&self, note: &mut Note) -> Result<(), StorageError> {
        Self::refresh(note);
        let needs_notes = note.alias.is_some() || !note.links.is_empty();
        if !needs_notes {
            return Ok(());
        }
        let notes = collect_notes(&self.inner).await?;
        Self::ensure_note_alias_unique(note, &notes)?;
        if !note.links.is_empty() {
            let notebooks = collect_notebooks(&self.inner).await?;
            Self::resolve_into(note, &notes, &notebooks);
        }
        Ok(())
    }
}

// ── Free helpers usable through a type-erased `&dyn StorageBackend` ───────────────
//
// The decorator is wrapped behind `Arc<dyn StorageBackend>`, so the surfaces (REST/gRPC)
// cannot call its inherent methods. These free functions operate purely through the
// `StorageBackend` trait — their writes flow back through `LinkingBackend`, so derivation,
// resolution and uniqueness all still apply.

/// Collect every live note by exhausting the paginated `list_notes`.
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

/// Collect every live notebook by exhausting the paginated `list_notebooks`.
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

/// Resolve a notebook segment (uuid or alias) to a uuid against live notebooks. A uuid is
/// returned as-is (existence is not checked); an alias picks the smallest-uuid live match.
fn resolve_notebook_seg(seg: &str, notebooks: &[Notebook]) -> Option<Uuid> {
    if let Ok(id) = Uuid::parse_str(seg) {
        return Some(id);
    }
    notebooks
        .iter()
        .filter(|nb| nb.alias.as_deref() == Some(seg))
        .map(|nb| nb.id)
        .min()
}

/// Resolve a note segment (uuid or alias) to a uuid against live notes, optionally scoped to
/// a notebook segment, breaking alias ties by smallest uuid. A uuid is returned as-is; an
/// alias that matches no live note yields `None` (which drives the 2-segment fallback).
fn resolve_note_seg(
    seg: &str,
    notebook_seg: Option<&str>,
    notes: &[Note],
    notebooks: &[Notebook],
) -> Option<Uuid> {
    if let Ok(id) = Uuid::parse_str(seg) {
        return Some(id);
    }
    let nb_id = notebook_seg.and_then(|ns| resolve_notebook_seg(ns, notebooks));
    let mut candidates: Vec<&Note> = notes
        .iter()
        .filter(|n| n.alias.as_deref() == Some(seg))
        .collect();
    if let Some(nb) = nb_id {
        let scoped: Vec<&Note> = candidates
            .iter()
            .copied()
            .filter(|n| n.notebook_id == Some(nb))
            .collect();
        if !scoped.is_empty() {
            candidates = scoped;
        }
    }
    if candidates.len() > 1 {
        tracing::warn!(alias = %seg, "ambiguous note alias; resolving to smallest uuid");
    }
    candidates.into_iter().map(|n| n.id).min()
}

/// Map a bookmark segment (number or alias) to a stored bookmark number within `note_id`,
/// using the note found in the `notes` snapshot. Returns `None` when the note is not in the
/// snapshot or has no matching bookmark.
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

/// Resolve a raw `#…` reference against snapshots of live notes/notebooks (pure).
///
/// Segment interpretation:
/// - `#note`
/// - `#notebook#note` — preferred when the second segment resolves to a note; otherwise the
///   reference is re-read as `#note#bookmark` (so a bookmark can be targeted without naming a
///   notebook).
/// - `#notebook#note#bookmark`
fn resolve_ref(raw: &str, notes: &[Note], notebooks: &[Notebook]) -> Option<ResolvedReference> {
    let body = raw.strip_prefix('#')?;
    let segments: Vec<&str> = body.split('#').collect();
    if segments.iter().any(|s| s.is_empty()) {
        return None;
    }
    let (note_id, bookmark_number) = match segments.as_slice() {
        [note] => (resolve_note_seg(note, None, notes, notebooks)?, None),
        [first, second] => {
            // Prefer notebook#note; fall back to note#bookmark when the second segment is
            // not a resolvable note.
            if let Some(id) = resolve_note_seg(second, Some(first), notes, notebooks) {
                (id, None)
            } else {
                let id = resolve_note_seg(first, None, notes, notebooks)?;
                (id, resolve_bookmark_seg(second, id, notes))
            }
        }
        [notebook, note, bookmark] => {
            let id = resolve_note_seg(note, Some(notebook), notes, notebooks)?;
            (id, resolve_bookmark_seg(bookmark, id, notes))
        }
        _ => return None,
    };
    Some(ResolvedReference {
        note_id,
        bookmark_number,
    })
}

/// Resolve a raw `#…` reference to a concrete note (and bookmark number) against the store.
pub async fn resolve(
    backend: &dyn StorageBackend,
    raw: &str,
) -> Result<Option<ResolvedReference>, StorageError> {
    let notes = collect_notes(backend).await?;
    let notebooks = collect_notebooks(backend).await?;
    Ok(resolve_ref(raw, &notes, &notebooks))
}

/// Return a page of the live notes that link to `target_id`.
///
/// Delegates to [`NoteRepository::note_backlinks`](crate::storage::NoteRepository::note_backlinks),
/// which `DbBackend` answers with an indexed, paginated lookup and other backends with an
/// `O(N)` scan paginated in memory.
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

/// Group `items` by their (optional) alias, keeping only aliases shared by two or more
/// entities. Groups are ordered by alias; entities within a group are ordered by uuid.
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

/// List every alias currently shared by two or more **live** notes (or notebooks).
///
/// Local writes reject duplicate aliases, but sync replays edits made independently on other
/// devices, so a collision can still appear after a sync. This surfaces those collisions so a
/// human can rename one side; resolution itself stays deterministic in the meantime.
pub async fn alias_conflicts(backend: &dyn StorageBackend) -> Result<AliasConflicts, StorageError> {
    let notes = collect_notes(backend).await?;
    let notebooks = collect_notebooks(backend).await?;
    Ok(AliasConflicts {
        notes: group_conflicts(notes, |n| n.alias.clone(), |n| n.id),
        notebooks: group_conflicts(notebooks, |nb| nb.alias.clone(), |nb| nb.id),
    })
}

/// Set (or clear) a note's alias and persist it (read-modify-write → one `NoteUpdate`).
pub async fn set_note_alias(
    backend: &dyn StorageBackend,
    note_id: Uuid,
    alias: Option<String>,
) -> Result<Note, StorageError> {
    let mut note = backend.read_note(note_id).await?;
    note.alias = alias;
    backend.update_note(note).await
}

/// Set (or clear) a notebook's alias and persist it.
pub async fn set_notebook_alias(
    backend: &dyn StorageBackend,
    notebook_id: Uuid,
    alias: Option<String>,
) -> Result<Notebook, StorageError> {
    let mut notebook = backend.read_notebook(notebook_id).await?;
    notebook.alias = alias;
    backend.update_notebook(notebook).await
}

/// Add a manual (global) link from `note_id` to a raw `#…` reference. Returns the note.
pub async fn add_manual_link(
    backend: &dyn StorageBackend,
    note_id: Uuid,
    raw: &str,
) -> Result<Note, StorageError> {
    let link = NoteLink::from_raw(raw, LinkSource::Manual)
        .ok_or_else(|| StorageError::InvalidState(format!("invalid link reference '{raw}'")))?;
    let mut note = backend.read_note(note_id).await?;
    note.links.push(link);
    backend.update_note(note).await
}

/// Remove the link at `index` (into the note's `links`) and persist. Returns the note.
pub async fn remove_link(
    backend: &dyn StorageBackend,
    note_id: Uuid,
    index: usize,
) -> Result<Note, StorageError> {
    let mut note = backend.read_note(note_id).await?;
    if index >= note.links.len() {
        return Err(StorageError::NotFound(format!(
            "link {index} in note {note_id}"
        )));
    }
    note.links.remove(index);
    backend.update_note(note).await
}

// ── Sub-trait impls ──────────────────────────────────────────────────────────────

#[async_trait]
impl<B: StorageBackend> NoteRepository for LinkingBackend<B> {
    async fn create_note(&self, mut note: Note) -> Result<Note, StorageError> {
        // Hold the lock across the uniqueness check + write only when an alias is involved.
        let _guard = if note.alias.is_some() {
            Some(self.alias_write_lock.lock().await)
        } else {
            None
        };
        self.prepare(&mut note).await?;
        self.inner.create_note(note).await
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
        self.inner.update_note(note).await
    }

    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_note(id).await
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
        // Delegate so an inner indexed backend (e.g. DbBackend) is reached.
        self.inner
            .note_backlinks(target_id, page_size, page_token)
            .await
    }
}

#[async_trait]
impl<B: StorageBackend> NotebookRepository for LinkingBackend<B> {
    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        if notebook.alias.is_none() {
            return self.inner.create_notebook(notebook).await;
        }
        let _guard = self.alias_write_lock.lock().await;
        let notebooks = collect_notebooks(&self.inner).await?;
        Self::ensure_notebook_alias_unique(&notebook, &notebooks)?;
        self.inner.create_notebook(notebook).await
    }

    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        self.inner.read_notebook(id).await
    }

    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        if notebook.alias.is_none() {
            return self.inner.update_notebook(notebook).await;
        }
        let _guard = self.alias_write_lock.lock().await;
        let notebooks = collect_notebooks(&self.inner).await?;
        Self::ensure_notebook_alias_unique(&notebook, &notebooks)?;
        self.inner.update_notebook(notebook).await
    }

    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_notebook(id).await
    }

    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        self.inner.list_notebooks(page_size, page_token).await
    }
}

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
}

// Sync delegates unchanged: a synced note already carries derived metadata from the origin
// device, and `target_note_id`s are global uuids, so no re-derivation is needed. Alias
// uniqueness is best-effort and cannot be enforced against incoming sync.
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
        self.inner.apply_change(change).await
    }

    async fn send_changes(&self, changes: Vec<Change>) -> Result<(), StorageError> {
        self.inner.send_changes(changes).await
    }

    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        self.inner.receive_changes().await
    }

    async fn prune_change_journal(&self, older_than: DateTime<Utc>) -> Result<u64, StorageError> {
        self.inner.prune_change_journal(older_than).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::fs::FsBackend;

    async fn backend() -> LinkingBackend<FsBackend> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        LinkingBackend::new(FsBackend::new(&path).await.unwrap())
    }

    #[tokio::test]
    async fn derives_bookmarks_and_content_links() {
        let be = backend().await;
        let body =
            "Intro [Bookmark1](###) and [Other](### \"Alias2\") and a [link](#notebook1#note3#1)";
        let stored = be.create_note(Note::new("t", body)).await.unwrap();

        assert_eq!(stored.bookmarks.len(), 2);
        assert_eq!(stored.bookmarks[0].number, 1);
        assert_eq!(stored.bookmarks[0].text, "Bookmark1");
        // No title → alias defaults to the link text.
        assert_eq!(stored.bookmarks[0].alias, "Bookmark1");
        assert_eq!(stored.bookmarks[1].number, 2);
        assert_eq!(stored.bookmarks[1].text, "Other");
        // Title present → alias is the title.
        assert_eq!(stored.bookmarks[1].alias, "Alias2");

        assert_eq!(stored.links.len(), 1);
        assert_eq!(stored.links[0].source, LinkSource::Content);
        assert_eq!(stored.links[0].raw, "#notebook1#note3#1");
    }

    #[tokio::test]
    async fn bookmark_alias_comes_from_the_body_title() {
        let be = backend().await;
        // The alias lives in the body (the link title); editing the body changes it.
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

    #[tokio::test]
    async fn resolves_link_by_alias_and_uuid() {
        let be = backend().await;
        // Target note with alias "note3".
        let mut target = Note::new("target", "[Anchor](###) body");
        target.alias = Some("note3".to_string());
        let target = be.create_note(target).await.unwrap();

        // Source note linking to it by alias.
        let src = be
            .create_note(Note::new("src", "go [here](#note3)"))
            .await
            .unwrap();
        assert_eq!(src.links[0].target_note_id, Some(target.id));

        // Resolve a 3-segment ref to note + bookmark number 1.
        let resolved = resolve(&be, &format!("#whatever#{}#Anchor", target.id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.note_id, target.id);
        assert_eq!(resolved.bookmark_number, Some(1));

        // Backlinks: target is linked by src.
        let (back, next) = backlinks(&be, target.id, 0, None).await.unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].id, src.id);
        assert!(next.is_none());
    }

    #[tokio::test]
    async fn rejects_duplicate_note_alias() {
        let be = backend().await;
        let mut a = Note::new("a", "");
        a.alias = Some("dup".to_string());
        be.create_note(a).await.unwrap();

        let mut b = Note::new("b", "");
        b.alias = Some("dup".to_string());
        let err = be.create_note(b).await.unwrap_err();
        assert!(matches!(err, StorageError::Conflict(_)));
    }

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

    #[tokio::test]
    async fn resolves_two_segment_note_bookmark_shorthand() {
        let be = backend().await;
        let mut target = Note::new("target", "[Anchor](###) body");
        target.alias = Some("note3".to_string());
        let target = be.create_note(target).await.unwrap();

        // `#note#bookmark` by bookmark alias.
        let r = resolve(&be, "#note3#Anchor").await.unwrap().unwrap();
        assert_eq!(r.note_id, target.id);
        assert_eq!(r.bookmark_number, Some(1));

        // `#note#bookmark` by bookmark number.
        let r = resolve(&be, "#note3#1").await.unwrap().unwrap();
        assert_eq!(r.note_id, target.id);
        assert_eq!(r.bookmark_number, Some(1));
    }

    #[tokio::test]
    async fn two_segment_prefers_notebook_note() {
        let be = backend().await;
        let mut nb = Notebook::new("lib");
        nb.alias = Some("lib1".to_string());
        let nb = be.create_notebook(nb).await.unwrap();

        let mut note = Note::new("n", "");
        note.alias = Some("nA".to_string());
        note.notebook_id = Some(nb.id);
        let note = be.create_note(note).await.unwrap();

        // `#notebook#note` resolves to the note (not interpreted as note#bookmark).
        let r = resolve(&be, "#lib1#nA").await.unwrap().unwrap();
        assert_eq!(r.note_id, note.id);
        assert_eq!(r.bookmark_number, None);
    }

    #[tokio::test]
    async fn alias_conflicts_lists_duplicates() {
        // A raw FsBackend (no LinkingBackend) lets us plant a duplicate alias the way sync
        // would, bypassing the write-time uniqueness check.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let fs = FsBackend::new(&path).await.unwrap();

        for title in ["a", "b"] {
            let mut n = Note::new(title, "");
            n.alias = Some("dup".to_string());
            fs.create_note(n).await.unwrap();
        }
        let mut unique = Note::new("c", "");
        unique.alias = Some("unique".to_string());
        fs.create_note(unique).await.unwrap();

        let conflicts = alias_conflicts(&fs).await.unwrap();
        assert_eq!(conflicts.notes.len(), 1);
        assert_eq!(conflicts.notes[0].alias, "dup");
        assert_eq!(conflicts.notes[0].entities.len(), 2);
        assert!(conflicts.notebooks.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_duplicate_alias_yields_exactly_one_winner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let be = Arc::new(LinkingBackend::new(FsBackend::new(&path).await.unwrap()));

        // Eight concurrent creates all claim alias "dup"; the write lock must let exactly one
        // through and reject the rest as conflicts (no local duplicate).
        let mut handles = Vec::new();
        for i in 0..8 {
            let b = Arc::clone(&be);
            handles.push(tokio::spawn(async move {
                let mut note = Note::new(format!("n{i}"), "");
                note.alias = Some("dup".to_string());
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
