# `models.rs` — domain data types

Self-contained companion for `keeplin-core/src/models.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this
file must be able to understand and modify the module without opening anything else, so
project-wide conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in the `.rs` carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section here;
grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::links::{Bookmark, NoteLink};
use crate::storage::note_log::VersionVector;
```

**What it does** — Every domain type the Keeplin data model is built on: `Note`, `Notebook`,
`Tag`, `NoteTag`, `Resource`, and the `Change` enum — the fundamental unit of synchronisation
(every mutation produces one `Change` replayable on another device to reach the same state).
All types derive `serde::{Serialize, Deserialize}` so they persist as JSON (log files, SQLite
TEXT columns) and cross the network without a conversion layer.

**Dependencies** —
- `chrono::{DateTime, Utc}` — timestamp type for every entity's `created_at`/`updated_at`/
  `deleted_at`; expects `Utc` to stay the only timezone used (the whole project assumes UTC).
- `serde::{Deserialize, Serialize}` — derived on every type here; expects the derive to keep
  round-tripping field names as-is (JSON logs + SQLite TEXT + wire depend on stable names).
- `uuid::Uuid` — entity ids; expects `Uuid::nil()` to stay the sentinel used for the Inbox.
- `crate::links::{Bookmark, NoteLink}` — embedded in `Note` (`bookmarks`/`links`); expects
  both to stay `serde`-serializable and `#[serde(default)]`-friendly so pre-link records load.
- `crate::storage::note_log::VersionVector` — the `vv` field on every entity; expects it to
  keep `Default` (empty ⇒ pre-VV record) and to be the type `resolve` compares.

**Used by** — essentially the whole workspace: both backends, every decorator, the sync
engine, ordering, linking, interop, history, and the daemon's REST/gRPC layers; keeplin-srv
pins this crate largely for `Change` and the resolution types.

**Repeated context** — The conflict-resolution contract every backend relies on: every entity
carries `vv` (per-device version vector), `last_writer` (authoring device id), `updated_at`,
and soft-delete via `deleted_at`. Resolution is version vectors first; a genuine concurrency
resolves by the deterministic `(timestamp, device_id)` LWW tiebreak
(`storage::note_log::resolve`).

---

## fn new_id

**Identification** — `pub fn new_id() -> Uuid`; marker `// md:fn new_id`.

**Code** — complete and verbatim:

```rust
// md:fn new_id
pub fn new_id() -> Uuid {
    Uuid::new_v4()
}
```

**What it does** — Generates a random UUID v4. All entity constructors call it; callers must
never generate ids themselves, keeping id generation consistent and testable.

**Dependencies** —
- `Uuid::new_v4` — the id source; expects it to stay collision-free-in-practice (random v4)
  so offline devices can mint ids without coordination. If it ever became sequential/derived,
  the "no server-assigned ids" invariant would break silently.

**Used by** — `Note::new`, `Notebook::new`, `Tag::new`, `Resource::new`; various callers
across the daemon and tests.

**Repeated context** — device-generated UUIDs (no server-assigned ids) are what let offline
devices create entities without coordination.

---

## fn now

**Identification** — `pub fn now() -> DateTime<Utc>`; marker `// md:fn now`.

**Code** — complete and verbatim:

```rust
// md:fn now
pub fn now() -> DateTime<Utc> {
    Utc::now()
}
```

**What it does** — The current UTC timestamp. Used by constructors for
`created_at`/`updated_at`, by the sync engine for the cycle watermark, and as the
`#[serde(default = "now")]` fallback on old delete records.

**Dependencies** —
- `chrono::Utc::now` — the clock; expects it to return UTC. It is also referenced by name in
  `#[serde(default = "now")]` on `Change` delete variants — renaming this fn silently breaks
  those serde defaults (a compile error there, but easy to miss the coupling).

**Used by** — constructors here; `sync/engine.rs`; `history.rs`; `ordering.rs`; serde defaults
in `Change`.

**Repeated context** — all timestamps in the project are UTC.

---

## Note

**Identification** — struct deriving `Debug, Clone, Serialize, Deserialize, PartialEq, Eq,
Hash`; marker `// md:Note`.

**Code** — complete and verbatim:

```rust
// md:Note
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Note {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    #[serde(default = "Uuid::nil", deserialize_with = "de_notebook_id")]
    pub notebook_id: Uuid,
    pub is_todo: bool,
    pub todo_due: Option<DateTime<Utc>>,
    pub todo_completed: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    #[serde(default)]
    pub links: Vec<NoteLink>,
    #[serde(default)]
    pub vv: VersionVector,
    #[serde(default)]
    pub last_writer: String,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub is_starred: bool,
    #[serde(default)]
    pub sort_key: u32,
}
```

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

**Dependencies** —
- `Bookmark`, `NoteLink` (`crate::links`) — the `bookmarks`/`links` vecs; expect both to stay
  `#[serde(default)]`-loadable so pre-link JSON parses (missing ⇒ empty vec).
- `VersionVector` — the `vv` field; expects `Default` for pre-VV records.
- `chrono::DateTime<Utc>` / `uuid::Uuid` — id + timestamps; expect the UTC + v4 conventions.
- `de_notebook_id` (below) — named in the `notebook_id` attribute; expects that fn to keep
  mapping `null`/missing → `Uuid::nil()`. Renaming it breaks the `deserialize_with` path.

**Used by** — everywhere (140 cross-file edges — the most-referenced type in the graph).

**Repeated context** — at-rest encryption covers human-readable content (`title`, `body`,
`alias`) but leaves ids, flags, keys, and sync metadata plaintext so queries and resolution
work on ciphertext stores. The ordering fields' placement rules live in `crate::ordering`.

---

## fn de_notebook_id

**Identification** — the custom deserializer
`fn de_notebook_id<'de, D>(deserializer: D) -> Result<Uuid, D::Error>`; marker
`// md:fn de_notebook_id`.

**Code** — complete and verbatim:

```rust
// md:fn de_notebook_id
fn de_notebook_id<'de, D>(deserializer: D) -> Result<Uuid, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<Uuid>::deserialize(deserializer)?.unwrap_or_else(Uuid::nil))
}
```

**What it does** — Backward-compatible `notebook_id` parsing: records written when the field
was `Option<Uuid>` carry an explicit `null` (older ones may omit it entirely); both must land
in the Inbox (nil UUID) rather than fail. Reads an `Option<Uuid>` and unwraps to
`Uuid::nil()`.

**Dependencies** —
- `Option::<Uuid>::deserialize` — reads the legacy shape; expects serde to still accept `null`
  and a bare UUID string for the same field. If a value is present it is used verbatim; only
  `null`/absent collapse to nil.
- `Uuid::nil` — the Inbox sentinel; expects `ordering::INBOX_ID` to remain the nil UUID (they
  must agree, or old notes land somewhere that isn't the Inbox).

**Used by** — the `notebook_id` field attribute on `Note`; tests
`pre_ordering_note_json_lands_in_the_inbox_with_defaults` /
`pre_ordering_note_msgpack_round_trips`.

**Repeated context** — "a note is always in exactly one notebook" is a data invariant
introduced after v1; this deserializer is what upgrades old records transparently, with no
migration pass.

---

## impl Note

**Identification** — inherent impl; marker `// md:impl Note`. One const + two methods.

**Code** — container: members documented as sub-blocks below: `DEFAULT_SORT_KEY`, `fn new`,
`fn effective_sort_key`.

### DEFAULT_SORT_KEY

**Identification** — `pub const DEFAULT_SORT_KEY: u32 = 1000;` marker
`// md:impl Note > DEFAULT_SORT_KEY`.

**Code** — complete and verbatim:

```rust
    // md:impl Note > DEFAULT_SORT_KEY
    pub const DEFAULT_SORT_KEY: u32 = 1000;
```

**What it does** — The sort key a never-positioned note (`sort_key == 0`) is ordered as: the
start of the normal (unpinned) band. Old notes appear at the top of the normal band, tie-broken
by id, without any data rewrite.

**Dependencies** — none (a plain constant).

**Used by** — `effective_sort_key`; `ordering.rs`; `NotebookSortProfile` docs.

**Repeated context** — the pinned band is `1..=999`, so `1000` is the first normal-band key;
changing this constant reshuffles every legacy `sort_key == 0` note.

### fn new

**Identification** — `pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self`;
marker `// md:impl Note > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl Note > fn new
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        let ts = now();
        Self {
            id: new_id(),
            title: title.into(),
            body: body.into(),
            notebook_id: Uuid::nil(),
            is_todo: false,
            todo_due: None,
            todo_completed: None,
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
            alias: None,
            bookmarks: Vec::new(),
            links: Vec::new(),
            vv: VersionVector::new(),
            last_writer: String::new(),
            is_pinned: false,
            is_starred: false,
            sort_key: 0,
        }
    }
```

**What it does** — Fresh UUID, given title/body, one shared `now()` for `created_at` and
`updated_at`; everything else `None`/`false`/empty — the note starts in the Inbox (nil notebook
id) with no manual position (`sort_key 0`).

**Dependencies** —
- `new_id` — the id; expects a fresh v4 per call.
- `now` — both timestamps; expects a single call so `created_at == updated_at` on a new note.
- `VersionVector::new` — empty vv; expects "empty" to mean "no edits recorded yet".

**Used by** — the daemon's create paths, tests, `interop.rs`.

**Repeated context** — none.

### fn effective_sort_key

**Identification** — `pub fn effective_sort_key(&self) -> u32`; marker
`// md:impl Note > fn effective_sort_key`.

**Code** — complete and verbatim:

```rust
    // md:impl Note > fn effective_sort_key
    pub fn effective_sort_key(&self) -> u32 {
        if self.sort_key == 0 {
            Self::DEFAULT_SORT_KEY
        } else {
            self.sort_key
        }
    }
```

**What it does** — The key the note actually sorts by: `sort_key`, with the legacy `0` sentinel
mapped to `DEFAULT_SORT_KEY` so never-positioned notes order at the start of the normal band
instead of above pinned notes.

**Dependencies** —
- `Self::DEFAULT_SORT_KEY` — the fallback for the `0` sentinel; expects it to stay in the
  normal band (`>= 1000`) so legacy notes never sort into the pinned band.

**Used by** — the backends' `list_notes_in_notebook` ordering and `notebook_sort_profile`;
`ordering.rs` placement rules.

**Repeated context** — none.

---

## Notebook

**Identification** — struct deriving `Debug, Clone, Serialize, Deserialize, PartialEq, Eq,
Hash`; marker `// md:Notebook`.

**Code** — complete and verbatim:

```rust
// md:Notebook
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Notebook {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub vv: VersionVector,
    #[serde(default)]
    pub last_writer: String,
}
```

**What it does** — A named collection grouping notes: `id`, `title` (encrypted at rest),
`created_at`, `updated_at`, `deleted_at` (soft delete), `alias` (`#[serde(default)]` — human
alias, unique among **live notebooks**, lets links scope `#<notebook alias>#<note>`; encrypted
at rest), `vv` and `last_writer` (`#[serde(default)]`, same resolution contract as `Note`).

**Dependencies** —
- `VersionVector` — the `vv` field; expects `Default` for pre-VV records.
- `chrono::DateTime<Utc>`, `uuid::Uuid`, `serde` derive — id/timestamps/serialisation; expect
  the shared UTC + stable-field-name conventions.

**Used by** — the notebook CRUD surface, ordering (the Inbox is a `Notebook` with the nil
UUID), linking (alias scope), history.

**Repeated context** — soft-delete-always; alias uniqueness enforced by `LinkingBackend` with
`StorageError::Conflict` on duplicates.

---

## impl Notebook

**Identification** — inherent impl; marker `// md:impl Notebook`. One method.

**Code** — container: members documented as sub-blocks below: `fn new`.

### fn new

**Identification** — `pub fn new(title: impl Into<String>) -> Self`; marker
`// md:impl Notebook > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl Notebook > fn new
    pub fn new(title: impl Into<String>) -> Self {
        let ts = now();
        Self {
            id: new_id(),
            title: title.into(),
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
            alias: None,
            vv: VersionVector::new(),
            last_writer: String::new(),
        }
    }
```

**What it does** — Fresh UUID, given title, one shared `now()` for both timestamps;
`deleted_at`/`alias` `None`, empty `vv`/`last_writer`.

**Dependencies** —
- `new_id`, `now`, `VersionVector::new` — id/timestamps/empty vv; same expectations as
  `Note::new`.

**Used by** — the daemon's create path; `ordering::ensure_inbox` builds the Inbox differently
(fixed nil UUID), not through this constructor.

**Repeated context** — none.

---

## Tag

**Identification** — struct deriving `Debug, Clone, Serialize, Deserialize, PartialEq, Eq,
Hash`; marker `// md:Tag`.

**Code** — complete and verbatim:

```rust
// md:Tag
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Tag {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub vv: VersionVector,
    #[serde(default)]
    pub last_writer: String,
    #[serde(default)]
    pub system: bool,
}
```

**What it does** — A short label attachable to any number of notes: `id`, `title` (encrypted
at rest), the three timestamps (`deleted_at` = soft delete), `vv`/`last_writer`
(`#[serde(default)]`). `system` (`#[serde(default)]` = `false`) is an internal-function
marker the frontend sets on tags it uses to implement features (hidden from the user); the
backend only transports and persists it — it never interprets the tag's `title` pattern
(which arrives encrypted) nor filters tags by this flag. Associations live in the separate
`NoteTag` type.

**Dependencies** —
- `VersionVector` — `vv`; expects `Default`.
- `chrono`, `uuid`, `serde` derive — same shared conventions as the other entities.

**Used by** — tag CRUD, `TagRepository`, `migrate.rs`, search indexing.

**Repeated context** — none beyond the shared entity contract.

---

## impl Tag

**Identification** — inherent impl; marker `// md:impl Tag`. One method.

**Code** — container: members documented as sub-blocks below: `fn new`.

### fn new

**Identification** — `pub fn new(title: impl Into<String>) -> Self`; marker
`// md:impl Tag > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl Tag > fn new
    pub fn new(title: impl Into<String>) -> Self {
        let ts = now();
        Self {
            id: new_id(),
            title: title.into(),
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
            vv: VersionVector::new(),
            last_writer: String::new(),
            system: false,
        }
    }
```

**What it does** — Fresh UUID, given title, shared `now()`, `deleted_at: None`, empty
`vv`/`last_writer`, `system: false` (a plain user tag; the frontend flips `system` on tags
it uses internally).

**Dependencies** —
- `new_id`, `now`, `VersionVector::new` — id/timestamps/empty vv; same expectations as
  `Note::new`.

**Used by** — tag create paths; `migrate.rs`; tests.

**Repeated context** — none.

---

## NoteTag

**Identification** — struct deriving `Debug, Clone, Serialize, Deserialize, PartialEq, Eq,
Hash`; marker `// md:NoteTag`.

**Code** — complete and verbatim:

```rust
// md:NoteTag
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct NoteTag {
    pub note_id: Uuid,
    pub tag_id: Uuid,
}
```

**What it does** — A many-to-many association between one note (`note_id`) and one tag
(`tag_id`). Created by `StorageBackend::add_note_tag`, removed by `remove_note_tag`. The struct
itself has no version fields — the *association state* is versioned in the change journal
(`Change::NoteTagAdd`/`NoteTagRemove` carry `vv`/`updated_at`/`last_writer`), and the backends
store that state natively.

**Dependencies** —
- `uuid::Uuid`, `serde` derive — the two id fields; expect stable field names so stored rows
  and wire messages keep matching.

**Used by** — `TagRepository::add_note_tag`; `migrate.rs`.

**Repeated context** — none.

---

## SYSTEM_RESOURCE_NOTE_ID

**Identification** — module-level `pub const` of type `Uuid`; marker
`// md:SYSTEM_RESOURCE_NOTE_ID`.

**Code** — complete and verbatim:

```rust
// md:SYSTEM_RESOURCE_NOTE_ID
pub const SYSTEM_RESOURCE_NOTE_ID: Uuid = Uuid::from_u128(1);
```

**What it does** — The reserved **non-nil** sentinel `note_id` for a "system resource" — an
attachment that does not hang off a user note (vCard contacts and iCal events produced by
`interop.rs`). `Uuid::nil()` is already the Inbox notebook id (`ordering::INBOX_ID`), so the
sentinel is deliberately `Uuid::from_u128(1)` (`00000000-0000-0000-0000-000000000001`) to avoid
reusing it. Per-note listings filter by a real note id, which never equals this value, so system
resources are naturally excluded from them. It is also the `#[serde(default)]` for
`Resource.note_id`, so a pre-#125 record with no `note_id` deserialises as a system resource.

**Dependencies** —
- `uuid::Uuid::from_u128` — const constructor; expects a stable numeric literal (changing it
  would silently reclassify every system resource).

**Used by** — `system_resource_note_id` (the serde default fn); `Resource::new` callers in
`interop.rs`; the `fs`/`db` backends and the server as the migration default and cascade
boundary; tests.

**Repeated context** — the id-plaintext convention: `note_id` is never encrypted (like
`notebook_id`) so queries and cascades work under `EncryptedBackend`.

---

## fn system_resource_note_id

**Identification** — private free function `fn system_resource_note_id() -> Uuid`; marker
`// md:fn system_resource_note_id`.

**Code** — complete and verbatim:

```rust
// md:fn system_resource_note_id
fn system_resource_note_id() -> Uuid {
    SYSTEM_RESOURCE_NOTE_ID
}
```

**What it does** — The `#[serde(default = "…")]` provider for `Resource.note_id`: a record
missing `note_id` (any pre-#125 journal entry) deserialises to the system sentinel rather than
failing. Construction-time obligation (a real `note_id` for user attachments) is enforced by
`Resource::new`, not by the parser — so `#[serde(default)]` is cheap defensive compatibility,
consistent with the additive-evolution rule in this module.

**Dependencies** —
- `SYSTEM_RESOURCE_NOTE_ID` — the value returned; expects the sentinel to stay non-nil and
  distinct from `Uuid::nil()`.

**Used by** — serde, via `#[serde(default = "system_resource_note_id")]` on `Resource.note_id`.

**Repeated context** — none.

---

## Resource

**Identification** — struct deriving `Debug, Clone, Serialize, Deserialize, PartialEq, Eq,
Hash`; marker `// md:Resource`.

**Code** — complete and verbatim:

```rust
// md:Resource
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Resource {
    pub id: Uuid,
    #[serde(default = "system_resource_note_id")]
    pub note_id: Uuid,
    pub title: String,
    pub mime_type: String,
    pub file_name: String,
    pub size: u64,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub dimensions: Option<(u32, u32)>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub vv: VersionVector,
    #[serde(default)]
    pub last_writer: String,
}
```

**What it does** — Metadata for a binary file attachment; the payload is stored separately (a
`data` file on disk for `FsBackend`, a BLOB column for `DbBackend`) and fetched explicitly via
`read_resource`. Every attachment belongs to **exactly one note**: `note_id` is **plaintext**
(an id, like `notebook_id`, so per-note queries and the soft-delete cascade work under
`EncryptedBackend`), `#[serde(default = "system_resource_note_id")]` (missing ⇒ system
sentinel), and **immutable** after creation (attachments are never reparented; the obligation
to pass a real note is enforced by `Resource::new`, not the parser). Other fields: `id`,
`title` / `mime_type` (IANA, e.g. `"image/png"`) / `file_name`
— all encrypted at rest; `size` in **plaintext** (needed to validate uploads without
decrypting); `duration_ms` (audio/video length) and `dimensions` (`(width, height)` for
images) — both `Option`, `#[serde(default)]`, and in **plaintext** for the same reason as
`size`: they are media metadata, not content, so a frontend can render/measure an attachment
without downloading or decrypting the blob. The backend never computes or validates them — the
producer of the attachment fills them in; a non-media attachment leaves both `None`. `dimensions`
is both-or-neither (an image has width and height, or neither). `created_at`; `deleted_at`
(`#[serde(default)]`) — resources use a soft-delete
tombstone rather than physical removal so a delete converges with a concurrent create through
`note_log::resolve`, with the payload retained on delete (reclaiming it is a separate
compaction concern — `purge_deleted_resources`); `vv`/`last_writer` (`#[serde(default)]`).

**Dependencies** —
- `VersionVector` — `vv`; expects `Default`.
- `system_resource_note_id` — the `note_id` serde default; expects it to return the reserved
  sentinel.
- `chrono`, `uuid`, `serde` derive — same shared conventions.

**Used by** — `ResourceRepository`; `interop.rs` (contacts/events are stored as resources);
`migrate.rs`; the daemon's resource endpoints.

**Repeated context** — none beyond the shared entity contract.

---

## impl Resource

**Identification** — inherent impl; marker `// md:impl Resource`. One method.

**Code** — container: members documented as sub-blocks below: `fn new`.

### fn new

**Identification** — `pub fn new(note_id: Uuid, title, mime_type, file_name, size: u64) -> Self`
(the string parameters are `impl Into<String>`); marker `// md:impl Resource > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl Resource > fn new
    pub fn new(
        note_id: Uuid,
        title: impl Into<String>,
        mime_type: impl Into<String>,
        file_name: impl Into<String>,
        size: u64,
    ) -> Self {
        Self {
            id: new_id(),
            note_id,
            title: title.into(),
            mime_type: mime_type.into(),
            file_name: file_name.into(),
            size,
            duration_ms: None,
            dimensions: None,
            created_at: now(),
            deleted_at: None,
            vv: VersionVector::new(),
            last_writer: String::new(),
        }
    }
```

**What it does** — Fresh UUID + `now()`; `note_id` is **required in construction** — this is
where the "every attachment belongs to exactly one note" invariant is enforced (a user
attachment passes its owning note; `interop.rs` passes `SYSTEM_RESOURCE_NOTE_ID`).
`duration_ms`/`dimensions` default to `None` (the
producer sets them afterwards if the attachment is media); the binary payload is **not** stored here — it is
passed separately to `create_resource`. `size` must be the exact byte length of that payload.

**Dependencies** —
- `new_id`, `now`, `VersionVector::new` — id/timestamp/empty vv; same expectations as the
  other constructors.

**Used by** — `ResourceRepository` create path; `interop.rs`; `migrate.rs`; tests.

**Repeated context** — `size` is plaintext and must equal the payload length the caller passes
to `create_resource` (used to validate uploads without decrypting).

---

## Change

**Identification** — enum deriving `Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash`
with `#[serde(tag = "op", rename_all = "snake_case")]`; marker `// md:Change`.

**Code** — complete and verbatim:

```rust
// md:Change
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    #[serde(alias = "create")]
    NoteCreate {
        note: Note,
    },
    #[serde(alias = "update")]
    NoteUpdate {
        note: Note,
    },
    #[serde(alias = "delete")]
    NoteDelete {
        id: Uuid,
        #[serde(default = "now")]
        deleted_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
    NotebookCreate {
        notebook: Notebook,
    },
    NotebookUpdate {
        notebook: Notebook,
    },
    NotebookDelete {
        id: Uuid,
        #[serde(default = "now")]
        deleted_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
    TagCreate {
        tag: Tag,
    },
    TagUpdate {
        tag: Tag,
    },
    TagDelete {
        id: Uuid,
        #[serde(default = "now")]
        deleted_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
    NoteTagAdd {
        note_id: Uuid,
        tag_id: Uuid,
        #[serde(default = "now")]
        updated_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
    NoteTagRemove {
        note_id: Uuid,
        tag_id: Uuid,
        #[serde(default = "now")]
        updated_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
    ResourceCreate {
        resource: Resource,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        data: Option<Vec<u8>>,
    },
    ResourceDelete {
        id: Uuid,
        #[serde(default = "now")]
        deleted_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
}
```

**What it does** — One unit of change applied to a local store to converge with another
device. Every mutating backend operation appends one to the change journal (`entity_changes`
table in `DbBackend`, NDJSON log in `FsBackend`); sync sends/receives them and applies each via
the idempotent `apply_change`. Serialised as `{"op":"note_create","note":{…}}` — the `op` tag
carries the variant in snake_case. Variants:

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

**Dependencies** —
- `Note`, `Notebook`, `Tag`, `Resource` — the create/update payloads; expect each to stay
  `serde`-serializable with stable field names (a full snapshot is embedded per change).
- `VersionVector` — the `vv` on every tombstone/association variant; expects `Default` so
  pre-version records parse, and expects `resolve` to compare it identically for deletes and
  edits (the delete-competes-like-an-edit invariant).
- `now` — named in `#[serde(default = "now")]` on the timestamp fields; expects that fn to
  exist by that name (renaming it breaks these defaults).
- `serde` (`tag`/`rename_all`/`alias`/`skip_serializing_if`) — the wire shape; expects
  additive-only evolution (see Repeated context).

**Used by** — both backends' journals and `apply_change`; `sync/engine.rs`;
`keeplin-daemon/src/event_backend.rs` (publishes applied changes);
`keeplin-daemon/src/search.rs` (re-indexes on changes); keeplin-srv's relay stores these
verbatim.

**Repeated context** — **additive evolution only**: renaming variants or fields breaks stored
journals and the server relay; v1 aliases are kept so old logs read without migration
(clean-break premise applies to *wire/protocol* versions, while stored journals stay readable
via serde defaults/aliases). Full snapshots per change are what make journal-derived history
possible.

---

## mod tests

**Identification** — `#[cfg(test)]` unit-test module; marker `// md:mod tests`. Two tests.

**Code** — container: members documented as sub-blocks below:
`fn pre_ordering_note_json_lands_in_the_inbox_with_defaults`,
`fn pre_ordering_note_msgpack_round_trips`. (The module preamble is `#[cfg(test)]`,
`mod tests {`, `use super::*;`.)

**What it does** — Pins the backward-compatible deserialisation of pre-ordering note records in
both encodings (JSON and MessagePack).

**Dependencies** —
- `super::*` — brings `Note`, `now`, `new_id`, `DEFAULT_SORT_KEY` into the tests; expects those
  names/shapes to stay.
- `serde_json`, `rmp_serde` — the two decoders exercised; expect JSON and MessagePack to keep
  reading old-shaped records via `#[serde(default)]`/`de_notebook_id`.

**Used by** — CI (`cargo test --workspace`).

**Repeated context** — MessagePack (`rmp_serde`) is the on-disk note-log encoding; JSON is used
for the global journal and network.

The explicit `imports` leaf below preserves the test-module dependency preamble
verbatim.

### imports

**Identification** — test-module dependencies; marker `// md:mod tests > imports`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > imports
    use super::*;
```

**What it does** — Brings the parent module API and test-only dependencies into
scope.

**Dependencies** —

- `super::*` — the items under test from the parent module; expects: the parent keeps them at module scope; a rename or a move into a submodule breaks these tests at compile time, which is the intended early signal.

**Used by** — every block of `mod tests` in this file: `fn pre_ordering_note_json_lands_in_the_inbox_with_defaults`, `fn pre_ordering_note_msgpack_round_trips`. Nothing outside the module can use it: the preamble is private to `mod tests`.

**Repeated context** — This preamble is a leaf block, not scaffolding: only the `mod` declaration, its attributes and its braces are exempt from coverage, so these `use` lines carry their own marker and are verified verbatim against the source (template v2.5.0, RULE 6). Changing an import here without updating this fence fails `scripts/check-docs.sh`.

### fn pre_ordering_note_json_lands_in_the_inbox_with_defaults

**Identification** — unit test; marker
`// md:mod tests > fn pre_ordering_note_json_lands_in_the_inbox_with_defaults`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn pre_ordering_note_json_lands_in_the_inbox_with_defaults
    #[test]
    fn pre_ordering_note_json_lands_in_the_inbox_with_defaults() {
        let old = r#"{
            "id": "6f2a5b1c-9d5c-4c3a-8f21-3b1a2c4d5e6f",
            "title": "t", "body": "b",
            "notebook_id": null,
            "is_todo": false, "todo_due": null, "todo_completed": null,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "deleted_at": null
        }"#;
        let note: Note = serde_json::from_str(old).unwrap();
        assert_eq!(note.notebook_id, Uuid::nil(), "null lands in the Inbox");
        assert!(!note.is_pinned);
        assert!(!note.is_starred);
        assert_eq!(note.sort_key, 0);
        assert_eq!(note.effective_sort_key(), Note::DEFAULT_SORT_KEY);
    }
```

**What it does** — A note serialized before pinning/starring/ordering existed (`notebook_id:
null`, new fields absent) must load with the defaults: Inbox (nil UUID), unpinned, unstarred,
`sort_key 0`, and `effective_sort_key()` mapping to `DEFAULT_SORT_KEY`.

**Dependencies** —
- `serde_json::from_str` — decodes the legacy JSON; expects `de_notebook_id` + the
  `#[serde(default)]` fields to fill in everything the old record omits.
- `Note::effective_sort_key`, `Note::DEFAULT_SORT_KEY` — the assertion targets; expect the
  `0 → DEFAULT_SORT_KEY` mapping to hold.

**Used by** — CI.

**Repeated context** — none.

### fn pre_ordering_note_msgpack_round_trips

**Identification** — unit test; marker
`// md:mod tests > fn pre_ordering_note_msgpack_round_trips`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn pre_ordering_note_msgpack_round_trips
    #[test]
    fn pre_ordering_note_msgpack_round_trips() {
        #[derive(serde::Serialize)]
        struct OldNote<'a> {
            id: Uuid,
            title: &'a str,
            body: &'a str,
            notebook_id: Option<Uuid>,
            is_todo: bool,
            todo_due: Option<DateTime<Utc>>,
            todo_completed: Option<DateTime<Utc>>,
            created_at: DateTime<Utc>,
            updated_at: DateTime<Utc>,
            deleted_at: Option<DateTime<Utc>>,
        }
        let ts = now();
        let old = OldNote {
            id: new_id(),
            title: "t",
            body: "b",
            notebook_id: None,
            is_todo: false,
            todo_due: None,
            todo_completed: None,
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
        };
        let bytes = rmp_serde::to_vec_named(&old).unwrap();
        let note: Note = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(note.notebook_id, Uuid::nil());
        assert_eq!(note.sort_key, 0);
    }
```

**What it does** — Serialises an old-shape mirror struct (an in-test `OldNote` with
`notebook_id: Option<Uuid>` and no ordering fields) with `rmp_serde::to_vec_named` and
deserialises it as today's `Note`: nil notebook id and `sort_key 0`.

**Dependencies** —
- `rmp_serde::to_vec_named` / `rmp_serde::from_slice` — encode the old shape, decode as today's
  `Note`; expect named MessagePack (field names, not positions) so `#[serde(default)]` works on
  the missing fields. If encoding switched to positional, the missing trailing fields would not
  default-fill.
- `new_id`, `now` — build the fixture.

**Used by** — CI.

**Repeated context** — `to_vec_named` (not positional) is required for the on-disk note-log
encoding so added fields stay backward-compatible.

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
- `keeplin-core/src/storage/db/` — DbBackend across its modules (EXTRACTED: edges to `mod.rs`, `migrations.rs`, `rows.rs`, `notes.rs`, `notebooks.rs`, `tags.rs`, `resources.rs`, `sync.rs`, `server.rs`; e.g. `.get_or_create_device_id()`, `.send_changes()`, `.create_note()`)
- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: calls×1, references×37; e.g. `.read_or_create_device_id()`, `.append_note_op()`, `.create_note()`)
- `keeplin-core/src/storage/note_log.rs` — version-vector resolution (EXTRACTED: imports_from×1, references×2; e.g. `Merged`, `NoteOp`)
- `keeplin-core/src/sync/engine.rs` — SyncEngine (EXTRACTED: references×2; e.g. `run_sync()`, `.sync()`)
- `keeplin-daemon/src/event_backend.rs` — `EventBackend` change-publishing decorator (EXTRACTED: references×30; e.g. `.create_note()`, `.list_notes()`, `.list_notes_in_notebook()`)
- `keeplin-daemon/src/metrics.rs` — operational metrics (EXTRACTED: references×26; e.g. `.create_note()`, `.list_notes()`, `.list_notes_in_notebook()`)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: references×42; e.g. `add_link()`, `batch_revert_notes_ep()`, `create_note()`)
- `keeplin-daemon/src/search.rs` — daemon full-text search (EXTRACTED: references×5; e.g. `denormalize()`, `index_note()`, `.upsert()`)
- `keeplin-daemon/src/server.rs` — gRPC service implementation (EXTRACTED: references×5; e.g. `note_to_proto()`, `proto_to_note()`, `notebook_to_proto()`)

**Invariants** (the rules this file must keep true — restated here even if stated elsewhere)

- Every entity carries `vv` + `last_writer` + `updated_at` + soft-delete `deleted_at`; these
  are the inputs `note_log::resolve` reads. Dropping one breaks convergence.
- `notebook_id` is never "none": `Uuid::nil()` (the Inbox) is the sentinel; `de_notebook_id`
  upgrades old `null`/missing values.
- `Change` evolves additively only (serde `alias`/`default` keep old journals readable); a
  rename breaks stored logs and the server relay.
- All human-readable content (`title`, `body`, `alias`, resource `mime_type`/`file_name`) is
  encrypted at rest; ids/flags/sizes/sync-metadata stay plaintext.

**Cross-repo contracts**

- `sync-change-envelope` — canonical `Change` variants serialized through the server's opaque relay.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `fn new_id` | `// md:fn new_id` |
| 3 | `fn now` | `// md:fn now` |
| 4 | `struct Note` | `// md:Note` |
| 5 | `fn de_notebook_id` | `// md:fn de_notebook_id` |
| 6 | `impl Note` (container) | `// md:impl Note` |
| 7 | `const DEFAULT_SORT_KEY` | `// md:impl Note > DEFAULT_SORT_KEY` |
| 8 | `fn new` (Note) | `// md:impl Note > fn new` |
| 9 | `fn effective_sort_key` | `// md:impl Note > fn effective_sort_key` |
| 10 | `struct Notebook` | `// md:Notebook` |
| 11 | `impl Notebook` (container) | `// md:impl Notebook` |
| 12 | `fn new` (Notebook) | `// md:impl Notebook > fn new` |
| 13 | `struct Tag` | `// md:Tag` |
| 14 | `impl Tag` (container) | `// md:impl Tag` |
| 15 | `fn new` (Tag) | `// md:impl Tag > fn new` |
| 16 | `struct NoteTag` | `// md:NoteTag` |
| 17 | `const SYSTEM_RESOURCE_NOTE_ID` | `// md:SYSTEM_RESOURCE_NOTE_ID` |
| 18 | `fn system_resource_note_id` | `// md:fn system_resource_note_id` |
| 19 | `struct Resource` | `// md:Resource` |
| 20 | `impl Resource` (container) | `// md:impl Resource` |
| 21 | `fn new` (Resource) | `// md:impl Resource > fn new` |
| 22 | `enum Change` | `// md:Change` |
| 23 | `mod tests` (container) | `// md:mod tests` |
| 24 | `imports` | `// md:mod tests > imports` |
| 25 | `fn pre_ordering_note_json_lands_in_the_inbox_with_defaults` | `// md:mod tests > fn pre_ordering_note_json_lands_in_the_inbox_with_defaults` |
| 26 | `fn pre_ordering_note_msgpack_round_trips` | `// md:mod tests > fn pre_ordering_note_msgpack_round_trips` |
