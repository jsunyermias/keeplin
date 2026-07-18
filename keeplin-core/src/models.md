# `models.rs` — domain data types

Self-contained companion for `keeplin-core/src/models.rs`. It documents **every code
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
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::links::{Bookmark, NoteLink};
use crate::storage::note_log::VersionVector;
```

**What it does** — Every domain type the Keeplin data model is built on: `Note`,
`Notebook`, `Tag`, `NoteTag`, `Resource`, and the `Change` enum — the fundamental
unit of synchronisation (every mutation produces one `Change` replayable on another
device to reach the same state). All types derive
`serde::{Serialize, Deserialize}` so they persist as JSON (log files, SQLite TEXT
columns) and cross the network without a conversion layer.

**Dependencies** — `chrono`, `serde`, `uuid`, `crate::links::{Bookmark, NoteLink}`
(embedded in `Note`), `crate::storage::note_log::VersionVector`.

**Used by** — essentially the whole workspace: both backends, every decorator, the
sync engine, ordering, linking, interop, history, and the daemon's REST/gRPC
layers; keeplin-srv pins this crate largely for `Change` and the resolution types.

**Repeated context** — The conflict-resolution contract every backend relies on:
every entity carries `vv` (per-device version vector), `last_writer` (authoring
device id), `updated_at`, and soft-delete via `deleted_at`. Resolution is version
vectors first; a genuine concurrency resolves by the deterministic
`(timestamp, device_id)` LWW tiebreak (`storage::note_log::resolve`).

---

## fn new_id

**Identification** — `pub fn new_id() -> Uuid`; marker `// md:fn new_id`.

**What it does** — Generates a random UUID v4. All entity constructors call it;
callers must never generate ids themselves, keeping id generation consistent and
testable.

**Dependencies** — `uuid`.

**Used by** — `Note::new`, `Notebook::new`, `Tag::new`, `Resource::new`; various
callers across the daemon and tests.

**Repeated context** — device-generated UUIDs (no server-assigned ids) are what
let offline devices create entities without coordination.

---

## fn now

**Identification** — `pub fn now() -> DateTime<Utc>`; marker `// md:fn now`.

**What it does** — The current UTC timestamp. Used by constructors for
`created_at`/`updated_at`, by the sync engine for the cycle watermark, and as the
`#[serde(default = "now")]` fallback on old delete records.

**Dependencies** — `chrono`.

**Used by** — constructors here; `sync/engine.rs`; `history.rs`; `ordering.rs`;
serde defaults in `Change`.

**Repeated context** — all timestamps in the project are UTC.

---

## Note

**Identification** — struct deriving `Debug, Clone, Serialize, Deserialize,
PartialEq, Eq, Hash`; marker `// md:Note`.

**What it does** — A user-created note, the primary content unit. Fields:

| Field | Type | Contract |
|-------|------|----------|
| `id` | `Uuid` | stable UUID v4, generated once, never changed |
| `title` / `body` | `String` | user content; **encrypted at rest** under `EncryptedBackend` |
| `notebook_id` | `Uuid` | parent notebook — **never "none"**: unfiled notes belong to the Inbox system notebook (nil UUID, `crate::ordering::INBOX_ID`). `#[serde(default = "Uuid::nil", deserialize_with = "de_notebook_id")]` maps old `null`/missing values to the Inbox. Plaintext (needed for queries) |
| `is_todo` / `todo_due` / `todo_completed` | `bool` / `Option<DateTime>` ×2 | to-do flag, optional deadline, completion time (`None` while open) |
| `created_at` | `DateTime<Utc>` | set once; never modified |
| `updated_at` | `DateTime<Utc>` | refreshed on every mutation; the LWW tiebreak input |
| `deleted_at` | `Option<DateTime<Utc>>` | soft-delete tombstone; `None` = active. Deleted notes are excluded from `list_notes` but readable by id (needed for conflict resolution) |
| `alias` | `Option<String>` | human alias, unique among live notes **in the same notebook** (may recur elsewhere); lets links target `#<alias>`. Inbox notes carry no alias and are never link targets. Encrypted at rest. `#[serde(default)]` |
| `bookmarks` | `Vec<Bookmark>` | in-note anchors derived from `[text](### "alias")` in the body; maintained by `LinkingBackend`. `#[serde(default)]` |
| `links` | `Vec<NoteLink>` | content-derived + manual links; maintained by `LinkingBackend`. `#[serde(default)]` |
| `vv` | `VersionVector` | per-device edit counters; a local write increments this device's component. Plaintext sync metadata (not encrypted). `#[serde(default)]` — empty ⇒ pre-VV record |
| `last_writer` | `String` | device id that authored the current value; tiebreak partner of `updated_at`. Plaintext. `#[serde(default)]` |
| `is_pinned` | `bool` | pinned to the top of its notebook (`sort_key` in `1..=999`); the Inbox has no pinning. Plaintext. `#[serde(default)]` |
| `is_starred` | `bool` | globally starred; orthogonal to pinning/notebook — never moves the note. Plaintext. `#[serde(default)]` |
| `sort_key` | `u32` | manual position, ascending; `1..=999` pinned band, `>= 1000` normal band, the Inbox one flat band. `0` = legacy "never positioned" sentinel. `#[serde(default)]` |

**Dependencies** — `Bookmark`, `NoteLink`, `VersionVector`, `chrono`, `uuid`.

**Used by** — everywhere (140 cross-file edges — the most-referenced type in the
graph).

**Repeated context** — at-rest encryption covers human-readable content
(`title`, `body`, `alias`) but leaves ids, flags, keys, and sync metadata
plaintext so queries and resolution work on ciphertext stores. The ordering
fields' placement rules live in `crate::ordering`.

---

## fn de_notebook_id

**Identification** — the custom deserializer
`fn de_notebook_id<'de, D>(deserializer: D) -> Result<Uuid, D::Error>`; marker
`// md:fn de_notebook_id`.

**What it does** — Backward-compatible `notebook_id` parsing: records written
when the field was `Option<Uuid>` carry an explicit `null` (older ones may omit
it entirely); both must land in the Inbox (nil UUID) rather than fail. Reads an
`Option<Uuid>` and unwraps to `Uuid::nil()`.

**Dependencies** — `serde`.

**Used by** — the `notebook_id` field attribute on `Note`; tests
`pre_ordering_note_json_lands_in_the_inbox_with_defaults` /
`pre_ordering_note_msgpack_round_trips`.

**Repeated context** — "a note is always in exactly one notebook" is a data
invariant introduced after v1; this deserializer is what upgrades old records
transparently, with no migration pass.

---

## impl Note

**Identification** — inherent impl; marker `// md:impl Note`. One const + two
methods.

### DEFAULT_SORT_KEY

**Identification** — `pub const DEFAULT_SORT_KEY: u32 = 1000;` marker
`// md:impl Note > DEFAULT_SORT_KEY`.

**What it does** — The sort key a never-positioned note (`sort_key == 0`) is
ordered as: the start of the normal (unpinned) band. Old notes appear at the top
of the normal band, tie-broken by id, without any data rewrite.

**Used by** — `effective_sort_key`; `ordering.rs`; `NotebookSortProfile` docs.

### fn new

**Identification** — `pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self`;
marker `// md:impl Note > fn new`.

**What it does** — Fresh UUID, given title/body, one shared `now()` for
`created_at` and `updated_at`; everything else `None`/`false`/empty — the note
starts in the Inbox (nil notebook id) with no manual position (`sort_key 0`).

**Dependencies** — `new_id`, `now`.

**Used by** — the daemon's create paths, tests, `interop.rs`.

**Repeated context** — none.

### fn effective_sort_key

**Identification** — `pub fn effective_sort_key(&self) -> u32`; marker
`// md:impl Note > fn effective_sort_key`.

**What it does** — The key the note actually sorts by: `sort_key`, with the
legacy `0` sentinel mapped to `DEFAULT_SORT_KEY` so never-positioned notes order
at the start of the normal band instead of above pinned notes.

**Dependencies** — `DEFAULT_SORT_KEY`.

**Used by** — the backends' `list_notes_in_notebook` ordering and
`notebook_sort_profile`; `ordering.rs` placement rules.

**Repeated context** — none.

---

## Notebook

**Identification** — struct deriving `Debug, Clone, Serialize, Deserialize,
PartialEq, Eq, Hash`; marker `// md:Notebook`.

**What it does** — A named collection grouping notes: `id`, `title` (encrypted at
rest), `created_at`, `updated_at`, `deleted_at` (soft delete), `alias`
(`#[serde(default)]` — human alias, unique among **live notebooks**, lets links
scope `#<notebook alias>#<note>`; encrypted at rest), `vv` and `last_writer`
(`#[serde(default)]`, same resolution contract as `Note`).

**Dependencies** — `VersionVector`, `chrono`, `uuid`, `serde`.

**Used by** — the notebook CRUD surface, ordering (the Inbox is a `Notebook` with
the nil UUID), linking (alias scope), history.

**Repeated context** — soft-delete-always; alias uniqueness enforced by
`LinkingBackend` with `StorageError::Conflict` on duplicates.

---

## impl Notebook

**Identification** — inherent impl; marker `// md:impl Notebook`. One method.

### fn new

**Identification** — `pub fn new(title: impl Into<String>) -> Self`; marker
`// md:impl Notebook > fn new`.

**What it does** — Fresh UUID, given title, one shared `now()` for both
timestamps; `deleted_at`/`alias` `None`, empty `vv`/`last_writer`.

**Used by** — the daemon's create path; `ordering::ensure_inbox` builds the Inbox
differently (fixed nil UUID), not through this constructor.

---

## Tag

**Identification** — struct deriving `Debug, Clone, Serialize, Deserialize,
PartialEq, Eq, Hash`; marker `// md:Tag`.

**What it does** — A short label attachable to any number of notes: `id`,
`title` (encrypted at rest), the three timestamps (`deleted_at` = soft delete),
`vv`/`last_writer` (`#[serde(default)]`). Associations live in the separate
`NoteTag` type.

**Dependencies** — `VersionVector`, `chrono`, `uuid`, `serde`.

**Used by** — tag CRUD, `TagRepository`, `migrate.rs`, search indexing.

**Repeated context** — none beyond the shared entity contract.

---

## impl Tag

**Identification** — inherent impl; marker `// md:impl Tag`. One method.

### fn new

**Identification** — `pub fn new(title: impl Into<String>) -> Self`; marker
`// md:impl Tag > fn new`.

**What it does** — Fresh UUID, given title, shared `now()`, `deleted_at: None`,
empty `vv`/`last_writer`.

---

## NoteTag

**Identification** — struct deriving `Debug, Clone, Serialize, Deserialize,
PartialEq, Eq, Hash`; marker `// md:NoteTag`.

**What it does** — A many-to-many association between one note (`note_id`) and
one tag (`tag_id`). Created by `StorageBackend::add_note_tag`, removed by
`remove_note_tag`. The struct itself has no version fields — the *association
state* is versioned in the change journal (`Change::NoteTagAdd`/`NoteTagRemove`
carry `vv`/`updated_at`/`last_writer`), and the backends store that state
natively.

**Dependencies** — `uuid`, `serde`.

**Used by** — `TagRepository::add_note_tag`; `migrate.rs`.

**Repeated context** — none.

---

## Resource

**Identification** — struct deriving `Debug, Clone, Serialize, Deserialize,
PartialEq, Eq, Hash`; marker `// md:Resource`.

**What it does** — Metadata for a binary file attachment; the payload is stored
separately (a `data` file on disk for `FsBackend`, a BLOB column for
`DbBackend`) and fetched explicitly via `read_resource`. Fields: `id`, `title` /
`mime_type` (IANA, e.g. `"image/png"`) / `file_name` — all encrypted at rest;
`size` in **plaintext** (needed to validate uploads without decrypting);
`created_at`; `deleted_at` (`#[serde(default)]`) — resources use a soft-delete
tombstone rather than physical removal so a delete converges with a concurrent
create through `note_log::resolve`, with the payload retained on delete
(reclaiming it is a separate compaction concern — `purge_deleted_resources`);
`vv`/`last_writer` (`#[serde(default)]`).

**Dependencies** — `VersionVector`, `chrono`, `uuid`, `serde`.

**Used by** — `ResourceRepository`; `interop.rs` (contacts/events are stored as
resources); `migrate.rs`; the daemon's resource endpoints.

**Repeated context** — none beyond the shared entity contract.

---

## impl Resource

**Identification** — inherent impl; marker `// md:impl Resource`. One method.

### fn new

**Identification** —
`pub fn new(title, mime_type, file_name, size: u64) -> Self` (the string
parameters are `impl Into<String>`); marker `// md:impl Resource > fn new`.

**What it does** — Fresh UUID + `now()`; the binary payload is **not** stored
here — it is passed separately to `create_resource`. `size` must be the exact
byte length of that payload.

---

## Change

**Identification** — enum deriving `Debug, Clone, Serialize, Deserialize,
PartialEq, Eq, Hash` with `#[serde(tag = "op", rename_all = "snake_case")]`;
marker `// md:Change`.

**What it does** — One unit of change applied to a local store to converge with
another device. Every mutating backend operation appends one to the change
journal (`entity_changes` table in `DbBackend`, NDJSON log in `FsBackend`); sync
sends/receives them and applies each via the idempotent `apply_change`.
Serialised as `{"op":"note_create","note":{…}}` — the `op` tag carries the
variant in snake_case. Variants:

| Variant | Payload | Notes |
|---------|---------|-------|
| `NoteCreate { note }` | full snapshot | `#[serde(alias = "create")]` reads v1 logs |
| `NoteUpdate { note }` | full snapshot | `#[serde(alias = "update")]` |
| `NoteDelete { id, deleted_at, vv, last_writer }` | tombstone | `#[serde(alias = "delete")]`; `deleted_at` defaults to `now()`, `vv`/`last_writer` default empty for old records. Carrying the deleting write's vv/author makes the delete **compete in `resolve` exactly like an edit**: a stale edit can't resurrect a newer delete, a stale delete can't override a newer edit, and a causal edit made after seeing the delete revives the note |
| `NotebookCreate` / `NotebookUpdate` | full snapshot | |
| `NotebookDelete { id, deleted_at, vv, last_writer }` | tombstone | same semantics/defaults as `NoteDelete` |
| `TagCreate` / `TagUpdate` | full snapshot | |
| `TagDelete { id, deleted_at, vv, last_writer }` | tombstone | same semantics/defaults |
| `NoteTagAdd { note_id, tag_id, updated_at, vv, last_writer }` | association present | the association is a versioned present/absent state; version fields `#[serde(default)]` (empty vv ⇒ uninformed write) so pre-version records parse; concurrent add-vs-remove converges through `resolve` like a note edit |
| `NoteTagRemove { … }` | association absent (tombstone) | same version metadata, so add and remove compete deterministically |
| `ResourceCreate { resource, data }` | metadata + optional bytes | `data: Option<Vec<u8>>` with `#[serde(skip_serializing_if = "Option::is_none", default)]`: carries the payload through `DbBackend` (no shared filesystem), is always `None` on `FsBackend` (Syncthing replicates `resources/{id}/data`), and stays absent from v1 JSON |
| `ResourceDelete { id, deleted_at, vv, last_writer }` | tombstone | payload retained; reclaim is separate compaction |

**Dependencies** — every entity type above, `VersionVector`, `serde`.

**Used by** — both backends' journals and `apply_change`; `sync/engine.rs`;
`keeplin-daemon/src/event_backend.rs` (publishes applied changes);
`keeplin-daemon/src/search.rs` (re-indexes on changes); keeplin-srv's relay
stores these verbatim.

**Repeated context** — **additive evolution only**: renaming variants or fields
breaks stored journals and the server relay; v1 aliases are kept so old logs
read without migration (clean-break premise applies to *wire/protocol* versions,
while stored journals stay readable via serde defaults/aliases). Full snapshots
per change are what make journal-derived history possible.

---

## mod tests

**Identification** — `#[cfg(test)]` unit-test module; marker `// md:mod tests`.
Two tests.

**What it does** — Pins the backward-compatible deserialisation of pre-ordering
note records in both encodings (JSON and MessagePack).

**Dependencies** — `super::*`, `serde_json`, `rmp_serde`.

**Used by** — CI (`cargo test --workspace`).

**Repeated context** — MessagePack (`rmp_serde`) is the on-disk note-log
encoding; JSON is used for the global journal and network.

### fn pre_ordering_note_json_lands_in_the_inbox_with_defaults

**Identification** — unit test; marker
`// md:mod tests > fn pre_ordering_note_json_lands_in_the_inbox_with_defaults`.

**What it does** — A note serialized before pinning/starring/ordering existed
(`notebook_id: null`, new fields absent) must load with the defaults: Inbox (nil
UUID), unpinned, unstarred, `sort_key 0`, and `effective_sort_key()` mapping to
`DEFAULT_SORT_KEY`.

### fn pre_ordering_note_msgpack_round_trips

**Identification** — unit test; marker
`// md:mod tests > fn pre_ordering_note_msgpack_round_trips`.

**What it does** — Serialises an old-shape mirror struct (an in-test `OldNote`
with `notebook_id: Option<Uuid>` and no ordering fields) with
`rmp_serde::to_vec_named` and deserialises it as today's `Note`: nil notebook id
and `sort_key 0`.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `Note` — defined here (EXTRACTED; 140 cross-file edge(s))
- `Notebook` — defined here (EXTRACTED; 58 cross-file edge(s))
- `Tag` — defined here (EXTRACTED; 45 cross-file edge(s))
- `Change` — defined here (EXTRACTED; 44 cross-file edge(s))
- `Resource` — defined here (EXTRACTED; 34 cross-file edge(s))
- `NoteTag` — defined here (EXTRACTED; 7 cross-file edge(s))
- `new_id()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `now()` — defined here (EXTRACTED; file-local)
- `de_notebook_id()` — defined here (EXTRACTED; file-local)
- `.new()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/links.rs` — bookmark & link types and pure parsing (EXTRACTED: references×2; e.g. `Bookmark`, `NoteLink`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/collab/mod.rs` — client of the keeplin-srv collaborative channel (EXTRACTED: references×31; e.g. `.apply_from_server()`, `.push_local_edit()`, `.patch_meta()`)
- `keeplin-core/src/encryption.rs` — transparent at-rest encryption (EXTRACTED: references×34; e.g. `.enc_note()`, `.dec_note()`, `.enc_notebook()`)
- `keeplin-core/src/history.rs` — change history reads + forward-revert (EXTRACTED: references×4; e.g. `revert_note()`, `revert_notebook()`, `revert_notes_to()`)
- `keeplin-core/src/interop.rs` — vCard & iCalendar format compatibility (EXTRACTED: calls×2, imports_from×1, references×9; e.g. `interop.rs`, `.from_note()`, `.apply_to_note()`)
- `keeplin-core/src/linking.rs` — `LinkingBackend` decorator + reference resolution (EXTRACTED: references×52; e.g. `AliasConflicts`, `.from_snapshots()`, `.upsert_note()`)
- `keeplin-core/src/ordering.rs` — the Inbox, pinning, manual ordering, and starring (EXTRACTED: references×12; e.g. `create_placed()`, `move_note()`, `pin_note()`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: references×1; e.g. `paginate_notes()`)
- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: calls×2, references×32; e.g. `.get_or_create_device_id()`, `.send_changes()`, `.create_note()`)
- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: calls×1, references×37; e.g. `.read_or_create_device_id()`, `.append_note_op()`, `.create_note()`)
- `keeplin-core/src/storage/note_log.rs` — version-vector resolution (EXTRACTED: imports_from×1, references×2; e.g. `Merged`, `NoteOp`)
- `keeplin-core/src/sync/engine.rs` — SyncEngine (EXTRACTED: references×2; e.g. `run_sync()`, `.sync()`)
- `keeplin-daemon/src/event_backend.rs` — `EventBackend` change-publishing decorator (EXTRACTED: references×30; e.g. `.create_note()`, `.list_notes()`, `.list_notes_in_notebook()`)
- `keeplin-daemon/src/metrics.rs` — operational metrics (EXTRACTED: references×26; e.g. `.create_note()`, `.list_notes()`, `.list_notes_in_notebook()`)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: references×42; e.g. `add_link()`, `batch_revert_notes_ep()`, `create_note()`)
- `keeplin-daemon/src/search.rs` — daemon full-text search (EXTRACTED: references×5; e.g. `denormalize()`, `index_note()`, `.upsert()`)
- `keeplin-daemon/src/server.rs` — gRPC service implementation (EXTRACTED: references×5; e.g. `note_to_proto()`, `proto_to_note()`, `notebook_to_proto()`)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `fn new_id` | `// md:fn new_id` |
| 3 | `fn now` | `// md:fn now` |
| 4 | `struct Note` | `// md:Note` |
| 5 | `fn de_notebook_id` | `// md:fn de_notebook_id` |
| 6 | `impl Note` | `// md:impl Note` |
| 7 | `const DEFAULT_SORT_KEY` | `// md:impl Note > DEFAULT_SORT_KEY` |
| 8 | `fn new` (Note) | `// md:impl Note > fn new` |
| 9 | `fn effective_sort_key` | `// md:impl Note > fn effective_sort_key` |
| 10 | `struct Notebook` | `// md:Notebook` |
| 11 | `impl Notebook` | `// md:impl Notebook` |
| 12 | `fn new` (Notebook) | `// md:impl Notebook > fn new` |
| 13 | `struct Tag` | `// md:Tag` |
| 14 | `impl Tag` | `// md:impl Tag` |
| 15 | `fn new` (Tag) | `// md:impl Tag > fn new` |
| 16 | `struct NoteTag` | `// md:NoteTag` |
| 17 | `struct Resource` | `// md:Resource` |
| 18 | `impl Resource` | `// md:impl Resource` |
| 19 | `fn new` (Resource) | `// md:impl Resource > fn new` |
| 20 | `enum Change` | `// md:Change` |
| 21 | `mod tests` | `// md:mod tests` |
| 22 | `fn pre_ordering_note_json_lands_in_the_inbox_with_defaults` | `// md:mod tests > fn pre_ordering_note_json_lands_in_the_inbox_with_defaults` |
| 23 | `fn pre_ordering_note_msgpack_round_trips` | `// md:mod tests > fn pre_ordering_note_msgpack_round_trips` |
