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
//!    note reference (by uuid, or by alias through the in-memory alias index). Notes in the
//!    **Inbox** (the "Pizarra"; [`crate::ordering::is_inbox`]) are never link targets: a
//!    reference that names one — whether by alias or by raw uuid — resolves to nothing, and
//!    those notes themselves carry no alias and emit no links.
//! 4. **Alias uniqueness** — note aliases are unique **per notebook**, not globally: the same
//!    alias may live in two different notebooks, but a create/update whose `alias` collides
//!    with another **live** note in the *same* notebook is rejected with
//!    [`StorageError::Conflict`]. Notebook aliases remain globally unique.
//!
//! A bare `#alias` reference resolves globally when exactly one live note carries the alias;
//! when several notebooks share it, resolution scopes to the referencing note's own notebook
//! (see [`AliasIndex::resolve_note_seg`]).
//!
//! Reads, sync (`apply_change`) and the other entities delegate unchanged. Cross-device
//! concurrent edits can still introduce duplicate aliases through sync (which cannot be
//! rejected); resolution then picks the smallest-uuid match deterministically and warns.
//!
//! # The alias index
//!
//! Uniqueness checks and link-target resolution only need the alias → live-entity mapping,
//! so the decorator keeps an in-memory [`AliasIndex`] instead of re-scanning the corpus on
//! every alias- or link-bearing write (on `FsBackend` a scan re-materialises every note).
//! The index is built lazily by one full scan on the first write that needs it, updated
//! incrementally by every write that flows through this decorator, and **invalidated**
//! (rebuilt on next use) whenever a sync `apply_change`/`receive_changes` touches a note or
//! notebook — sync outcomes depend on conflict resolution inside the inner backend, so
//! reflecting them incrementally would risk drift. Writes that bypass the decorator (e.g.
//! a second process on the same store) are not visible until the next invalidation; the
//! daemon routes every surface and the sync engine through one shared decorator stack, so
//! within a daemon the index stays coherent.

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

/// The in-memory alias → live-entity mapping used for uniqueness checks and reference
/// resolution (see the module docs). Only **live** (non-tombstoned) aliased entities are
/// indexed, so its size is bounded by the number of aliases, not the corpus.
#[derive(Debug, Default)]
struct AliasIndex {
    /// alias → the live notes carrying it, as `(note_id, notebook_id)` so scoped resolution
    /// (`#notebook#note`) works without the full note. Ordered sets make the smallest-uuid
    /// tiebreak deterministic.
    note_aliases: BTreeMap<String, BTreeSet<(Uuid, Uuid)>>,
    /// note id → its indexed `(alias, notebook_id)`, so an edit can remove the old entry.
    aliased_notes: HashMap<Uuid, (String, Uuid)>,
    /// alias → the live notebooks carrying it.
    notebook_aliases: BTreeMap<String, BTreeSet<Uuid>>,
    /// notebook id → its indexed alias.
    aliased_notebooks: HashMap<Uuid, String>,
}

impl AliasIndex {
    /// Build the index from full snapshots of live notes and notebooks.
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

    /// Reflect a note's current state: its previous entry (if any) is dropped, and a live
    /// aliased note is (re-)inserted. A tombstoned or alias-less note simply disappears.
    fn upsert_note(&mut self, note: &Note) {
        self.remove_note(note.id);
        if note.deleted_at.is_some() {
            return;
        }
        // Inbox notes carry no alias, so they are never indexed; the "nothing links to an
        // Inbox note" guarantee on the raw-uuid path is enforced by the callers instead
        // (a backend read in `prepare`, the snapshot check in `resolve_ref`), keeping the
        // index bounded by the alias count rather than the Inbox size.
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

    /// Drop a note's entry (used for deletes and as the first half of an upsert).
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

    /// Reflect a notebook's current state (see [`upsert_note`](Self::upsert_note)).
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

    /// Drop a notebook's entry.
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

    /// Whether `alias` is carried by another live note **in the same notebook**
    /// (`self_id` excluded). Note aliases are unique per notebook, not globally,
    /// so the same alias may live in two different notebooks.
    fn note_alias_taken(&self, alias: &str, self_id: Uuid, notebook_id: Uuid) -> bool {
        self.note_aliases.get(alias).is_some_and(|set| {
            set.iter()
                .any(|(id, nb)| *id != self_id && *nb == notebook_id)
        })
    }

    /// Whether `alias` is carried by a live notebook other than `self_id`.
    fn notebook_alias_taken(&self, alias: &str, self_id: Uuid) -> bool {
        self.notebook_aliases
            .get(alias)
            .is_some_and(|set| set.iter().any(|id| *id != self_id))
    }

    /// Resolve a notebook segment (uuid or alias) to a uuid. A uuid is returned as-is
    /// (existence is not checked); an alias picks the smallest-uuid live match.
    fn resolve_notebook_seg(&self, seg: &str) -> Option<Uuid> {
        if let Ok(id) = Uuid::parse_str(seg) {
            return Some(id);
        }
        self.notebook_aliases
            .get(seg)
            .and_then(|set| set.iter().next().copied())
    }

    /// Resolve a note segment (uuid or alias) to a uuid.
    ///
    /// - A uuid is returned as-is.
    /// - With an explicit `notebook_seg`, the alias is scoped to that notebook.
    /// - A **bare** alias resolves globally when it is the only live note carrying it;
    ///   when several notebooks share the alias it is scoped to `source_notebook`
    ///   (the notebook of the note that carries the link).
    ///
    /// Notes in the **Inbox** are never link targets and are excluded from alias matches.
    /// An alias that matches no eligible live note yields `None` (driving the 2-segment
    /// fallback).
    fn resolve_note_seg(
        &self,
        seg: &str,
        notebook_seg: Option<&str>,
        source_notebook: Option<Uuid>,
    ) -> Option<Uuid> {
        if let Ok(id) = Uuid::parse_str(seg) {
            // A raw uuid is returned as-is. Inbox notes are never link targets, but that is
            // enforced by the callers (`prepare` verifies uuid-resolved targets with a
            // backend read; `resolve_ref` checks its notes snapshot) rather than indexed
            // here, so the index need not track every Inbox note id.
            return Some(id);
        }
        // Candidates carrying this alias, excluding Inbox notes (never targets).
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
        // Explicit notebook scoping: only that notebook (Inbox is not a target).
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
        // Bare alias: unique → resolve globally; otherwise scope to the source notebook.
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
        // No source context (standalone resolve): deterministic smallest-uuid.
        tracing::warn!(alias = %seg, "ambiguous note alias; resolving to smallest uuid");
        candidates.into_iter().map(|(id, _)| id).min()
    }

    /// Resolve a raw `#…` reference to its target note id plus the still-unresolved
    /// bookmark segment, if the reference carried one. This is the segment logic shared by
    /// write-time link resolution (which only needs the note id) and the full
    /// [`resolve_ref`] (which additionally maps the bookmark segment to a number).
    ///
    /// Segment interpretation:
    /// - `#note`
    /// - `#notebook#note` — preferred when the second segment resolves to a note; otherwise
    ///   the reference is re-read as `#note#bookmark` (so a bookmark can be targeted
    ///   without naming a notebook).
    /// - `#notebook#note#bookmark`
    ///
    /// `source_notebook` is the notebook of the note that carries the reference (when known),
    /// used to disambiguate a bare alias shared across notebooks (see [`resolve_note_seg`]).
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
                // Prefer notebook#note; fall back to note#bookmark when the second segment
                // is not a resolvable note.
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

/// Whether a sync change can move an alias (or a note between notebooks), requiring the
/// alias index to be rebuilt before its next use.
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

/// Decorator that maintains bookmarks/links and enforces alias uniqueness.
pub struct LinkingBackend<B> {
    inner: B,
    /// Serialises alias-bearing writes so the "check for a duplicate, then write" sequence
    /// is atomic. Without it, two concurrent writes claiming the same alias could each pass
    /// the uniqueness check before either is persisted, creating a local duplicate. Only
    /// taken when the entity actually carries an alias, so plain notes never serialise here.
    alias_write_lock: Arc<Mutex<()>>,
    /// The lazily built [`AliasIndex`]; `None` means "not built yet" or "invalidated by a
    /// sync change" — the next alias/link-bearing write rebuilds it with one corpus scan.
    alias_index: Arc<RwLock<Option<AliasIndex>>>,
}

impl<B: StorageBackend> LinkingBackend<B> {
    /// Wrap `inner`.
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            alias_write_lock: Arc::new(Mutex::new(())),
            alias_index: Arc::new(RwLock::new(None)),
        }
    }

    /// Run `f` against the alias index, building it first (one corpus scan) when it is
    /// absent or was invalidated. The double-checked write lock means concurrent callers
    /// trigger at most one build.
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

    /// Fold a successfully written note into the index (a no-op while the index is unbuilt
    /// — the next build scans the store, which already reflects this write).
    async fn index_upsert_note(&self, note: &Note) {
        if let Some(idx) = self.alias_index.write().await.as_mut() {
            idx.upsert_note(note);
        }
    }

    /// Drop a deleted note from the index.
    async fn index_remove_note(&self, id: Uuid) {
        if let Some(idx) = self.alias_index.write().await.as_mut() {
            idx.remove_note(id);
        }
    }

    /// Fold a successfully written notebook into the index.
    async fn index_upsert_notebook(&self, notebook: &Notebook) {
        if let Some(idx) = self.alias_index.write().await.as_mut() {
            idx.upsert_notebook(notebook);
        }
    }

    /// Drop a deleted notebook from the index.
    async fn index_remove_notebook(&self, id: Uuid) {
        if let Some(idx) = self.alias_index.write().await.as_mut() {
            idx.remove_notebook(id);
        }
    }

    /// Discard the index so the next use rebuilds it from the store. Called when a sync
    /// change lands: what actually got stored depends on conflict resolution inside the
    /// inner backend, so applying the change to the index directly could drift.
    async fn index_invalidate(&self) {
        *self.alias_index.write().await = None;
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

    /// Prepare a note for a create/update: refresh its derived bookmarks/links, then — only
    /// when needed — enforce alias uniqueness and resolve link targets against the
    /// [`AliasIndex`].
    ///
    /// The index is skipped entirely for the common case of a note with no alias and no
    /// links; the first write that does need it pays one corpus scan to build it, and
    /// subsequent writes are index lookups (no scan). Write-time resolution only needs each
    /// link's target **note id** — bookmark numbers are not stored on links — so the pure
    /// index suffices and no note bodies are fetched.
    async fn prepare(&self, note: &mut Note) -> Result<(), StorageError> {
        Self::refresh(note);
        // Inbox (Pizarra) notes carry no alias and do not link out: they cannot
        // point at other notebooks/notes/bookmarks, and moving a note into the
        // Inbox clears its alias.
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
                // Targets the index does not know as aliased notes were resolved through
                // the raw-uuid path and could name an Inbox note (never a link target);
                // they are verified against the store below. Alias-resolved targets are
                // always indexed, hence never in the Inbox.
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
        // One read per uuid-resolved target: a link into a live Inbox note does not resolve
        // (mirrors the old index-side rejection; a uuid that reads as missing or deleted
        // keeps resolving as-is, same as before).
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

    /// Reject a notebook whose alias collides with another live notebook (index lookup;
    /// a notebook without an alias passes immediately, no index needed).
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
/// The segment logic lives in [`AliasIndex::resolve_target`] (shared with write-time link
/// resolution); this wrapper additionally maps a bookmark segment to its stored number,
/// which needs the target note's bookmarks and therefore the `notes` snapshot.
fn resolve_ref(raw: &str, notes: &[Note], notebooks: &[Notebook]) -> Option<ResolvedReference> {
    let idx = AliasIndex::from_snapshots(notes, notebooks);
    // No source note here, so a bare ambiguous alias falls back deterministically.
    let (note_id, bookmark_seg) = idx.resolve_target(raw, None)?;
    // Inbox notes are never link targets — a uuid reference to one does not resolve
    // (alias references already exclude the Inbox inside `resolve_target`).
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
        notes: group_note_conflicts(notes),
        notebooks: group_conflicts(notebooks, |nb| nb.alias.clone(), |nb| nb.id),
    })
}

/// Group note conflicts by `(alias, notebook)`: note aliases are unique **per notebook**,
/// so the same alias in two different notebooks is not a conflict. Inbox notes carry no
/// alias and are skipped. Entities within a group are ordered by uuid.
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

/// Read a note for a user-facing read-modify-write, rejecting a tombstone as `NotFound`.
///
/// These helpers serve the API surfaces, which present soft-deleted entities as absent
/// (`404`/`NOT_FOUND`). Without this check the read-modify-write would silently *revive*
/// a deleted note (the update writes `deleted_at: None` back). Revival is reserved for
/// the sync path (`apply_change` resolving a causal edit made after the delete), never a
/// side effect of an alias or link edit.
async fn read_live_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    let note = backend.read_note(id).await?;
    if note.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()));
    }
    Ok(note)
}

/// Read a notebook for a user-facing read-modify-write, rejecting a tombstone as
/// `NotFound` (see [`read_live_note`]).
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

/// Set (or clear) a note's alias and persist it (read-modify-write → one `NoteUpdate`).
/// A soft-deleted note is `NotFound` — the edit must not revive it. Setting an alias on an
/// Inbox note is `InvalidInput`: Inbox notes carry no alias.
pub async fn set_note_alias(
    backend: &dyn StorageBackend,
    note_id: Uuid,
    alias: Option<String>,
) -> Result<Note, StorageError> {
    let mut note = read_live_note(backend, note_id).await?;
    // Inbox ("Pizarra") notes carry no alias; reject rather than silently clear, matching the
    // other Inbox domain rules (e.g. pinning). Clearing an already-null alias is a no-op.
    if alias.is_some() && is_inbox(note.notebook_id) {
        return Err(StorageError::InvalidInput(
            "an Inbox note cannot carry an alias".into(),
        ));
    }
    note.alias = alias;
    backend.update_note(note).await
}

/// Set (or clear) a notebook's alias and persist it. A soft-deleted notebook is
/// `NotFound` — the edit must not revive it.
pub async fn set_notebook_alias(
    backend: &dyn StorageBackend,
    notebook_id: Uuid,
    alias: Option<String>,
) -> Result<Notebook, StorageError> {
    let mut notebook = read_live_notebook(backend, notebook_id).await?;
    notebook.alias = alias;
    backend.update_notebook(notebook).await
}

/// Add a manual (global) link from `note_id` to a raw `#…` reference. Returns the note.
/// A soft-deleted note is `NotFound` — the edit must not revive it.
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

/// Remove the link at `index` (into the note's `links`) and persist. Returns the note.
/// A soft-deleted note is `NotFound` — the edit must not revive it.
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
        // Upsert even when no alias is involved: this edit may be *removing* one.
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
        // Inbox notes are never link targets, so they have no backlinks (this also
        // hides any stale target left by data written before this rule).
        if let Ok(note) = self.inner.read_note(target_id).await {
            if is_inbox(note.notebook_id) {
                return Ok((Vec::new(), None));
            }
        }
        // Delegate so an inner indexed backend (e.g. DbBackend) is reached.
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
        // Upsert even when no alias is involved: this edit may be *removing* one.
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

    async fn purge_deleted_resources(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, StorageError> {
        self.inner.purge_deleted_resources(older_than).await
    }
}

// Sync delegates unchanged: a synced note already carries derived metadata from the origin
// device, and `target_note_id`s are global uuids, so no re-derivation is needed. Alias
// uniqueness is best-effort and cannot be enforced against incoming sync. The only local
// concern is the alias index: a sync change that touches a note or notebook invalidates it,
// because what actually got stored depends on conflict resolution inside the inner backend.
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
        // `FsBackend` materializes newly replicated peer notes as a *side effect* of this
        // call (not through `apply_change`), so any note/notebook change reported here
        // must invalidate the index too.
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

#[async_trait]
impl<B: StorageBackend> crate::storage::HistoryRepository for LinkingBackend<B> {
    // History returns the snapshots as they were stored (their links/bookmarks were derived
    // at write time), so this decorator just delegates.
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

    /// A fixed non-Inbox notebook id: Inbox (nil) notes carry no alias and do not
    /// link, so alias/link tests place their notes in a real notebook.
    fn nb() -> Uuid {
        Uuid::from_u128(0x00b0_000c)
    }

    /// A note with `alias` in the notebook `nb()`.
    fn aliased(title: &str, alias: &str) -> Note {
        let mut n = Note::new(title, "");
        n.alias = Some(alias.to_string());
        n.notebook_id = nb();
        n
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
        // Target note with alias "note3", in a real notebook.
        let mut target = Note::new("target", "[Anchor](###) body");
        target.alias = Some("note3".to_string());
        target.notebook_id = nb();
        let target = be.create_note(target).await.unwrap();

        // Source note (also in a real notebook) linking to it by alias.
        let mut src = Note::new("src", "go [here](#note3)");
        src.notebook_id = nb();
        let src = be.create_note(src).await.unwrap();
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
        be.create_note(aliased("a", "dup")).await.unwrap();
        // Same alias in the same notebook conflicts.
        let err = be.create_note(aliased("b", "dup")).await.unwrap_err();
        assert!(matches!(err, StorageError::Conflict(_)));
    }

    #[tokio::test]
    async fn same_alias_in_different_notebooks_is_allowed() {
        let be = backend().await;
        let mut a = aliased("a", "shared");
        a.notebook_id = Uuid::from_u128(1);
        be.create_note(a).await.unwrap();
        // A note with the same alias in a *different* notebook is fine.
        let mut b = aliased("b", "shared");
        b.notebook_id = Uuid::from_u128(2);
        be.create_note(b).await.unwrap();
    }

    #[tokio::test]
    async fn inbox_note_cannot_carry_an_alias() {
        let be = backend().await;
        // An alias set on an Inbox (nil notebook) note is dropped, not stored.
        let mut n = Note::new("n", "");
        n.alias = Some("x".to_string());
        // notebook_id defaults to the nil Inbox.
        let stored = be.create_note(n).await.unwrap();
        assert!(stored.alias.is_none(), "Inbox notes carry no alias");
    }

    #[tokio::test]
    async fn set_note_alias_rejects_inbox_notes() {
        let be = backend().await;
        // The explicit alias endpoint rejects (not silently clears) an Inbox note.
        let inbox_note = be.create_note(Note::new("i", "")).await.unwrap();
        let err = set_note_alias(&be, inbox_note.id, Some("x".into()))
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)), "got: {err}");
        // Clearing an (already absent) alias on an Inbox note is still allowed.
        let cleared = set_note_alias(&be, inbox_note.id, None).await.unwrap();
        assert!(cleared.alias.is_none());
    }

    #[tokio::test]
    async fn moving_a_note_to_inbox_clears_its_alias() {
        let be = backend().await;
        let mut n = be.create_note(aliased("n", "keep")).await.unwrap();
        assert_eq!(n.alias.as_deref(), Some("keep"));
        // Move it to the Inbox: the alias is cleared.
        n.notebook_id = crate::ordering::INBOX_ID;
        let moved = be.update_note(n).await.unwrap();
        assert!(moved.alias.is_none());
    }

    #[tokio::test]
    async fn inbox_note_does_not_link_out() {
        let be = backend().await;
        let target = be.create_note(aliased("t", "tgt")).await.unwrap();
        // A note in the Inbox that references the target does not resolve the link.
        let src = be
            .create_note(Note::new("s", "see [t](#tgt)"))
            .await
            .unwrap();
        assert_eq!(
            src.links[0].target_note_id, None,
            "Inbox notes do not link out"
        );
        // ...and the target gets no backlink from it.
        let (back, _) = backlinks(&be, target.id, 0, None).await.unwrap();
        assert!(back.is_empty());
    }

    #[tokio::test]
    async fn nothing_links_to_an_inbox_note() {
        let be = backend().await;
        // An Inbox target (no alias possible) referenced by uuid does not resolve.
        let inbox_note = be.create_note(Note::new("i", "")).await.unwrap();
        let mut src = Note::new("s", format!("go [x](#{})", inbox_note.id));
        src.notebook_id = nb();
        let src = be.create_note(src).await.unwrap();
        assert_eq!(src.links[0].target_note_id, None);
        let (back, _) = backlinks(&be, inbox_note.id, 0, None).await.unwrap();
        assert!(back.is_empty(), "Inbox notes have no backlinks");
    }

    #[tokio::test]
    async fn bare_alias_resolves_globally_when_unique_else_scoped() {
        let be = backend().await;
        let nb1 = Uuid::from_u128(1);
        let nb2 = Uuid::from_u128(2);
        // Unique alias: a bare `#only` from anywhere resolves to it.
        let mut only = aliased("only", "only");
        only.notebook_id = nb1;
        let only = be.create_note(only).await.unwrap();
        let mut elsewhere = Note::new("e", "go [x](#only)");
        elsewhere.notebook_id = nb2;
        let elsewhere = be.create_note(elsewhere).await.unwrap();
        assert_eq!(elsewhere.links[0].target_note_id, Some(only.id));

        // Now the same alias exists in two notebooks: a bare ref scopes to the
        // source note's own notebook.
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

    #[tokio::test]
    async fn alias_and_link_edits_reject_deleted_entities() {
        let be = backend().await;

        let note = be.create_note(Note::new("n", "")).await.unwrap();
        be.delete_note(note.id).await.unwrap();
        // None of these read-modify-write edits may revive the tombstoned note.
        for err in [
            set_note_alias(&be, note.id, Some("ghost".into()))
                .await
                .unwrap_err(),
            add_manual_link(&be, note.id, "#target").await.unwrap_err(),
            remove_link(&be, note.id, 0).await.unwrap_err(),
        ] {
            assert!(matches!(err, StorageError::NotFound(_)), "got: {err}");
        }
        // Still deleted: the failed edits must not have written anything.
        let read = be.read_note(note.id).await.unwrap();
        assert!(read.deleted_at.is_some(), "note must remain tombstoned");

        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        be.delete_notebook(nb.id).await.unwrap();
        let err = set_notebook_alias(&be, nb.id, Some("ghost".into()))
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)), "got: {err}");
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
        target.notebook_id = nb();
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
        note.notebook_id = nb.id;
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

    /// The alias index must track deletes and renames made through the decorator: a freed
    /// alias becomes claimable again, and a renamed-away alias frees its old name while
    /// occupying the new one.
    #[tokio::test]
    async fn alias_index_tracks_deletes_and_renames() {
        let be = backend().await;

        // Delete frees the alias.
        let a = be.create_note(aliased("a", "freed")).await.unwrap();
        be.delete_note(a.id).await.unwrap();
        be.create_note(aliased("b", "freed")).await.unwrap();

        // Rename frees the old alias and claims the new one.
        let mut c = be.create_note(aliased("c", "old")).await.unwrap();
        c.alias = Some("new".to_string());
        be.update_note(c).await.unwrap();

        be.create_note(aliased("d", "old")).await.unwrap();

        let err = be.create_note(aliased("e", "new")).await.unwrap_err();
        assert!(matches!(err, StorageError::Conflict(_)));
    }

    /// A sync `apply_change` that lands a notebook must invalidate the warm index, so the
    /// next uniqueness check sees the synced alias.
    #[tokio::test]
    async fn sync_applied_change_invalidates_alias_index() {
        let be = backend().await;

        // Warm the index with an alias-bearing write.
        let mut warm = Note::new("warm", "");
        warm.alias = Some("warm".to_string());
        be.create_note(warm).await.unwrap();

        // A notebook with alias "synced" arrives via sync (bypasses uniqueness checks).
        let mut nb = Notebook::new("remote");
        nb.alias = Some("synced".to_string());
        be.apply_change(Change::NotebookCreate { notebook: nb })
            .await
            .unwrap();

        // The decorator must now reject a local notebook claiming that alias.
        let mut local = Notebook::new("local");
        local.alias = Some("synced".to_string());
        let err = be.create_notebook(local).await.unwrap_err();
        assert!(matches!(err, StorageError::Conflict(_)), "got: {err:?}");
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
