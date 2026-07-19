# `linking.rs` — `LinkingBackend` decorator + reference resolution

Self-contained companion for `keeplin-core/src/linking.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be able
to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `LinkingBackend<B>` wraps any `StorageBackend` and, on every
note create/update, rewrites the note's `bookmarks` and `links` from its markdown
body before delegating to `inner`, then enforces that note/notebook `alias`es are
unique. Placement in the stack: **outside** any `EncryptedBackend` (so it parses
the *plaintext* body and resolves aliases against decrypted reads) and **inside**
`EventBackend` (so the live feed carries refreshed metadata):
`EventBackend( LinkingBackend( [EncryptedBackend]( Fs|Db ) ) )`.

What it does on write: (1) **bookmarks** — `[text](### "alias")` declarations
become numbered `Bookmark`s in order of appearance; the body is the single source
of truth (the alias is the link title, defaulting to the text); (2) **links** —
markdown `[t](#…)` destinations become `source = Content` `NoteLink`s, existing
`Manual` links are preserved; (3) **resolution** — each link's `target_note_id` is
filled best-effort (by uuid or by alias through the in-memory index). Notes in the
**Inbox** (`ordering::is_inbox`) are never link targets: a reference naming one —
by alias or raw uuid — resolves to nothing, and those notes carry no alias and
emit no links; (4) **alias uniqueness** — note aliases are unique **per
notebook** (the same alias may live in two notebooks), a colliding create/update
in the *same* notebook is rejected with `StorageError::Conflict`; notebook aliases
are globally unique. A bare `#alias` resolves globally when exactly one live note
carries it; when several notebooks share it, resolution scopes to the referencing
note's own notebook.

Reads, sync (`apply_change`) and other entities delegate unchanged. Cross-device
concurrent edits can still introduce duplicate aliases through sync (which cannot
be rejected); resolution then picks the smallest-uuid match deterministically and
warns.

**The alias index** — uniqueness checks and link resolution only need the
alias → live-entity mapping, so the decorator keeps an in-memory `AliasIndex`
instead of re-scanning the corpus on every alias/link-bearing write (on
`FsBackend` a scan re-materialises every note). Built lazily by one full scan on
the first write that needs it; updated incrementally by every write that flows
through this decorator; **invalidated** (rebuilt on next use) whenever a sync
`apply_change`/`receive_changes` touches a note or notebook — sync outcomes
depend on conflict resolution inside the inner backend, so reflecting them
incrementally would risk drift. Writes that bypass the decorator (a second
process on the same store) are invisible until the next invalidation; the daemon
routes every surface and the sync engine through one shared stack, so within a
daemon the index stays coherent.

**Dependencies** — `crate::links` (the pure grammar), `crate::ordering::is_inbox`,
the storage traits, `tokio::sync`, `serde` (conflict DTOs).

**Used by** — the daemon's stack (`main.rs`); the alias/link/resolve/backlink
endpoints in `rest.rs` and `server.rs` via the free helpers; tests.

**Repeated context** — the decorator pattern and the free-function pattern
coexist here: per-write behaviour lives in trait impls, cross-surface operations
in free functions over `&dyn StorageBackend` whose writes flow back through the
decorator.

---

## SCAN_PAGE

**Identification** — `const SCAN_PAGE: u32 = 500;` marker `// md:SCAN_PAGE`.

**Code** — complete and verbatim:

```rust
// md:SCAN_PAGE
const SCAN_PAGE: u32 = 500;
```

**What it does** — Page size used when scanning every live note/notebook for
resolution and uniqueness.

**Dependencies** — none. **Used by** — `collect_notes`, `collect_notebooks`.
**Repeated context** — none.

---

## ResolvedReference

**Identification** — struct deriving `Debug, Clone, PartialEq, Eq`; marker
`// md:ResolvedReference`.

**Code** — complete and verbatim:

```rust
// md:ResolvedReference
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedReference {
    pub note_id: Uuid,
    pub bookmark_number: Option<u32>,
}
```

**What it does** — A resolved `#…` reference: the concrete `note_id` plus the
1-based `bookmark_number` when the reference had a (resolved) bookmark segment.

**Dependencies** — `uuid`. **Used by** — returned by `resolve`/`resolve_ref`;
the daemon's resolve endpoint. **Repeated context** — none.

---

## AliasConflict

**Identification** — `pub struct AliasConflict<T>` deriving `Debug, Clone,
Serialize`; marker `// md:AliasConflict`.

**Code** — complete and verbatim:

```rust
// md:AliasConflict
#[derive(Debug, Clone, Serialize)]
pub struct AliasConflict<T> {
    pub alias: String,
    pub entities: Vec<T>,
}
```

**What it does** — One alias shared by two or more live entities of the same
type — the residue of a cross-device collision sync could not reject: the
`alias` plus the colliding `entities` ordered by uuid (the smallest is what
resolution prefers).

**Dependencies** — `serde`. **Used by** — `AliasConflicts`; the daemon's
conflicts endpoint. **Repeated context** — none.

---

## AliasConflicts

**Identification** — struct deriving `Debug, Clone, Serialize`; marker
`// md:AliasConflicts`.

**Code** — complete and verbatim:

```rust
// md:AliasConflicts
#[derive(Debug, Clone, Serialize)]
pub struct AliasConflicts {
    pub notes: Vec<AliasConflict<Note>>,
    pub notebooks: Vec<AliasConflict<Notebook>>,
}
```

**What it does** — All current collisions, grouped by entity type (`notes`,
`notebooks`); empty vectors mean none.

**Dependencies** — `AliasConflict`. **Used by** — `alias_conflicts`; the
daemon's `GET /api/aliases/conflicts` / `ListAliasConflicts`.
**Repeated context** — none.

---

## AliasIndex

**Identification** — private `#[derive(Debug, Default)] struct AliasIndex`;
marker `// md:AliasIndex`.

**Code** — complete and verbatim:

```rust
// md:AliasIndex
#[derive(Debug, Default)]
struct AliasIndex {
    note_aliases: BTreeMap<String, BTreeSet<(Uuid, Uuid)>>,
    aliased_notes: HashMap<Uuid, (String, Uuid)>,
    notebook_aliases: BTreeMap<String, BTreeSet<Uuid>>,
    aliased_notebooks: HashMap<Uuid, String>,
}
```

**What it does** — The in-memory alias → live-entity mapping used for
uniqueness checks and reference resolution. Only **live** (non-tombstoned)
aliased entities are indexed, so its size is bounded by the number of aliases,
not the corpus. Fields: `note_aliases: BTreeMap<String, BTreeSet<(Uuid, Uuid)>>`
(alias → live notes as `(note_id, notebook_id)` so scoped resolution works
without the full note; ordered sets make the smallest-uuid tiebreak
deterministic), `aliased_notes: HashMap<Uuid, (String, Uuid)>` (reverse map so
an edit can remove the old entry), `notebook_aliases` / `aliased_notebooks`
(the notebook equivalents).

**Dependencies** — `BTreeMap`/`BTreeSet`/`HashMap`, `uuid`.

**Used by** — `LinkingBackend` (behind an `RwLock<Option<…>>`), `resolve_ref`
(builds a throwaway one from snapshots).

**Repeated context** — none.

---

## impl AliasIndex

**Identification** — inherent impl; marker `// md:impl AliasIndex`. Ten
methods.

**Code** — container: members documented as sub-blocks below: fn from_snapshots, fn upsert_note, fn remove_note, fn upsert_notebook, fn remove_notebook, fn note_alias_taken, fn notebook_alias_taken, fn resolve_notebook_seg, fn resolve_note_seg, fn resolve_target.

### fn from_snapshots

**Identification** — `fn from_snapshots(notes: &[Note], notebooks: &[Notebook]) -> Self`;
marker `// md:impl AliasIndex > fn from_snapshots`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Builds the index by upserting every snapshot entity.

**Used by** — `with_index`'s lazy build; `resolve_ref`.

### fn upsert_note

**Identification** — `fn upsert_note(&mut self, note: &Note)`; marker
`// md:impl AliasIndex > fn upsert_note`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Reflects a note's current state: drop any previous entry,
then (re-)insert only when the note is live, **not in the Inbox**, and carries
an alias. Inbox notes are never indexed — the "nothing links to an Inbox note"
guarantee on the raw-uuid path is enforced by the callers instead (a backend
read in `prepare`, the snapshot check in `resolve_ref`), keeping the index
bounded by the alias count rather than the Inbox size.

**Dependencies** — `remove_note`, `is_inbox`.

**Used by** — `from_snapshots`, `index_upsert_note`.

### fn remove_note

**Identification** — `fn remove_note(&mut self, id: Uuid)`; marker
`// md:impl AliasIndex > fn remove_note`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Drops a note's entry via the reverse map, pruning
now-empty alias sets. Used for deletes and as the first half of an upsert.

### fn upsert_notebook

**Identification** — marker `// md:impl AliasIndex > fn upsert_notebook`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Notebook twin of `upsert_note` (no Inbox special case —
the Inbox notebook itself simply carries no alias).

### fn remove_notebook

**Identification** — marker `// md:impl AliasIndex > fn remove_notebook`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Drops a notebook's entry, pruning empty sets.

### fn note_alias_taken

**Identification** —
`fn note_alias_taken(&self, alias: &str, self_id: Uuid, notebook_id: Uuid) -> bool`;
marker `// md:impl AliasIndex > fn note_alias_taken`.

**Code** — complete and verbatim:

```rust
    // md:impl AliasIndex > fn note_alias_taken
    fn note_alias_taken(&self, alias: &str, self_id: Uuid, notebook_id: Uuid) -> bool {
        self.note_aliases.get(alias).is_some_and(|set| {
            set.iter()
                .any(|(id, nb)| *id != self_id && *nb == notebook_id)
        })
    }
```

**What it does** — Whether `alias` is carried by another live note **in the
same notebook** (`self_id` excluded) — per-notebook uniqueness, not global.

**Used by** — `prepare`.

### fn notebook_alias_taken

**Identification** — marker `// md:impl AliasIndex > fn notebook_alias_taken`.

**Code** — complete and verbatim:

```rust
    // md:impl AliasIndex > fn notebook_alias_taken
    fn notebook_alias_taken(&self, alias: &str, self_id: Uuid) -> bool {
        self.notebook_aliases
            .get(alias)
            .is_some_and(|set| set.iter().any(|id| *id != self_id))
    }
```

**What it does** — Whether `alias` is carried by a live notebook other than
`self_id` (global uniqueness for notebooks).

**Used by** — `ensure_notebook_alias_free`.

### fn resolve_notebook_seg

**Identification** — `fn resolve_notebook_seg(&self, seg: &str) -> Option<Uuid>`;
marker `// md:impl AliasIndex > fn resolve_notebook_seg`.

**Code** — complete and verbatim:

```rust
    // md:impl AliasIndex > fn resolve_notebook_seg
    fn resolve_notebook_seg(&self, seg: &str) -> Option<Uuid> {
        if let Ok(id) = Uuid::parse_str(seg) {
            return Some(id);
        }
        self.notebook_aliases
            .get(seg)
            .and_then(|set| set.iter().next().copied())
    }
```

**What it does** — A uuid segment is returned as-is (existence unchecked); an
alias picks the smallest-uuid live match.

**Used by** — `resolve_note_seg`.

### fn resolve_note_seg

**Identification** —
`fn resolve_note_seg(&self, seg, notebook_seg: Option<&str>, source_notebook: Option<Uuid>) -> Option<Uuid>`;
marker `// md:impl AliasIndex > fn resolve_note_seg`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Resolves a note segment: a uuid as-is (the Inbox exclusion
for raw uuids is enforced by callers, not here); an alias filters candidates to
non-Inbox notes, then — explicit `notebook_seg` → only that notebook (and the
Inbox is never a valid scope); bare alias with a single candidate → global
resolution; multiple candidates + `source_notebook` → scoped to it; no source
context → deterministic smallest-uuid with a warning. `None` (no eligible
match) drives the two-segment fallback.

**Used by** — `resolve_target`.

**Repeated context** — smallest-uuid on ambiguity is the deterministic answer
to sync-introduced duplicates.

### fn resolve_target

**Identification** —
`fn resolve_target<'a>(&self, raw: &'a str, source_notebook: Option<Uuid>) -> Option<(Uuid, Option<&'a str>)>`;
marker `// md:impl AliasIndex > fn resolve_target`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Resolves a raw `#…` reference to its target note id plus
the still-unresolved bookmark segment — the segment logic shared by write-time
link resolution (which only needs the note id) and `resolve_ref` (which also
maps the bookmark to a number). Interpretation: `#note`; `#a#b` preferred as
`notebook#note`, falling back to `note#bookmark` when the second segment is not
a resolvable note; `#notebook#note#bookmark`. Empty segments or >3 segments →
`None`.

**Used by** — `prepare`, `resolve_ref`.

---

## fn change_affects_aliases

**Identification** — `fn change_affects_aliases(change: &Change) -> bool`;
marker `// md:fn change_affects_aliases`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Whether a sync change can move an alias (or a note between
notebooks): any note or notebook create/update/delete.

**Dependencies** — `Change`. **Used by** — `apply_change`, `receive_changes`.
**Repeated context** — none.

---

## LinkingBackend

**Identification** — `pub struct LinkingBackend<B>`; marker
`// md:LinkingBackend`.

**Code** — complete and verbatim:

```rust
// md:LinkingBackend
pub struct LinkingBackend<B> {
    inner: B,
    alias_write_lock: Arc<Mutex<()>>,
    alias_index: Arc<RwLock<Option<AliasIndex>>>,
}
```

**What it does** — The decorator: `inner`, `alias_write_lock: Arc<Mutex<()>>`
(serialises alias-bearing writes so "check for duplicate, then write" is
atomic — without it two concurrent writes claiming the same alias could both
pass the check; only taken when the entity actually carries an alias, so plain
notes never serialise), `alias_index: Arc<RwLock<Option<AliasIndex>>>` (`None`
= unbuilt or invalidated; the next alias/link-bearing write rebuilds with one
corpus scan).

**Dependencies** — `AliasIndex`, tokio sync.

**Used by** — the daemon's stack assembly; tests.

**Repeated context** — none.

---

## impl LinkingBackend

**Identification** — inherent impl `impl<B: StorageBackend> LinkingBackend<B>`;
marker `// md:impl LinkingBackend`. Ten methods.

**Code** — container: members documented as sub-blocks below: fn new, fn with_index, fn index_upsert_note, fn index_remove_note, fn index_upsert_notebook, fn index_remove_notebook, fn index_invalidate, fn refresh, fn prepare, fn ensure_notebook_alias_free.

### fn new

**Identification** — `pub fn new(inner: B) -> Self`; marker
`// md:impl LinkingBackend > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl LinkingBackend > fn new
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            alias_write_lock: Arc::new(Mutex::new(())),
            alias_index: Arc::new(RwLock::new(None)),
        }
    }
```

**What it does** — Wraps `inner` with a fresh lock and an unbuilt index.

### fn with_index

**Identification** —
`async fn with_index<R>(&self, f: impl FnOnce(&AliasIndex) -> R) -> Result<R, StorageError>`;
marker `// md:impl LinkingBackend > fn with_index`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Runs `f` against the index, building it first (one corpus
scan via `collect_notes`/`collect_notebooks`) when absent or invalidated.
Double-checked locking: read-lock fast path, then write-lock rebuild — at most
one concurrent build.

**Used by** — `prepare`, `ensure_notebook_alias_free`.

### fn index_upsert_note

**Identification** — marker `// md:impl LinkingBackend > fn index_upsert_note`.

**Code** — complete and verbatim:

```rust
    // md:impl LinkingBackend > fn index_upsert_note
    async fn index_upsert_note(&self, note: &Note) {
        if let Some(idx) = self.alias_index.write().await.as_mut() {
            idx.upsert_note(note);
        }
    }
```

**What it does** — Folds a successfully written note into the index; a no-op
while unbuilt (the next build scans the store, which already reflects the
write).

### fn index_remove_note

**Identification** — marker `// md:impl LinkingBackend > fn index_remove_note`.

**Code** — complete and verbatim:

```rust
    // md:impl LinkingBackend > fn index_remove_note
    async fn index_remove_note(&self, id: Uuid) {
        if let Some(idx) = self.alias_index.write().await.as_mut() {
            idx.remove_note(id);
        }
    }
```

**What it does** — Drops a deleted note from the index (if built).

### fn index_upsert_notebook

**Identification** — marker
`// md:impl LinkingBackend > fn index_upsert_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl LinkingBackend > fn index_upsert_notebook
    async fn index_upsert_notebook(&self, notebook: &Notebook) {
        if let Some(idx) = self.alias_index.write().await.as_mut() {
            idx.upsert_notebook(notebook);
        }
    }
```

**What it does** — Folds a successfully written notebook into the index.

### fn index_remove_notebook

**Identification** — marker
`// md:impl LinkingBackend > fn index_remove_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl LinkingBackend > fn index_remove_notebook
    async fn index_remove_notebook(&self, id: Uuid) {
        if let Some(idx) = self.alias_index.write().await.as_mut() {
            idx.remove_notebook(id);
        }
    }
```

**What it does** — Drops a deleted notebook from the index.

### fn index_invalidate

**Identification** — marker `// md:impl LinkingBackend > fn index_invalidate`.

**Code** — complete and verbatim:

```rust
    // md:impl LinkingBackend > fn index_invalidate
    async fn index_invalidate(&self) {
        *self.alias_index.write().await = None;
    }
```

**What it does** — Discards the index so the next use rebuilds from the
store — called when a sync change lands, because what actually got stored
depends on conflict resolution inside the inner backend.

### fn refresh

**Identification** — `fn refresh(note: &mut Note)` (associated, pure); marker
`// md:impl LinkingBackend > fn refresh`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Rewrites `note.bookmarks` (each `[text](### "alias")`
declaration → a numbered `Bookmark`, alias = title, defaulting to the text when
omitted or empty) and `note.links` (keep `Manual`, re-derive `Content` from
`links::parse_content_links`, validating each raw with `NoteLink::from_raw`).
No I/O.

**Dependencies** — `links::parse_bookmarks`/`parse_content_links`,
`NoteLink::from_raw`.

**Used by** — `prepare`.

### fn prepare

**Identification** — `async fn prepare(&self, note: &mut Note) -> Result<(), StorageError>`;
marker `// md:impl LinkingBackend > fn prepare`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Prepares a note for create/update: `refresh`, then the
Inbox short-circuit — an Inbox note gets `alias = None` and every
`target_note_id = None` (Inbox notes carry no alias and do not link out; moving
a note into the Inbox clears its alias) — then, only when the note has an alias
or links: one `with_index` pass computing (a) whether the alias is taken in
this notebook (→ `Conflict`), (b) each link's best-effort target, (c) the
targets the index does not know as aliased notes (resolved through the raw-uuid
path — they could name an Inbox note, so each is **verified with one backend
read**; alias-resolved targets are always indexed, hence never Inbox). Links
into a live Inbox note get `target_note_id = None`; a uuid that reads as
missing or deleted keeps resolving as-is. The common case (no alias, no links)
never touches the index.

**Dependencies** — `refresh`, `with_index`, `is_inbox`, `read_note`.

**Used by** — `create_note`, `update_note`.

**Repeated context** — `Conflict` maps to HTTP 409 / gRPC `ALREADY_EXISTS` in
the daemon.

### fn ensure_notebook_alias_free

**Identification** —
`async fn ensure_notebook_alias_free(&self, notebook: &Notebook) -> Result<(), StorageError>`;
marker `// md:impl LinkingBackend > fn ensure_notebook_alias_free`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Rejects a notebook whose alias collides with another live
notebook (`Conflict`); an alias-less notebook passes immediately, no index
needed.

**Used by** — `create_notebook`, `update_notebook`.

---

## fn collect_notes

**Identification** — `pub async fn collect_notes(backend: &dyn StorageBackend) -> Result<Vec<Note>, StorageError>`;
marker `// md:fn collect_notes`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Every live note, by exhausting the paginated `list_notes`
at `SCAN_PAGE`. (The free helpers exist because the decorator sits behind
`Arc<dyn StorageBackend>`, so surfaces cannot call inherent methods; their
writes flow back through the decorator, so derivation, resolution and
uniqueness still apply.)

**Used by** — `with_index` build, `resolve`, `alias_conflicts`; the daemon.

---

## fn collect_notebooks

**Identification** — marker `// md:fn collect_notebooks`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Every live notebook, same pattern.

**Used by** — `with_index` build, `resolve`, `alias_conflicts`.

---

## fn resolve_bookmark_seg

**Identification** —
`fn resolve_bookmark_seg(seg: &str, note_id: Uuid, notes: &[Note]) -> Option<u32>`;
marker `// md:fn resolve_bookmark_seg`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Maps a bookmark segment (number or alias, via
`links::BookmarkRef::parse`) to a stored bookmark number within `note_id`,
using the note found in the `notes` snapshot; `None` when the note is not in
the snapshot or has no matching bookmark.

**Used by** — `resolve_ref`.

---

## fn resolve_ref

**Identification** —
`fn resolve_ref(raw: &str, notes: &[Note], notebooks: &[Notebook]) -> Option<ResolvedReference>`;
marker `// md:fn resolve_ref`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Pure resolution against snapshots: builds a throwaway
`AliasIndex`, runs `resolve_target` (no source note, so a bare ambiguous alias
falls back deterministically), rejects a uuid-resolved **Inbox** target via the
snapshot (alias references already exclude the Inbox inside `resolve_target`),
then maps any bookmark segment to its stored number.

**Used by** — `resolve`.

---

## fn resolve

**Identification** —
`pub async fn resolve(backend: &dyn StorageBackend, raw: &str) -> Result<Option<ResolvedReference>, StorageError>`;
marker `// md:fn resolve`.

**Code** — complete and verbatim:

```rust
// md:fn resolve
pub async fn resolve(
    backend: &dyn StorageBackend,
    raw: &str,
) -> Result<Option<ResolvedReference>, StorageError> {
    let notes = collect_notes(backend).await?;
    let notebooks = collect_notebooks(backend).await?;
    Ok(resolve_ref(raw, &notes, &notebooks))
}
```

**What it does** — Collects live notes + notebooks and runs `resolve_ref` —
the store-backed resolution the daemon's resolve endpoint uses.

**Used by** — `rest.rs`/`server.rs` resolve endpoints; tests.

---

## fn backlinks

**Identification** —
`pub async fn backlinks(backend, target_id, page_size, page_token) -> Result<(Vec<Note>, Option<String>), StorageError>`;
marker `// md:fn backlinks`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A page of live notes linking to `target_id`, delegating to
`NoteRepository::note_backlinks` (indexed on `DbBackend`, `O(N)` in-memory
pagination elsewhere).

**Used by** — the daemon's backlinks endpoints.

---

## fn group_conflicts

**Identification** —
`fn group_conflicts<T>(items, alias_of, id_of) -> Vec<AliasConflict<T>>`; marker
`// md:fn group_conflicts`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Groups items by their optional alias, keeping only aliases
shared by ≥ 2 entities; groups ordered by alias, entities by uuid.

**Used by** — `alias_conflicts` (notebook side).

---

## fn alias_conflicts

**Identification** —
`pub async fn alias_conflicts(backend: &dyn StorageBackend) -> Result<AliasConflicts, StorageError>`;
marker `// md:fn alias_conflicts`.

**Code** — complete and verbatim:

```rust
// md:fn alias_conflicts
pub async fn alias_conflicts(backend: &dyn StorageBackend) -> Result<AliasConflicts, StorageError> {
    let notes = collect_notes(backend).await?;
    let notebooks = collect_notebooks(backend).await?;
    Ok(AliasConflicts {
        notes: group_note_conflicts(notes),
        notebooks: group_conflicts(notebooks, |nb| nb.alias.clone(), |nb| nb.id),
    })
}
```

**What it does** — Lists every alias currently shared by two or more live
notes (per-notebook grouping) or notebooks (global). Local writes reject
duplicates, but sync replays independent edits, so collisions can appear after
a sync; this surfaces them for a human to rename — resolution stays
deterministic meanwhile.

**Used by** — `GET /api/aliases/conflicts` / `ListAliasConflicts`.

---

## fn group_note_conflicts

**Identification** — `fn group_note_conflicts(notes: Vec<Note>) -> Vec<AliasConflict<Note>>`;
marker `// md:fn group_note_conflicts`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Groups note conflicts by `(alias, notebook)` — the same
alias in two different notebooks is *not* a conflict (per-notebook
uniqueness). Inbox notes are skipped (they carry no alias). Entities ordered
by uuid.

**Used by** — `alias_conflicts`.

---

## fn read_live_note

**Identification** — `async fn read_live_note(backend, id) -> Result<Note, StorageError>`;
marker `// md:fn read_live_note`.

**Code** — complete and verbatim:

```rust
// md:fn read_live_note
async fn read_live_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    let note = backend.read_note(id).await?;
    if note.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()));
    }
    Ok(note)
}
```

**What it does** — Reads a note for a user-facing read-modify-write, rejecting
a tombstone as `NotFound`. Without this, the RMW would silently **revive** a
deleted note (writing `deleted_at: None` back). Revival is reserved for the
sync path (`apply_change` resolving a causal edit made after the delete),
never a side effect of an alias or link edit.

**Used by** — `set_note_alias`, `add_manual_link`, `remove_link`.

---

## fn read_live_notebook

**Identification** — marker `// md:fn read_live_notebook`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Notebook twin of `read_live_note`.

**Used by** — `set_notebook_alias`.

---

## fn set_note_alias

**Identification** —
`pub async fn set_note_alias(backend, note_id, alias: Option<String>) -> Result<Note, StorageError>`;
marker `// md:fn set_note_alias`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Sets or clears a note's alias (RMW → one `NoteUpdate`). A
soft-deleted note is `NotFound`; **setting** an alias on an Inbox note is
`InvalidInput` (Inbox notes carry no alias — rejected rather than silently
cleared, matching the other Inbox domain rules such as pinning); clearing an
already-null alias is a no-op that succeeds.

**Used by** — the daemon's alias endpoints.

---

## fn set_notebook_alias

**Identification** — marker `// md:fn set_notebook_alias`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Sets or clears a notebook's alias; a soft-deleted notebook
is `NotFound`.

**Used by** — the daemon's alias endpoints.

---

## fn add_manual_link

**Identification** —
`pub async fn add_manual_link(backend, note_id, raw: &str) -> Result<Note, StorageError>`;
marker `// md:fn add_manual_link`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Adds a `Manual` link from `note_id` to a raw `#…` reference
(validated via `NoteLink::from_raw`; an invalid reference is
`InvalidState`) and persists through `update_note` (so `prepare` resolves it).
A soft-deleted note is `NotFound`.

**Used by** — the daemon's link endpoints.

---

## fn remove_link

**Identification** —
`pub async fn remove_link(backend, note_id, index: usize) -> Result<Note, StorageError>`;
marker `// md:fn remove_link`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Removes the link at `index` into the note's `links`
(`NotFound` when out of range) and persists. A soft-deleted note is
`NotFound`.

**Used by** — the daemon's link endpoints.

---

## impl NoteRepository for LinkingBackend

**Identification** — marker `// md:impl NoteRepository for LinkingBackend`
(one marker for the impl block).

**Code** — complete and verbatim:

```rust
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
```

**What it does** —

- `create_note` / `update_note` — take the `alias_write_lock` **only when the
  note carries an alias** (holding it across the uniqueness check + write),
  run `prepare`, delegate, then `index_upsert_note` the stored copy (upsert
  even without an alias: the edit may be *removing* one).
- `delete_note` — delegate, then `index_remove_note`.
- `note_backlinks` — an Inbox target short-circuits to an empty page (Inbox
  notes are never link targets; this also hides stale targets written before
  the rule); otherwise delegate so an inner indexed backend is reached.
- `read_note`, `list_notes`, `list_notes_in_notebook`, `list_starred_notes`,
  `notebook_sort_profile` — pure delegation.

**Dependencies** — `prepare`, the index helpers, `is_inbox`, the inner
backend.

**Used by** — all note traffic.

**Repeated context** — none.

---

## impl NotebookRepository for LinkingBackend

**Identification** — marker
`// md:impl NotebookRepository for LinkingBackend`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `create_notebook`/`update_notebook` take the alias lock
when an alias is present, run `ensure_notebook_alias_free`, delegate, and
upsert the index (again: an update may be removing an alias);
`delete_notebook` delegates then removes from the index;
`read_notebook`/`list_notebooks` delegate.

**Dependencies** — `ensure_notebook_alias_free`, the index helpers.

**Used by** — notebook traffic.

**Repeated context** — none.

---

## impl TagRepository for LinkingBackend

**Identification** — marker `// md:impl TagRepository for LinkingBackend`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Pure delegation for all eight methods — tags carry no
aliases and no links.

**Used by** — tag traffic. **Dependencies** — the inner backend.
**Repeated context** — none.

---

## impl ResourceRepository for LinkingBackend

**Identification** — marker
`// md:impl ResourceRepository for LinkingBackend`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Pure delegation for all five methods.

**Used by** — resource traffic. **Dependencies** — the inner backend.
**Repeated context** — none.

---

## impl SyncBackend for LinkingBackend

**Identification** — marker `// md:impl SyncBackend for LinkingBackend`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Sync delegates unchanged: a synced note already carries
derived metadata from the origin device, and `target_note_id`s are global
uuids, so no re-derivation is needed; alias uniqueness is best-effort and
cannot be enforced against incoming sync. The only local concern is the alias
index:

- `apply_change` — delegate, then `index_invalidate` when the change touches
  a note or notebook (`change_affects_aliases`).
- `receive_changes` — delegate, then invalidate if any reported change
  affects aliases: `FsBackend` materialises newly replicated peer notes as a
  *side effect* of this call (not through `apply_change`).
- the rest — pure delegation.

**Dependencies** — `change_affects_aliases`, `index_invalidate`, the inner
backend.

**Used by** — the sync cycle.

**Repeated context** — none.

---

## impl HistoryRepository for LinkingBackend

**Identification** — marker `// md:impl HistoryRepository for LinkingBackend`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Pure delegation: history returns snapshots as stored (their
links/bookmarks were derived at write time).

**Used by** — history endpoints. **Dependencies** — the inner backend.
**Repeated context** — none.

---

## mod tests

**Identification** — `#[cfg(test)]` test module; marker `// md:mod tests`.
Three helpers + seventeen tests over `LinkingBackend<FsBackend>` in leaked
tempdirs.

**Code** — container: members documented as sub-blocks below: fn backend, fn nb, fn aliased, fn derives_bookmarks_and_content_links, fn bookmark_alias_comes_from_the_body_title, fn resolves_link_by_alias_and_uuid, fn rejects_duplicate_note_alias, fn same_alias_in_different_notebooks_is_allowed, fn inbox_note_cannot_carry_an_alias, fn set_note_alias_rejects_inbox_notes, fn moving_a_note_to_inbox_clears_its_alias, fn inbox_note_does_not_link_out, fn nothing_links_to_an_inbox_note, fn bare_alias_resolves_globally_when_unique_else_scoped, fn alias_and_link_edits_reject_deleted_entities, fn add_and_remove_manual_link, fn resolves_two_segment_note_bookmark_shorthand, fn two_segment_prefers_notebook_note, fn alias_conflicts_lists_duplicates, fn alias_index_tracks_deletes_and_renames, fn sync_applied_change_invalidates_alias_index, fn concurrent_duplicate_alias_yields_exactly_one_winner.

**What it does** — Covers derivation, resolution (alias/uuid/scoping/
two-segment fallback), per-notebook uniqueness, every Inbox rule, tombstone
protection, manual links, conflict listing, index maintenance and
invalidation, and the concurrent-alias race.

**Dependencies** — `super::*`, `storage::fs::FsBackend`, `tempfile`, `tokio`.

**Used by** — CI.

**Repeated context** — none.

### fn backend

**Identification** — helper `async fn backend() -> LinkingBackend<FsBackend>`;
marker `// md:mod tests > fn backend`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn backend
    async fn backend() -> LinkingBackend<FsBackend> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        LinkingBackend::new(FsBackend::new(&path).await.unwrap())
    }
```

**What it does** — A decorator over `FsBackend` in a leaked tempdir.

### fn nb

**Identification** — helper `fn nb() -> Uuid`; marker
`// md:mod tests > fn nb`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn nb
    fn nb() -> Uuid {
        Uuid::from_u128(0x00b0_000c)
    }
```

**What it does** — A fixed non-Inbox notebook id — alias/link tests must place
notes in a real notebook (Inbox notes carry no alias and do not link).

### fn aliased

**Identification** — helper `fn aliased(title, alias) -> Note`; marker
`// md:mod tests > fn aliased`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn aliased
    fn aliased(title: &str, alias: &str) -> Note {
        let mut n = Note::new(title, "");
        n.alias = Some(alias.to_string());
        n.notebook_id = nb();
        n
    }
```

**What it does** — A note with `alias` in the notebook `nb()`.

### fn derives_bookmarks_and_content_links

**Identification** — tokio test; marker
`// md:mod tests > fn derives_bookmarks_and_content_links`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A body with two bookmark declarations (one titled) and a
content link stores two numbered bookmarks (alias defaults to the text; a
title becomes the alias) and one `Content` link with the raw reference.

### fn bookmark_alias_comes_from_the_body_title

**Identification** — tokio test; marker
`// md:mod tests > fn bookmark_alias_comes_from_the_body_title`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The alias lives in the body: editing the link title renames
the bookmark alias.

### fn resolves_link_by_alias_and_uuid

**Identification** — tokio test; marker
`// md:mod tests > fn resolves_link_by_alias_and_uuid`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — An alias link resolves to the target's id; a three-segment
uuid ref resolves note + bookmark number; backlinks list the source.

### fn rejects_duplicate_note_alias

**Identification** — tokio test; marker
`// md:mod tests > fn rejects_duplicate_note_alias`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn rejects_duplicate_note_alias
    #[tokio::test]
    async fn rejects_duplicate_note_alias() {
        let be = backend().await;
        be.create_note(aliased("a", "dup")).await.unwrap();
        let err = be.create_note(aliased("b", "dup")).await.unwrap_err();
        assert!(matches!(err, StorageError::Conflict(_)));
    }
```

**What it does** — Same alias, same notebook → `Conflict`.

### fn same_alias_in_different_notebooks_is_allowed

**Identification** — tokio test; marker
`// md:mod tests > fn same_alias_in_different_notebooks_is_allowed`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Same alias in two different notebooks is fine.

### fn inbox_note_cannot_carry_an_alias

**Identification** — tokio test; marker
`// md:mod tests > fn inbox_note_cannot_carry_an_alias`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn inbox_note_cannot_carry_an_alias
    #[tokio::test]
    async fn inbox_note_cannot_carry_an_alias() {
        let be = backend().await;
        let mut n = Note::new("n", "");
        n.alias = Some("x".to_string());
        let stored = be.create_note(n).await.unwrap();
        assert!(stored.alias.is_none(), "Inbox notes carry no alias");
    }
```

**What it does** — An alias set on an Inbox create is dropped, not stored.

### fn set_note_alias_rejects_inbox_notes

**Identification** — tokio test; marker
`// md:mod tests > fn set_note_alias_rejects_inbox_notes`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The explicit endpoint rejects (`InvalidInput`) setting an
alias on an Inbox note; clearing an absent alias still succeeds.

### fn moving_a_note_to_inbox_clears_its_alias

**Identification** — tokio test; marker
`// md:mod tests > fn moving_a_note_to_inbox_clears_its_alias`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Moving an aliased note into the Inbox clears the alias.

### fn inbox_note_does_not_link_out

**Identification** — tokio test; marker
`// md:mod tests > fn inbox_note_does_not_link_out`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — An Inbox note referencing a target resolves no link and
produces no backlink.

### fn nothing_links_to_an_inbox_note

**Identification** — tokio test; marker
`// md:mod tests > fn nothing_links_to_an_inbox_note`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A uuid reference to an Inbox note does not resolve, and the
Inbox note has no backlinks.

### fn bare_alias_resolves_globally_when_unique_else_scoped

**Identification** — tokio test; marker
`// md:mod tests > fn bare_alias_resolves_globally_when_unique_else_scoped`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A unique bare alias resolves from any notebook; once the
alias exists in two notebooks, a bare ref scopes to the source note's own
notebook.

### fn alias_and_link_edits_reject_deleted_entities

**Identification** — tokio test; marker
`// md:mod tests > fn alias_and_link_edits_reject_deleted_entities`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `set_note_alias`/`add_manual_link`/`remove_link` on a
tombstoned note are all `NotFound` and write nothing (the note stays
tombstoned); `set_notebook_alias` likewise.

### fn add_and_remove_manual_link

**Identification** — tokio test; marker
`// md:mod tests > fn add_and_remove_manual_link`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A manual link is added (`source = Manual`) and removed by
index.

### fn resolves_two_segment_note_bookmark_shorthand

**Identification** — tokio test; marker
`// md:mod tests > fn resolves_two_segment_note_bookmark_shorthand`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `#note#bookmark` resolves by bookmark alias and by number.

### fn two_segment_prefers_notebook_note

**Identification** — tokio test; marker
`// md:mod tests > fn two_segment_prefers_notebook_note`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — `#notebook#note` resolves to the note when the second
segment is a resolvable note (not reinterpreted as `note#bookmark`).

### fn alias_conflicts_lists_duplicates

**Identification** — tokio test; marker
`// md:mod tests > fn alias_conflicts_lists_duplicates`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Duplicates planted through a raw `FsBackend` (the way sync
would, bypassing the write-time check) are listed by `alias_conflicts`, with
unique aliases excluded.

### fn alias_index_tracks_deletes_and_renames

**Identification** — tokio test; marker
`// md:mod tests > fn alias_index_tracks_deletes_and_renames`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — A delete frees the alias for reuse; a rename frees the old
name and occupies the new one (claiming the new name again is `Conflict`).

### fn sync_applied_change_invalidates_alias_index

**Identification** — tokio test; marker
`// md:mod tests > fn sync_applied_change_invalidates_alias_index`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — After warming the index, a notebook alias arriving via
`apply_change` (bypassing uniqueness) must still cause a later local claim of
that alias to be rejected — proving the invalidation.

### fn concurrent_duplicate_alias_yields_exactly_one_winner

**Identification** — multi-thread tokio test; marker
`// md:mod tests > fn concurrent_duplicate_alias_yields_exactly_one_winner`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Eight concurrent creates claiming the same alias: exactly
one succeeds, seven get `Conflict` — the `alias_write_lock` closes the
check-then-write race.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `LinkingBackend<B>` — defined here (EXTRACTED; high cross-file degree)
- `AliasIndex`, `AliasConflict`, `AliasConflicts`, `ResolvedReference` — defined here (EXTRACTED)
- `collect_notes()`, `collect_notebooks()`, `resolve()`, `backlinks()`, `alias_conflicts()`, `set_note_alias()`, `set_notebook_alias()`, `add_manual_link()`, `remove_link()` — defined here (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs` — error types (EXTRACTED: references×51; e.g. `StorageError`)
- `keeplin-core/src/links.rs` — bookmark & link types and pure parsing (EXTRACTED)
- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: references×52)
- `keeplin-core/src/ordering.rs` — the Inbox, pinning, ordering, starring (EXTRACTED: calls×7; e.g. `is_inbox`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: implements×6, references×16)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/main.rs` — stack assembly (INFERRED)
- `keeplin-daemon/src/rest.rs` / `server.rs` — alias/link/resolve/backlink/conflict endpoints via the free helpers (INFERRED: fully-qualified paths the AST pass does not link)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `SCAN_PAGE` | `// md:SCAN_PAGE` |
| 3 | `ResolvedReference` | `// md:ResolvedReference` |
| 4 | `AliasConflict` | `// md:AliasConflict` |
| 5 | `AliasConflicts` | `// md:AliasConflicts` |
| 6 | `AliasIndex` | `// md:AliasIndex` |
| 7 | `impl AliasIndex` (container) | `// md:impl AliasIndex` |
| 8 | `fn from_snapshots` | `// md:impl AliasIndex > fn from_snapshots` |
| 9 | `fn upsert_note` | `// md:impl AliasIndex > fn upsert_note` |
| 10 | `fn remove_note` | `// md:impl AliasIndex > fn remove_note` |
| 11 | `fn upsert_notebook` | `// md:impl AliasIndex > fn upsert_notebook` |
| 12 | `fn remove_notebook` | `// md:impl AliasIndex > fn remove_notebook` |
| 13 | `fn note_alias_taken` | `// md:impl AliasIndex > fn note_alias_taken` |
| 14 | `fn notebook_alias_taken` | `// md:impl AliasIndex > fn notebook_alias_taken` |
| 15 | `fn resolve_notebook_seg` | `// md:impl AliasIndex > fn resolve_notebook_seg` |
| 16 | `fn resolve_note_seg` | `// md:impl AliasIndex > fn resolve_note_seg` |
| 17 | `fn resolve_target` | `// md:impl AliasIndex > fn resolve_target` |
| 18 | `fn change_affects_aliases` | `// md:fn change_affects_aliases` |
| 19 | `LinkingBackend` | `// md:LinkingBackend` |
| 20 | `impl LinkingBackend` (container) | `// md:impl LinkingBackend` |
| 21 | `fn new` | `// md:impl LinkingBackend > fn new` |
| 22 | `fn with_index` | `// md:impl LinkingBackend > fn with_index` |
| 23 | `fn index_upsert_note` | `// md:impl LinkingBackend > fn index_upsert_note` |
| 24 | `fn index_remove_note` | `// md:impl LinkingBackend > fn index_remove_note` |
| 25 | `fn index_upsert_notebook` | `// md:impl LinkingBackend > fn index_upsert_notebook` |
| 26 | `fn index_remove_notebook` | `// md:impl LinkingBackend > fn index_remove_notebook` |
| 27 | `fn index_invalidate` | `// md:impl LinkingBackend > fn index_invalidate` |
| 28 | `fn refresh` | `// md:impl LinkingBackend > fn refresh` |
| 29 | `fn prepare` | `// md:impl LinkingBackend > fn prepare` |
| 30 | `fn ensure_notebook_alias_free` | `// md:impl LinkingBackend > fn ensure_notebook_alias_free` |
| 31 | `fn collect_notes` | `// md:fn collect_notes` |
| 32 | `fn collect_notebooks` | `// md:fn collect_notebooks` |
| 33 | `fn resolve_bookmark_seg` | `// md:fn resolve_bookmark_seg` |
| 34 | `fn resolve_ref` | `// md:fn resolve_ref` |
| 35 | `fn resolve` | `// md:fn resolve` |
| 36 | `fn backlinks` | `// md:fn backlinks` |
| 37 | `fn group_conflicts` | `// md:fn group_conflicts` |
| 38 | `fn alias_conflicts` | `// md:fn alias_conflicts` |
| 39 | `fn group_note_conflicts` | `// md:fn group_note_conflicts` |
| 40 | `fn read_live_note` | `// md:fn read_live_note` |
| 41 | `fn read_live_notebook` | `// md:fn read_live_notebook` |
| 42 | `fn set_note_alias` | `// md:fn set_note_alias` |
| 43 | `fn set_notebook_alias` | `// md:fn set_notebook_alias` |
| 44 | `fn add_manual_link` | `// md:fn add_manual_link` |
| 45 | `fn remove_link` | `// md:fn remove_link` |
| 46 | `impl NoteRepository for LinkingBackend` | `// md:impl NoteRepository for LinkingBackend` |
| 47 | `impl NotebookRepository for LinkingBackend` | `// md:impl NotebookRepository for LinkingBackend` |
| 48 | `impl TagRepository for LinkingBackend` | `// md:impl TagRepository for LinkingBackend` |
| 49 | `impl ResourceRepository for LinkingBackend` | `// md:impl ResourceRepository for LinkingBackend` |
| 50 | `impl SyncBackend for LinkingBackend` | `// md:impl SyncBackend for LinkingBackend` |
| 51 | `impl HistoryRepository for LinkingBackend` | `// md:impl HistoryRepository for LinkingBackend` |
| 52 | `mod tests` (container) | `// md:mod tests` |
| 53 | `fn backend` | `// md:mod tests > fn backend` |
| 54 | `fn nb` | `// md:mod tests > fn nb` |
| 55 | `fn aliased` | `// md:mod tests > fn aliased` |
| 56 | `fn derives_bookmarks_and_content_links` | `// md:mod tests > fn derives_bookmarks_and_content_links` |
| 57 | `fn bookmark_alias_comes_from_the_body_title` | `// md:mod tests > fn bookmark_alias_comes_from_the_body_title` |
| 58 | `fn resolves_link_by_alias_and_uuid` | `// md:mod tests > fn resolves_link_by_alias_and_uuid` |
| 59 | `fn rejects_duplicate_note_alias` | `// md:mod tests > fn rejects_duplicate_note_alias` |
| 60 | `fn same_alias_in_different_notebooks_is_allowed` | `// md:mod tests > fn same_alias_in_different_notebooks_is_allowed` |
| 61 | `fn inbox_note_cannot_carry_an_alias` | `// md:mod tests > fn inbox_note_cannot_carry_an_alias` |
| 62 | `fn set_note_alias_rejects_inbox_notes` | `// md:mod tests > fn set_note_alias_rejects_inbox_notes` |
| 63 | `fn moving_a_note_to_inbox_clears_its_alias` | `// md:mod tests > fn moving_a_note_to_inbox_clears_its_alias` |
| 64 | `fn inbox_note_does_not_link_out` | `// md:mod tests > fn inbox_note_does_not_link_out` |
| 65 | `fn nothing_links_to_an_inbox_note` | `// md:mod tests > fn nothing_links_to_an_inbox_note` |
| 66 | `fn bare_alias_resolves_globally_when_unique_else_scoped` | `// md:mod tests > fn bare_alias_resolves_globally_when_unique_else_scoped` |
| 67 | `fn alias_and_link_edits_reject_deleted_entities` | `// md:mod tests > fn alias_and_link_edits_reject_deleted_entities` |
| 68 | `fn add_and_remove_manual_link` | `// md:mod tests > fn add_and_remove_manual_link` |
| 69 | `fn resolves_two_segment_note_bookmark_shorthand` | `// md:mod tests > fn resolves_two_segment_note_bookmark_shorthand` |
| 70 | `fn two_segment_prefers_notebook_note` | `// md:mod tests > fn two_segment_prefers_notebook_note` |
| 71 | `fn alias_conflicts_lists_duplicates` | `// md:mod tests > fn alias_conflicts_lists_duplicates` |
| 72 | `fn alias_index_tracks_deletes_and_renames` | `// md:mod tests > fn alias_index_tracks_deletes_and_renames` |
| 73 | `fn sync_applied_change_invalidates_alias_index` | `// md:mod tests > fn sync_applied_change_invalidates_alias_index` |
| 74 | `fn concurrent_duplicate_alias_yields_exactly_one_winner` | `// md:mod tests > fn concurrent_duplicate_alias_yields_exactly_one_winner` |