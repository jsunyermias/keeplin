# `storage/backend.rs` — the `StorageBackend` supertrait and its sub-traits

Self-contained companion for `keeplin-core/src/storage/backend.rs`. It documents
**every code block of the source file, in source order** — a reader with only this
file must be able to understand it without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

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

**Used by** — implemented by `storage/fs.rs`, the trait modules of `storage/db/`, and the decorators
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

**Code** — complete and verbatim:

```rust
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
```

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

**Code** — complete and verbatim:

```rust
// md:NotebookSortProfile
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotebookSortProfile {
    pub pinned_keys: Vec<u32>,
    pub min_key: Option<u32>,
    pub max_normal_key: Option<u32>,
    pub live_notes: usize,
}
```

**What it does** — A compact summary of one notebook's live-note ordering,
computed natively by each backend (an indexed scan of sort keys — never the note
bodies): `pinned_keys` (keys currently used in the pinned band `1..=999`,
ascending), `min_key` (smallest effective key, `None` when the notebook has no
live notes), `max_normal_key` (largest key in the normal band `>= 1000`, `None`
when that band is empty), `live_notes` (how many live notes the notebook holds).
All keys are *effective* keys — the legacy `0` sentinel already mapped to
`Note::DEFAULT_SORT_KEY`.

`live_notes` exists so the notes-per-notebook cap (`format::MAX_NOTES_PER_NOTEBOOK`,
issue keeplin#130) can be enforced from the profile the placement path already
fetches, without a second query: `ordering::place_new_note` refuses a note once the
destination is full. It counts **live** notes only — both backends build the profile
from non-deleted rows — so tombstones never consume capacity.

**Dependencies** — none.

**Used by** — `NoteRepository::notebook_sort_profile`; `crate::ordering`'s
placement rules (new-note position, pin, unpin) and its notes-per-notebook cap.

**Repeated context** — the sort-key model: `1..=999` is the pinned band,
`>= 1000` the normal band; placement logic works from this profile so it stays
O(keys), not O(notes×bodies).

---

## impl NotebookSortProfile

**Identification** — inherent impl; marker `// md:impl NotebookSortProfile`. One
method.

**Code** — container: members documented as sub-blocks below: fn from_effective_keys.

### fn from_effective_keys

**Identification** —
`pub fn from_effective_keys(keys: impl IntoIterator<Item = u32>) -> Self`; marker
`// md:impl NotebookSortProfile > fn from_effective_keys`.

**Code** — complete and verbatim:

```rust
    // md:impl NotebookSortProfile > fn from_effective_keys
    pub fn from_effective_keys(keys: impl IntoIterator<Item = u32>) -> Self {
        let mut profile = Self::default();
        for key in keys {
            profile.live_notes += 1;
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
```

**What it does** — Builds a profile from an iterator of the notebook's live
effective sort keys: counts them into `live_notes`, tracks the global minimum,
routes `1..1000` keys into `pinned_keys` and everything else into the
`max_normal_key` maximum, then sorts `pinned_keys` ascending. Pure, total.
Because `live_notes` is incremented once per key, the caller's iterator defines
what "live" means — both backends already filter soft-deleted notes out, which is
the contract the notes-per-notebook cap depends on.

**Dependencies** — none.

**Used by** — the backends' `notebook_sort_profile` implementations (`fs.rs`,
`db.rs`); every field including `live_notes` is populated here, so a backend that
built a profile by hand would silently report a capacity of zero.

**Repeated context** — none.

---

## fn paginate_notes

**Identification** —
`fn paginate_notes(items: Vec<Note>, page_size: u32, token: Option<&str>) -> (Vec<Note>, Option<String>)`;
marker `// md:fn paginate_notes`.

**Code** — complete and verbatim:

```rust
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
```

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

**Code** — complete and verbatim:

```rust
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
```

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

**Code** — complete and verbatim:

```rust
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
```

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

**Code** — complete and verbatim:

```rust
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
```

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
- `list_resources_for_note(note_id, size, token)` — **additive default** method
  (issue #125): the attachments of one note, `created_at` order. The default impl
  exhausts `list_resources`, filters by `note_id`, and paginates in memory via
  `paginate_resources`; `fs`/`db` override it with a native filtered scan. Adding it
  as a defaulted trait method (rather than changing `list_resources`'s signature) is
  the trait's additive-evolution path — every existing impl keeps compiling. Known
  caveat: decorators (`EncryptedBackend`) inherit the O(N) default, exactly like
  `note_backlinks`; a follow-up may add native delegations.
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

## fn paginate_resources

**Identification** — private free function
`fn paginate_resources(items: Vec<Resource>, page_size: u32, token: Option<&str>) ->
(Vec<Resource>, Option<String>)`; marker `// md:fn paginate_resources`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — In-memory cursor pagination over an already-sorted `Vec<Resource>`, the
`Resource` twin of `paginate_notes`. The cursor is `"{sortable_rfc3339}|{uuid}"`; it
`partition_point`s to the first item strictly after the cursor (same-timestamp ties broken by
`id`), takes `effective_page_size` items, and emits a next-token only when more remain. Backs
the default `list_resources_for_note`, so callers get identical cursor semantics whether a
backend overrides that method natively or inherits the default.

**Dependencies** —
- `super::effective_page_size` — clamps `page_size` (0 ⇒ default); expects the same clamping
  the native backends use, so page sizes match across impls.
- `SortableRfc3339::to_sortable_rfc3339`, `Uuid::parse_str` — cursor encode/decode; expect the
  lexical RFC-3339 order to match chronological order.

**Used by** — `ResourceRepository::list_resources_for_note` (default impl).

**Repeated context** — pagination cursors are `(sortable-timestamp, id)` pairs throughout the
storage layer.

---

## trait SyncBackend

**Identification** — `#[async_trait] pub trait SyncBackend: Send + Sync + 'static`;
marker `// md:trait SyncBackend`.

**Code** — complete and verbatim:

```rust
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
```

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

**Code** — complete and verbatim:

```rust
// md:DEFAULT_HISTORY_LIMIT
pub const DEFAULT_HISTORY_LIMIT: u32 = 100;
```

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

**Code** — complete and verbatim:

```rust
// md:EntityVersion
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityVersion<T> {
    pub timestamp: DateTime<Utc>,
    pub device_id: String,
    pub entity: Option<T>,
}
```

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

**Code** — complete and verbatim:

```rust
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
```

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

**Code** — complete and verbatim:

```rust
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
```

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

**Code** — complete and verbatim:

```rust
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
```

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

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

<!-- Data source: CI artifact or local graphify-out/graph.json from this exact commit.
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. Never
     present inference as fact. -->
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
- `keeplin-core/src/storage/db/` — DbBackend's six trait implementations, one per module (EXTRACTED: edges to `notes.rs`, `notebooks.rs`, `tags.rs`, `resources.rs`, `sync.rs`, `server.rs` and `convert.rs`; e.g. `DbBackend`, `.notebook_sort_profile()`, `.entity_history()`)
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
| 9 | `trait ResourceRepository` (incl. default `list_resources_for_note`) | `// md:trait ResourceRepository` |
| 10 | `fn paginate_resources` | `// md:fn paginate_resources` |
| 11 | `trait SyncBackend` | `// md:trait SyncBackend` |
| 12 | `const DEFAULT_HISTORY_LIMIT` | `// md:DEFAULT_HISTORY_LIMIT` |
| 13 | `struct EntityVersion<T>` | `// md:EntityVersion` |
| 14 | `trait HistoryRepository` | `// md:trait HistoryRepository` |
| 15 | `trait StorageBackend` | `// md:trait StorageBackend` |
| 16 | blanket `impl StorageBackend for T` | `// md:impl StorageBackend for T` |
