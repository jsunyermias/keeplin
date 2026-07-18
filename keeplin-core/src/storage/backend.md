# `storage/backend.rs` — the `StorageBackend` supertrait and its sub-traits

Self-contained companion for `keeplin-core/src/storage/backend.rs`. It documents
**every code block of the source file, in source order** — a reader with only this
file must be able to understand it without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

```rust
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
};

use super::SortableRfc3339;
```

**What it does** — Defines the storage layer's contract. Rather than one 30-method
trait, it is split into five cohesive sub-traits — `NoteRepository`,
`NotebookRepository`, `TagRepository`, `ResourceRepository`, `SyncBackend` — plus
`HistoryRepository`; `StorageBackend` is a supertrait requiring all of them, giving
call-sites a single bound while keeping each domain independently testable and
mockable. A blanket impl satisfies `StorageBackend` for any type implementing the
sub-traits, so a new backend only writes the focused `impl` blocks — no glue.
`Send + Sync + 'static` bounds let implementors live in an `Arc`, cross
`tokio::spawn`, and sit in the tonic server struct; `async-trait` boxes each future
(one small heap alloc per call, negligible next to the I/O) so the traits stay
object-safe.

**Dependencies** — `async_trait`, `chrono`, `uuid`, `crate::error::StorageError`,
the `crate::models` entity types, `super::SortableRfc3339` (fixed-precision
RFC 3339 for the cursor format).

**Used by** — implemented by `storage/fs.rs`, `storage/db.rs`, and the decorators
(`encryption.rs`, `linking.rs`, `collab/mod.rs`, daemon-side `event_backend.rs`,
`metrics.rs`); consumed by everything that touches storage. The rest of the
codebase is written against `Arc<dyn StorageBackend>` and never names a concrete
backend.

**Repeated context** — Project-wide storage conventions the traits encode:
**soft-delete always** (`delete_*` sets `deleted_at`, records are retained,
`list_*` excludes them), **idempotent `apply_change`**, cursor pagination
(`page_size = 0` → default 100, cap 1000; cursor `None` = start / end), and
version-vector conflict resolution with the `(timestamp, device_id)` tiebreak.
Default methods are preferred for additive trait evolution — a signature change
ripples through every backend *and* every decorator.

---

## trait NoteRepository

**Identification** — `#[async_trait] pub trait NoteRepository: Send + Sync + 'static`;
marker `// md:trait NoteRepository`.

**What it does** — CRUD for `Note` entities. `delete_note` is a **soft delete**
(sets `deleted_at`, keeps the record); `list_notes` excludes soft-deleted notes.
Methods:

| Method | Contract |
|--------|----------|
| `create_note(note) -> Note` | persist and return the stored copy (may differ — e.g. `EncryptedBackend` returns the decrypted copy after storing ciphertext) |
| `read_note(id) -> Note` | `NotFound` when absent **or soft-deleted** |
| `update_note(note) -> Note` | overwrite all fields; `NotFound` when absent |
| `delete_note(id)` | soft delete (stamp `deleted_at = now`); `NotFound` when absent |
| `list_notes(page_size, token)` | live notes, `(created_at ASC, id ASC)`; standard pagination |
| `list_notes_in_notebook(nb, size, token)` | live notes of one notebook ordered by `(effective_sort_key ASC, id ASC)` — the manual order, pinned band (`sort_key 1..=999`) first; cursor `"<sort_key>|<uuid>"` over the *effective* key |
| `list_starred_notes(size, token)` | every live starred note across all notebooks (Inbox included), `(created_at, id)` order |
| `notebook_sort_profile(nb) -> NotebookSortProfile` | compact ordering summary for `crate::ordering`'s placement rules — never materialises the notebook |
| `note_backlinks(target, size, token)` | live notes linking **to** `target_id`; default implementation provided (below) |

The `note_backlinks` **default implementation** exhausts `list_notes`, filters by
each link's `target_note_id`, and paginates in memory with `paginate_notes`
(matches is already `(created_at, id)`-ordered because `list_notes` is) — correct
but `O(N)`. Backends with a link index (`DbBackend`'s `note_links` table) override
it with an indexed lookup; **decorators must delegate to their inner backend**
rather than inheriting this default, or the indexed override is never reached
(see `EncryptedBackend`/`LinkingBackend`).

**Dependencies** — `Note`, `NotebookSortProfile`, `paginate_notes`,
`StorageError`.

**Used by** — every storage consumer; the daemon's note endpoints; `migrate.rs`;
`ordering.rs` read-modify-write operations.

**Repeated context** — pagination contract restated: `page_size = 0` →
`DEFAULT_PAGE_SIZE` (100), values above `MAX_PAGE_SIZE` (1000) clamp;
`page_token = None` starts at the beginning; a `None` next-cursor means no
further pages.

---

## NotebookSortProfile

**Identification** — struct deriving `Debug, Clone, Default, PartialEq, Eq`;
marker `// md:NotebookSortProfile`.

**What it does** — A compact summary of one notebook's live-note ordering,
computed natively by each backend (an indexed scan of sort keys — never the note
bodies): `pinned_keys` (keys currently used in the pinned band `1..=999`,
ascending), `min_key` (smallest effective key, `None` when the notebook has no
live notes), `max_normal_key` (largest key in the normal band `>= 1000`, `None`
when that band is empty). All keys are *effective* keys — the legacy `0` sentinel
already mapped to `Note::DEFAULT_SORT_KEY`.

**Dependencies** — none.

**Used by** — `NoteRepository::notebook_sort_profile`; `crate::ordering`'s
placement rules (new-note position, pin, unpin).

**Repeated context** — the sort-key model: `1..=999` is the pinned band,
`>= 1000` the normal band; placement logic works from this profile so it stays
O(keys), not O(notes×bodies).

---

## impl NotebookSortProfile

**Identification** — inherent impl; marker `// md:impl NotebookSortProfile`. One
method.

### fn from_effective_keys

**Identification** —
`pub fn from_effective_keys(keys: impl IntoIterator<Item = u32>) -> Self`; marker
`// md:impl NotebookSortProfile > fn from_effective_keys`.

**What it does** — Builds a profile from an iterator of the notebook's live
effective sort keys: tracks the global minimum, routes `1..1000` keys into
`pinned_keys` and everything else into the `max_normal_key` maximum, then sorts
`pinned_keys` ascending. Pure, total.

**Dependencies** — none.

**Used by** — the backends' `notebook_sort_profile` implementations (`fs.rs`,
`db.rs`).

**Repeated context** — none.

---

## fn paginate_notes

**Identification** —
`fn paginate_notes(items: Vec<Note>, page_size: u32, token: Option<&str>) -> (Vec<Note>, Option<String>)`;
marker `// md:fn paginate_notes`.

**What it does** — Paginates an already-`(created_at, id)`-ordered vec of notes
with the same `"created_at|id"` cursor format the backends' `list_*` methods use.
Resolves the start index via `partition_point` over
`(to_sortable_rfc3339(), id)` — strictly after the cursor row; an empty or
malformed token starts at the beginning. Applies `effective_page_size` (0 → 100,
cap 1000) and emits a next-cursor from the page's last row only when more items
remain.

**Dependencies** — `super::effective_page_size`, `SortableRfc3339`, `uuid`.

**Used by** — the default `NoteRepository::note_backlinks` implementation (its
only caller).

**Repeated context** — cursors compare timestamps **as text**, which is why both
sides of the comparison use the fixed nine-fractional-digit
`to_sortable_rfc3339` shape.

---

## trait NotebookRepository

**Identification** — `#[async_trait] pub trait NotebookRepository: Send + Sync + 'static`;
marker `// md:trait NotebookRepository`.

**What it does** — CRUD for `Notebook` entities with the same soft-delete
semantics as notes: `create_notebook` (persist, return stored copy),
`read_notebook` (`NotFound` when absent), `update_notebook` (`NotFound` when
absent), `delete_notebook` (soft), `list_notebooks` (live only,
`(created_at ASC, id ASC)`, standard pagination).

**Dependencies** — `Notebook`, `StorageError`.

**Used by** — storage consumers, the daemon's notebook endpoints, `migrate.rs`.

**Repeated context** — the Inbox is a *system notebook* (nil-UUID, defined in
`crate::ordering`) that must never be deleted — enforced in the ordering/backend
layer, not by these signatures.

---

## trait TagRepository

**Identification** — `#[async_trait] pub trait TagRepository: Send + Sync + 'static`;
marker `// md:trait TagRepository`.

**What it does** — CRUD for `Tag` entities plus the note–tag association table
(kept here, not a separate trait: associations are always used together with tag
reads and have no independent lifecycle beyond the tags). `create_tag` /
`read_tag` / `update_tag` / `delete_tag` (soft) / `list_tags` follow the standard
contracts. Association methods:

- `add_note_tag(NoteTag)` — **idempotent** (attaching an already-attached tag
  succeeds); `NotFound` when the note or tag is absent/soft-deleted — the API
  must not create dangling associations. (`apply_change` deliberately skips this
  validation: sync delivery order is not guaranteed, so an association may
  arrive before its note or tag.)
- `remove_note_tag(note_id, tag_id)` — idempotent; success even if the
  association did not exist.
- `list_note_tags(note_id, size, token)` — tags currently attached to the note,
  `(created_at, id)` order, standard pagination.

**Dependencies** — `Tag`, `NoteTag`, `StorageError`.

**Used by** — storage consumers, the daemon's tag endpoints, `migrate.rs`.

**Repeated context** — idempotent association ops are what make sync replays and
migration re-runs safe.

---

## trait ResourceRepository

**Identification** — `#[async_trait] pub trait ResourceRepository: Send + Sync + 'static`;
marker `// md:trait ResourceRepository`.

**What it does** — CRUD for binary `Resource` attachments. Resources use the same
soft-delete tombstone (`deleted_at` + version vector) as every other entity so a
concurrent delete-vs-recreate converges; the binary payload is retained after a
soft delete. Methods:

- `create_resource(resource, data)` — store metadata alongside the raw bytes
  (PNG, PDF, …); returns the metadata.
- `read_resource(id) -> (Resource, Vec<u8>)` — both; `NotFound` when absent.
- `delete_resource(id)` — stamps the tombstone **plus a bumped version vector**
  so the delete competes in conflict resolution; the resource then reads as
  `NotFound` and is excluded from listings; the payload is retained.
- `list_resources(size, token)` — metadata only, `(created_at, id)` order; fetch
  bytes per-id via `read_resource`.
- `purge_deleted_resources(older_than) -> u64` — reclaims the payload bytes of
  tombstones older than the cutoff, returning how many were freed. Tombstone
  **metadata is always retained** (it must keep competing in resolution so the
  deletion converges); reads were already `NotFound`, so no read path changes.
  The cutoff exists because a *concurrent* revive on a peer that has not synced
  yet can still win resolution and legitimately need the payload; a generous
  window (the daemon uses days, mirroring journal retention) makes that race
  practically closed, and a revive landing after a purge is made whole by the
  revive itself, which always carries (or replicates) a fresh payload.

**Dependencies** — `Resource`, `StorageError`, `chrono`.

**Used by** — the daemon's resource endpoints and maintenance loop; `migrate.rs`.

**Repeated context** — soft-delete-always applies to binaries too: only
out-of-band maintenance reclaims space, never the delete itself.

---

## trait SyncBackend

**Identification** — `#[async_trait] pub trait SyncBackend: Send + Sync + 'static`;
marker `// md:trait SyncBackend`.

**What it does** — Device identification and change-journal synchronisation — the
operations `crate::sync::SyncEngine` sequences (collect → send → receive → apply
→ update timestamp → optionally prune). Methods:

| Method | Contract |
|--------|----------|
| `get_device_id() -> String` | stable per-installation id, generated once and persisted; also the Argon2id salt for AES key derivation and the change-log file name (`logs/{device_id}.log`) |
| `get_last_sync_time()` | UTC time of the last successful cycle; Unix epoch if never synced |
| `update_sync_time(ts)` | overwrite the watermark (called at the end of a successful cycle) |
| `get_changes_since(since)` | this device's `Change` events after `since`, in recorded order |
| `apply_change(change)` | apply one incoming change; **must be idempotent** |
| `send_changes(changes)` | `DbBackend`: WebSocket with exponential-backoff retry; `FsBackend`: no-op (Syncthing replicates the logs) |
| `receive_changes()` | `DbBackend`: drain the WebSocket; `FsBackend`: empty list (peers' changes are discovered by scanning their log files in `get_changes_since`) |
| `prune_change_journal(older_than) -> u64` | `DbBackend`: delete old `entity_changes` rows; `FsBackend`: always `Ok(0)` — pruning per-device logs could permanently lose changes for peers that haven't processed them |

Idempotency: applying the same `Change` twice must equal applying it once — the
built-ins satisfy this with version-vector resolution (re-applying a change the
store already dominates is a no-op) plus `INSERT OR IGNORE`/`REPLACE` writes and
no-op deletes; it converts at-least-once delivery into effectively-exactly-once
outcomes. Conflict resolution is **unified on version vectors** across both
backends: every entity resolves through `crate::storage::note_log`'s `resolve`
(state-based, for `DbBackend` rows and `FsBackend` sidecars) or `merge` (for
`FsBackend`'s per-note logs), which share the same domination test and
`(timestamp, device_id)` tiebreak, so every device converges on the same winner.
The storage shape differs; the decision does not (see `SECURITY.md`).

**Dependencies** — `Change`, `StorageError`, `chrono`.

**Used by** — `sync/engine.rs`; the daemon's maintenance loop (pruning);
`keeplin-daemon/src/search.rs` (re-indexes on applied changes).

**Repeated context** — none beyond the above.

---

## DEFAULT_HISTORY_LIMIT

**Identification** — `pub const DEFAULT_HISTORY_LIMIT: u32 = 100;` marker
`// md:DEFAULT_HISTORY_LIMIT`.

**What it does** — Default cap on the number of versions a `*_history` call
returns when the caller passes `limit = 0`. Bounds a single reply regardless of
how deep the journal retains history.

**Dependencies** — none.

**Used by** — the backends' history implementations; re-exported from
`storage/mod.rs`. `crate::history`'s reverts bypass it with an explicit
`REVERT_SCAN_LIMIT` (10 000).

**Repeated context** — none.

---

## EntityVersion

**Identification** — `pub struct EntityVersion<T>` deriving `Debug, Clone,
PartialEq, Eq`; marker `// md:EntityVersion`.

**What it does** — One past version of an entity, reconstructed from the change
journal: `timestamp` (wall-clock time the version was written — the edit's
`updated_at`/`deleted_at`), `device_id` (the authoring device),
`entity: Option<T>` — `None` marks a **tombstone** version (the entity was
soft-deleted at that point; a later version may revive it). The journal stores a
full snapshot per change, so history is *derived* from it rather than kept in a
parallel store; versions decrypt on the way up through
`crate::encryption::EncryptedBackend`, so the payload here is always plaintext.

**Dependencies** — `chrono`.

**Used by** — `HistoryRepository`'s return types; `crate::history::state_at` and
the revert helpers; the daemon's version DTOs.

**Repeated context** — none.

---

## trait HistoryRepository

**Identification** — `#[async_trait] pub trait HistoryRepository: Send + Sync + 'static`;
marker `// md:trait HistoryRepository`.

**What it does** — Read-only access to an entity's past versions, the raw
material for `crate::history`'s forward-revert helpers: `note_history(id, limit)`
and `notebook_history(id, limit)`, both newest-first (index 0 = current state),
bounded by `limit` (`0` = `DEFAULT_HISTORY_LIMIT`). An unknown id yields an
**empty list**, not `NotFound`, so callers treat "no history" and "no such note"
uniformly. Sources: per-device op logs in `FsBackend`; the `entity_changes`
journal (server first, local fallback) in `DbBackend`. What survives is governed
by the journal's retention policy (version count and, unless disabled, age).

**Dependencies** — `EntityVersion`, `Note`, `Notebook`, `StorageError`.

**Used by** — `crate::history`; the daemon's history endpoints.

**Repeated context** — history is derived from the journal — there is no
separate history store to migrate or corrupt.

---

## trait StorageBackend

**Identification** —
`pub trait StorageBackend: NoteRepository + NotebookRepository + TagRepository + ResourceRepository + SyncBackend + HistoryRepository {}`;
marker `// md:trait StorageBackend`.

**What it does** — The unified async storage interface: an empty supertrait whose
bounds pull in every sub-trait. Generic code writes `T: StorageBackend` once; all
sub-trait methods are available because supertrait bounds are transitive.
Implemented (via the blanket impl below) by `FsBackend`, `DbBackend`, and every
decorator.

**Dependencies** — the six sub-traits.

**Used by** — nearly every module: as a generic bound (`sync::SyncEngine<T>`,
the decorators' `B: StorageBackend`) and as a trait object (`&dyn StorageBackend`
in `history.rs`/`migrate.rs`, `Arc<dyn StorageBackend>` in the daemon).

**Repeated context** — none.

---

## impl StorageBackend for T

**Identification** — blanket impl
`impl<T: ?Sized> StorageBackend for T where T: NoteRepository + … + HistoryRepository {}`;
marker `// md:impl StorageBackend for T`.

**What it does** — Any type satisfying all sub-traits automatically satisfies
`StorageBackend`: a new backend writes only the focused `impl` blocks, no glue.
The `T: ?Sized` bound also lets the trait object `dyn StorageBackend` itself
satisfy `StorageBackend` (it auto-implements the object-safe sub-traits), so an
`Arc<dyn StorageBackend>` can be passed where a `B: StorageBackend` bound is
expected — e.g. to `crate::sync::run_sync` (whose bound is
`B: StorageBackend + ?Sized`) from the daemon's type-erased REST layer.

**Dependencies** — the sub-traits.

**Used by** — every backend and decorator, implicitly.

**Repeated context** — none.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `StorageBackend` — defined here (EXTRACTED; 56 cross-file edge(s))
- `EntityVersion` — defined here (EXTRACTED; 21 cross-file edge(s))
- `T` — defined here (EXTRACTED; 20 cross-file edge(s))
- `NotebookSortProfile` — defined here (EXTRACTED; 8 cross-file edge(s))
- `NoteRepository` — defined here (EXTRACTED; 7 cross-file edge(s))
- `NotebookRepository` — defined here (EXTRACTED; 7 cross-file edge(s))
- `TagRepository` — defined here (EXTRACTED; 7 cross-file edge(s))
- `ResourceRepository` — defined here (EXTRACTED; 7 cross-file edge(s))
- `SyncBackend` — defined here (EXTRACTED; 7 cross-file edge(s))
- `HistoryRepository` — defined here (EXTRACTED; 7 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: references×1; e.g. `Note`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/collab/mod.rs` — client of the keeplin-srv collaborative channel (EXTRACTED: implements×6, references×5; e.g. `Shared`, `CollabBackend<B>`, `.start()`)
- `keeplin-core/src/encryption.rs` — transparent at-rest encryption (EXTRACTED: implements×6, references×3; e.g. `EncryptedBackend<B>`, `.notebook_sort_profile()`, `.note_history()`)
- `keeplin-core/src/history.rs` — change history reads + forward-revert (EXTRACTED: references×7; e.g. `state_at()`, `revert_note()`, `revert_notebook()`)
- `keeplin-core/src/interop.rs` — vCard & iCalendar format compatibility (EXTRACTED: imports_from×1, references×15; e.g. `interop.rs`, `resources_with_mime()`, `resource_metas_with_mime()`)
- `keeplin-core/src/linking.rs` — `LinkingBackend` decorator + reference resolution (EXTRACTED: implements×6, references×16; e.g. `AliasConflict`, `LinkingBackend<B>`, `collect_notes()`)
- `keeplin-core/src/migrate.rs` — one-shot state copy between backends (EXTRACTED: references×2; e.g. `migrate()`, `collect()`)
- `keeplin-core/src/ordering.rs` — the Inbox, pinning, manual ordering, and starring (EXTRACTED: references×12; e.g. `ensure_inbox()`, `place_new_note()`, `pin_note()`)
- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: implements×6, references×8; e.g. `DbBackend`, `.notebook_sort_profile()`, `.entity_history()`)
- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: implements×6, references×12; e.g. `FsBackend`, `.notebook_sort_profile()`, `.note_history()`)
- `keeplin-core/src/sync/engine.rs` — SyncEngine (EXTRACTED: references×2; e.g. `SyncEngine`, `.new()`)
- `keeplin-core/tests/migrate.rs` — cross-backend migration tests (EXTRACTED: references×2; e.g. `assert_migrated()`, `seed()`)
- `keeplin-daemon/src/event_backend.rs` — `EventBackend` change-publishing decorator (EXTRACTED: implements×6, references×3; e.g. `EventBackend<B>`, `.notebook_sort_profile()`, `.note_history()`)
- `keeplin-daemon/src/main.rs` — daemon entry point (EXTRACTED: references×1; e.g. `build_storage()`)
- `keeplin-daemon/src/metrics.rs` — operational metrics (EXTRACTED: implements×6, references×4; e.g. `MetricsBackend<B>`, `.notebook_sort_profile()`, `.note_history()`)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: references×4; e.g. `note_version_dto()`, `notebook_version_dto()`, `AppState`)
- `keeplin-daemon/src/search.rs` — daemon full-text search (EXTRACTED: imports_from×1, references×6; e.g. `apply_change()`, `denormalize()`, `index_note()`)
- `keeplin-daemon/src/server.rs` — gRPC service implementation (EXTRACTED: references×1; e.g. `ensure_not_deleted()`)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `trait NoteRepository` (incl. default `note_backlinks`) | `// md:trait NoteRepository` |
| 3 | `struct NotebookSortProfile` | `// md:NotebookSortProfile` |
| 4 | `impl NotebookSortProfile` | `// md:impl NotebookSortProfile` |
| 5 | `fn from_effective_keys` | `// md:impl NotebookSortProfile > fn from_effective_keys` |
| 6 | `fn paginate_notes` | `// md:fn paginate_notes` |
| 7 | `trait NotebookRepository` | `// md:trait NotebookRepository` |
| 8 | `trait TagRepository` | `// md:trait TagRepository` |
| 9 | `trait ResourceRepository` | `// md:trait ResourceRepository` |
| 10 | `trait SyncBackend` | `// md:trait SyncBackend` |
| 11 | `const DEFAULT_HISTORY_LIMIT` | `// md:DEFAULT_HISTORY_LIMIT` |
| 12 | `struct EntityVersion<T>` | `// md:EntityVersion` |
| 13 | `trait HistoryRepository` | `// md:trait HistoryRepository` |
| 14 | `trait StorageBackend` | `// md:trait StorageBackend` |
| 15 | blanket `impl StorageBackend for T` | `// md:impl StorageBackend for T` |
