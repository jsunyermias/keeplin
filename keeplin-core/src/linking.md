# `linking.rs` — `LinkingBackend` decorator + reference resolution

Self-contained companion for `keeplin-core/src/linking.rs`. It documents **every code
block of the source file, in source order** — a reader with only this file must be able
to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

```rust
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
// … async_trait, chrono, serde::Serialize, tokio::sync::{Mutex, RwLock}, uuid,
// crate::{error, links, models, ordering::is_inbox, storage traits}
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

**What it does** — Page size used when scanning every live note/notebook for
resolution and uniqueness.

**Dependencies** — none. **Used by** — `collect_notes`, `collect_notebooks`.
**Repeated context** — none.

---

## ResolvedReference

**Identification** — struct deriving `Debug, Clone, PartialEq, Eq`; marker
`// md:ResolvedReference`.

**What it does** — A resolved `#…` reference: the concrete `note_id` plus the
1-based `bookmark_number` when the reference had a (resolved) bookmark segment.

**Dependencies** — `uuid`. **Used by** — returned by `resolve`/`resolve_ref`;
the daemon's resolve endpoint. **Repeated context** — none.

---

## AliasConflict

**Identification** — `pub struct AliasConflict<T>` deriving `Debug, Clone,
Serialize`; marker `// md:AliasConflict`.

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

**What it does** — All current collisions, grouped by entity type (`notes`,
`notebooks`); empty vectors mean none.

**Dependencies** — `AliasConflict`. **Used by** — `alias_conflicts`; the
daemon's `GET /api/aliases/conflicts` / `ListAliasConflicts`.
**Repeated context** — none.

---

## AliasIndex

**Identification** — private `#[derive(Debug, Default)] struct AliasIndex`;
marker `// md:AliasIndex`.

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

### fn from_snapshots

**Identification** — `fn from_snapshots(notes: &[Note], notebooks: &[Notebook]) -> Self`;
marker `// md:impl AliasIndex > fn from_snapshots`.

**What it does** — Builds the index by upserting every snapshot entity.

**Used by** — `with_index`'s lazy build; `resolve_ref`.

### fn upsert_note

**Identification** — `fn upsert_note(&mut self, note: &Note)`; marker
`// md:impl AliasIndex > fn upsert_note`.

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

**What it does** — Drops a note's entry via the reverse map, pruning
now-empty alias sets. Used for deletes and as the first half of an upsert.

### fn upsert_notebook

**Identification** — marker `// md:impl AliasIndex > fn upsert_notebook`.

**What it does** — Notebook twin of `upsert_note` (no Inbox special case —
the Inbox notebook itself simply carries no alias).

### fn remove_notebook

**Identification** — marker `// md:impl AliasIndex > fn remove_notebook`.

**What it does** — Drops a notebook's entry, pruning empty sets.

### fn note_alias_taken

**Identification** —
`fn note_alias_taken(&self, alias: &str, self_id: Uuid, notebook_id: Uuid) -> bool`;
marker `// md:impl AliasIndex > fn note_alias_taken`.

**What it does** — Whether `alias` is carried by another live note **in the
same notebook** (`self_id` excluded) — per-notebook uniqueness, not global.

**Used by** — `prepare`.

### fn notebook_alias_taken

**Identification** — marker `// md:impl AliasIndex > fn notebook_alias_taken`.

**What it does** — Whether `alias` is carried by a live notebook other than
`self_id` (global uniqueness for notebooks).

**Used by** — `ensure_notebook_alias_free`.

### fn resolve_notebook_seg

**Identification** — `fn resolve_notebook_seg(&self, seg: &str) -> Option<Uuid>`;
marker `// md:impl AliasIndex > fn resolve_notebook_seg`.

**What it does** — A uuid segment is returned as-is (existence unchecked); an
alias picks the smallest-uuid live match.

**Used by** — `resolve_note_seg`.

### fn resolve_note_seg

**Identification** —
`fn resolve_note_seg(&self, seg, notebook_seg: Option<&str>, source_notebook: Option<Uuid>) -> Option<Uuid>`;
marker `// md:impl AliasIndex > fn resolve_note_seg`.

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

**What it does** — Whether a sync change can move an alias (or a note between
notebooks): any note or notebook create/update/delete.

**Dependencies** — `Change`. **Used by** — `apply_change`, `receive_changes`.
**Repeated context** — none.

---

## LinkingBackend

**Identification** — `pub struct LinkingBackend<B>`; marker
`// md:LinkingBackend`.

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

### fn new

**Identification** — `pub fn new(inner: B) -> Self`; marker
`// md:impl LinkingBackend > fn new`.

**What it does** — Wraps `inner` with a fresh lock and an unbuilt index.

### fn with_index

**Identification** —
`async fn with_index<R>(&self, f: impl FnOnce(&AliasIndex) -> R) -> Result<R, StorageError>`;
marker `// md:impl LinkingBackend > fn with_index`.

**What it does** — Runs `f` against the index, building it first (one corpus
scan via `collect_notes`/`collect_notebooks`) when absent or invalidated.
Double-checked locking: read-lock fast path, then write-lock rebuild — at most
one concurrent build.

**Used by** — `prepare`, `ensure_notebook_alias_free`.

### fn index_upsert_note

**Identification** — marker `// md:impl LinkingBackend > fn index_upsert_note`.

**What it does** — Folds a successfully written note into the index; a no-op
while unbuilt (the next build scans the store, which already reflects the
write).

### fn index_remove_note

**Identification** — marker `// md:impl LinkingBackend > fn index_remove_note`.

**What it does** — Drops a deleted note from the index (if built).

### fn index_upsert_notebook

**Identification** — marker
`// md:impl LinkingBackend > fn index_upsert_notebook`.

**What it does** — Folds a successfully written notebook into the index.

### fn index_remove_notebook

**Identification** — marker
`// md:impl LinkingBackend > fn index_remove_notebook`.

**What it does** — Drops a deleted notebook from the index.

### fn index_invalidate

**Identification** — marker `// md:impl LinkingBackend > fn index_invalidate`.

**What it does** — Discards the index so the next use rebuilds from the
store — called when a sync change lands, because what actually got stored
depends on conflict resolution inside the inner backend.

### fn refresh

**Identification** — `fn refresh(note: &mut Note)` (associated, pure); marker
`// md:impl LinkingBackend > fn refresh`.

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

**What it does** — Rejects a notebook whose alias collides with another live
notebook (`Conflict`); an alias-less notebook passes immediately, no index
needed.

**Used by** — `create_notebook`, `update_notebook`.

---

## fn collect_notes

**Identification** — `pub async fn collect_notes(backend: &dyn StorageBackend) -> Result<Vec<Note>, StorageError>`;
marker `// md:fn collect_notes`.

**What it does** — Every live note, by exhausting the paginated `list_notes`
at `SCAN_PAGE`. (The free helpers exist because the decorator sits behind
`Arc<dyn StorageBackend>`, so surfaces cannot call inherent methods; their
writes flow back through the decorator, so derivation, resolution and
uniqueness still apply.)

**Used by** — `with_index` build, `resolve`, `alias_conflicts`; the daemon.

---

## fn collect_notebooks

**Identification** — marker `// md:fn collect_notebooks`.

**What it does** — Every live notebook, same pattern.

**Used by** — `with_index` build, `resolve`, `alias_conflicts`.

---

## fn resolve_bookmark_seg

**Identification** —
`fn resolve_bookmark_seg(seg: &str, note_id: Uuid, notes: &[Note]) -> Option<u32>`;
marker `// md:fn resolve_bookmark_seg`.

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

**What it does** — Collects live notes + notebooks and runs `resolve_ref` —
the store-backed resolution the daemon's resolve endpoint uses.

**Used by** — `rest.rs`/`server.rs` resolve endpoints; tests.

---

## fn backlinks

**Identification** —
`pub async fn backlinks(backend, target_id, page_size, page_token) -> Result<(Vec<Note>, Option<String>), StorageError>`;
marker `// md:fn backlinks`.

**What it does** — A page of live notes linking to `target_id`, delegating to
`NoteRepository::note_backlinks` (indexed on `DbBackend`, `O(N)` in-memory
pagination elsewhere).

**Used by** — the daemon's backlinks endpoints.

---

## fn group_conflicts

**Identification** —
`fn group_conflicts<T>(items, alias_of, id_of) -> Vec<AliasConflict<T>>`; marker
`// md:fn group_conflicts`.

**What it does** — Groups items by their optional alias, keeping only aliases
shared by ≥ 2 entities; groups ordered by alias, entities by uuid.

**Used by** — `alias_conflicts` (notebook side).

---

## fn alias_conflicts

**Identification** —
`pub async fn alias_conflicts(backend: &dyn StorageBackend) -> Result<AliasConflicts, StorageError>`;
marker `// md:fn alias_conflicts`.

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

**What it does** — Groups note conflicts by `(alias, notebook)` — the same
alias in two different notebooks is *not* a conflict (per-notebook
uniqueness). Inbox notes are skipped (they carry no alias). Entities ordered
by uuid.

**Used by** — `alias_conflicts`.

---

## fn read_live_note

**Identification** — `async fn read_live_note(backend, id) -> Result<Note, StorageError>`;
marker `// md:fn read_live_note`.

**What it does** — Reads a note for a user-facing read-modify-write, rejecting
a tombstone as `NotFound`. Without this, the RMW would silently **revive** a
deleted note (writing `deleted_at: None` back). Revival is reserved for the
sync path (`apply_change` resolving a causal edit made after the delete),
never a side effect of an alias or link edit.

**Used by** — `set_note_alias`, `add_manual_link`, `remove_link`.

---

## fn read_live_notebook

**Identification** — marker `// md:fn read_live_notebook`.

**What it does** — Notebook twin of `read_live_note`.

**Used by** — `set_notebook_alias`.

---

## fn set_note_alias

**Identification** —
`pub async fn set_note_alias(backend, note_id, alias: Option<String>) -> Result<Note, StorageError>`;
marker `// md:fn set_note_alias`.

**What it does** — Sets or clears a note's alias (RMW → one `NoteUpdate`). A
soft-deleted note is `NotFound`; **setting** an alias on an Inbox note is
`InvalidInput` (Inbox notes carry no alias — rejected rather than silently
cleared, matching the other Inbox domain rules such as pinning); clearing an
already-null alias is a no-op that succeeds.

**Used by** — the daemon's alias endpoints.

---

## fn set_notebook_alias

**Identification** — marker `// md:fn set_notebook_alias`.

**What it does** — Sets or clears a notebook's alias; a soft-deleted notebook
is `NotFound`.

**Used by** — the daemon's alias endpoints.

---

## fn add_manual_link

**Identification** —
`pub async fn add_manual_link(backend, note_id, raw: &str) -> Result<Note, StorageError>`;
marker `// md:fn add_manual_link`.

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

**What it does** — Removes the link at `index` into the note's `links`
(`NotFound` when out of range) and persists. A soft-deleted note is
`NotFound`.

**Used by** — the daemon's link endpoints.

---

## impl NoteRepository for LinkingBackend

**Identification** — marker `// md:impl NoteRepository for LinkingBackend`
(one marker for the impl block).

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

**What it does** — Pure delegation for all eight methods — tags carry no
aliases and no links.

**Used by** — tag traffic. **Dependencies** — the inner backend.
**Repeated context** — none.

---

## impl ResourceRepository for LinkingBackend

**Identification** — marker
`// md:impl ResourceRepository for LinkingBackend`.

**What it does** — Pure delegation for all five methods.

**Used by** — resource traffic. **Dependencies** — the inner backend.
**Repeated context** — none.

---

## impl SyncBackend for LinkingBackend

**Identification** — marker `// md:impl SyncBackend for LinkingBackend`.

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

**What it does** — Pure delegation: history returns snapshots as stored (their
links/bookmarks were derived at write time).

**Used by** — history endpoints. **Dependencies** — the inner backend.
**Repeated context** — none.

---

## mod tests

**Identification** — `#[cfg(test)]` test module; marker `// md:mod tests`.
Three helpers + seventeen tests over `LinkingBackend<FsBackend>` in leaked
tempdirs.

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

**What it does** — A decorator over `FsBackend` in a leaked tempdir.

### fn nb

**Identification** — helper `fn nb() -> Uuid`; marker
`// md:mod tests > fn nb`.

**What it does** — A fixed non-Inbox notebook id — alias/link tests must place
notes in a real notebook (Inbox notes carry no alias and do not link).

### fn aliased

**Identification** — helper `fn aliased(title, alias) -> Note`; marker
`// md:mod tests > fn aliased`.

**What it does** — A note with `alias` in the notebook `nb()`.

### fn derives_bookmarks_and_content_links

**Identification** — tokio test; marker
`// md:mod tests > fn derives_bookmarks_and_content_links`.

**What it does** — A body with two bookmark declarations (one titled) and a
content link stores two numbered bookmarks (alias defaults to the text; a
title becomes the alias) and one `Content` link with the raw reference.

### fn bookmark_alias_comes_from_the_body_title

**Identification** — tokio test; marker
`// md:mod tests > fn bookmark_alias_comes_from_the_body_title`.

**What it does** — The alias lives in the body: editing the link title renames
the bookmark alias.

### fn resolves_link_by_alias_and_uuid

**Identification** — tokio test; marker
`// md:mod tests > fn resolves_link_by_alias_and_uuid`.

**What it does** — An alias link resolves to the target's id; a three-segment
uuid ref resolves note + bookmark number; backlinks list the source.

### fn rejects_duplicate_note_alias

**Identification** — tokio test; marker
`// md:mod tests > fn rejects_duplicate_note_alias`.

**What it does** — Same alias, same notebook → `Conflict`.

### fn same_alias_in_different_notebooks_is_allowed

**Identification** — tokio test; marker
`// md:mod tests > fn same_alias_in_different_notebooks_is_allowed`.

**What it does** — Same alias in two different notebooks is fine.

### fn inbox_note_cannot_carry_an_alias

**Identification** — tokio test; marker
`// md:mod tests > fn inbox_note_cannot_carry_an_alias`.

**What it does** — An alias set on an Inbox create is dropped, not stored.

### fn set_note_alias_rejects_inbox_notes

**Identification** — tokio test; marker
`// md:mod tests > fn set_note_alias_rejects_inbox_notes`.

**What it does** — The explicit endpoint rejects (`InvalidInput`) setting an
alias on an Inbox note; clearing an absent alias still succeeds.

### fn moving_a_note_to_inbox_clears_its_alias

**Identification** — tokio test; marker
`// md:mod tests > fn moving_a_note_to_inbox_clears_its_alias`.

**What it does** — Moving an aliased note into the Inbox clears the alias.

### fn inbox_note_does_not_link_out

**Identification** — tokio test; marker
`// md:mod tests > fn inbox_note_does_not_link_out`.

**What it does** — An Inbox note referencing a target resolves no link and
produces no backlink.

### fn nothing_links_to_an_inbox_note

**Identification** — tokio test; marker
`// md:mod tests > fn nothing_links_to_an_inbox_note`.

**What it does** — A uuid reference to an Inbox note does not resolve, and the
Inbox note has no backlinks.

### fn bare_alias_resolves_globally_when_unique_else_scoped

**Identification** — tokio test; marker
`// md:mod tests > fn bare_alias_resolves_globally_when_unique_else_scoped`.

**What it does** — A unique bare alias resolves from any notebook; once the
alias exists in two notebooks, a bare ref scopes to the source note's own
notebook.

### fn alias_and_link_edits_reject_deleted_entities

**Identification** — tokio test; marker
`// md:mod tests > fn alias_and_link_edits_reject_deleted_entities`.

**What it does** — `set_note_alias`/`add_manual_link`/`remove_link` on a
tombstoned note are all `NotFound` and write nothing (the note stays
tombstoned); `set_notebook_alias` likewise.

### fn add_and_remove_manual_link

**Identification** — tokio test; marker
`// md:mod tests > fn add_and_remove_manual_link`.

**What it does** — A manual link is added (`source = Manual`) and removed by
index.

### fn resolves_two_segment_note_bookmark_shorthand

**Identification** — tokio test; marker
`// md:mod tests > fn resolves_two_segment_note_bookmark_shorthand`.

**What it does** — `#note#bookmark` resolves by bookmark alias and by number.

### fn two_segment_prefers_notebook_note

**Identification** — tokio test; marker
`// md:mod tests > fn two_segment_prefers_notebook_note`.

**What it does** — `#notebook#note` resolves to the note when the second
segment is a resolvable note (not reinterpreted as `note#bookmark`).

### fn alias_conflicts_lists_duplicates

**Identification** — tokio test; marker
`// md:mod tests > fn alias_conflicts_lists_duplicates`.

**What it does** — Duplicates planted through a raw `FsBackend` (the way sync
would, bypassing the write-time check) are listed by `alias_conflicts`, with
unique aliases excluded.

### fn alias_index_tracks_deletes_and_renames

**Identification** — tokio test; marker
`// md:mod tests > fn alias_index_tracks_deletes_and_renames`.

**What it does** — A delete frees the alias for reuse; a rename frees the old
name and occupies the new one (claiming the new name again is `Conflict`).

### fn sync_applied_change_invalidates_alias_index

**Identification** — tokio test; marker
`// md:mod tests > fn sync_applied_change_invalidates_alias_index`.

**What it does** — After warming the index, a notebook alias arriving via
`apply_change` (bypassing uniqueness) must still cause a later local claim of
that alias to be rejected — proving the invalidation.

### fn concurrent_duplicate_alias_yields_exactly_one_winner

**Identification** — multi-thread tokio test; marker
`// md:mod tests > fn concurrent_duplicate_alias_yields_exactly_one_winner`.

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
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `const SCAN_PAGE` | `// md:SCAN_PAGE` |
| 3 | `struct ResolvedReference` | `// md:ResolvedReference` |
| 4 | `struct AliasConflict<T>` | `// md:AliasConflict` |
| 5 | `struct AliasConflicts` | `// md:AliasConflicts` |
| 6 | `struct AliasIndex` | `// md:AliasIndex` |
| 7 | `impl AliasIndex` (+ ten methods) | `// md:impl AliasIndex` (+ `> fn …`) |
| 8 | `fn change_affects_aliases` | `// md:fn change_affects_aliases` |
| 9 | `struct LinkingBackend` | `// md:LinkingBackend` |
| 10 | `impl LinkingBackend` (+ ten methods) | `// md:impl LinkingBackend` (+ `> fn …`) |
| 11 | `fn collect_notes` | `// md:fn collect_notes` |
| 12 | `fn collect_notebooks` | `// md:fn collect_notebooks` |
| 13 | `fn resolve_bookmark_seg` | `// md:fn resolve_bookmark_seg` |
| 14 | `fn resolve_ref` | `// md:fn resolve_ref` |
| 15 | `fn resolve` | `// md:fn resolve` |
| 16 | `fn backlinks` | `// md:fn backlinks` |
| 17 | `fn group_conflicts` | `// md:fn group_conflicts` |
| 18 | `fn alias_conflicts` | `// md:fn alias_conflicts` |
| 19 | `fn group_note_conflicts` | `// md:fn group_note_conflicts` |
| 20 | `fn read_live_note` | `// md:fn read_live_note` |
| 21 | `fn read_live_notebook` | `// md:fn read_live_notebook` |
| 22 | `fn set_note_alias` | `// md:fn set_note_alias` |
| 23 | `fn set_notebook_alias` | `// md:fn set_notebook_alias` |
| 24 | `fn add_manual_link` | `// md:fn add_manual_link` |
| 25 | `fn remove_link` | `// md:fn remove_link` |
| 26 | `impl NoteRepository for LinkingBackend` (9 methods) | `// md:impl NoteRepository for LinkingBackend` |
| 27 | `impl NotebookRepository for LinkingBackend` (5 methods) | `// md:impl NotebookRepository for LinkingBackend` |
| 28 | `impl TagRepository for LinkingBackend` (8 methods) | `// md:impl TagRepository for LinkingBackend` |
| 29 | `impl ResourceRepository for LinkingBackend` (5 methods) | `// md:impl ResourceRepository for LinkingBackend` |
| 30 | `impl SyncBackend for LinkingBackend` (8 methods) | `// md:impl SyncBackend for LinkingBackend` |
| 31 | `impl HistoryRepository for LinkingBackend` (2 methods) | `// md:impl HistoryRepository for LinkingBackend` |
| 32 | `mod tests` (+ 3 helpers + 17 tests) | `// md:mod tests` (+ `> fn …`) |
