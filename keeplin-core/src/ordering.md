# `ordering.rs` — the Inbox, pinning, manual ordering, and starring

Self-contained companion for `keeplin-core/src/ordering.rs`. It documents **every code
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
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Note, Notebook},
    storage::{NotebookSortProfile, StorageBackend},
};
```

**What it does** — The Inbox system notebook, pinning, manual ordering, and
starring. The model: every note belongs to exactly one notebook
(`Note::notebook_id` is never "none"); a note created without choosing one lands
in the **Inbox** — a system notebook with the fixed nil UUID and the title
"Inbox", auto-created at daemon startup and protected from user deletion by the
API surfaces. Within a notebook, notes order by `(effective sort_key ASC, id ASC)`:

- **Normal notebooks** have two bands: `1..=999` is the **pinned** band (shown at
  the top, at most `MAX_PINNED` notes) and `>= 1000` the **normal** band. New
  notes enter the normal band after its current last note.
- **The Inbox** is one flat band with no pinning; new notes insert at the **top**
  (`min(existing) - 1`, resequencing first when the space above is exhausted).
- `sort_key == 0` is the legacy "never positioned" sentinel, ordered as
  `Note::DEFAULT_SORT_KEY` (the start of the normal band).

**Starring** is a global flag, fully orthogonal to pinning and to the notebook —
it never moves the note. Where the logic lives: like `crate::linking`'s alias
helpers, everything here is a free function over `&dyn StorageBackend` doing
read-modify-write through the normal `update_note` path, so version vectors,
encryption, link derivation, and the live-change feed all apply unchanged and the
result syncs like any other edit; placement decisions read one
`NotebookSortProfile` instead of materialising the notebook. Concurrency: sort
keys are plain note fields, so concurrent edits resolve like any note conflict
(whole-note version-vector resolution with the deterministic tiebreak); two
devices pinning concurrently can pick the same key — ordering stays deterministic
(`id` breaks ties) and the next reorder can separate them. Old peers ignore the
new fields (serde/protobuf defaults) and their notes keep `sort_key = 0`.

**Dependencies** — `uuid`, `crate::{error, models, storage}`.

**Used by** — the daemon's create/update/pin/reorder/star surfaces (REST + gRPC
call the same free functions for identical behaviour), `linking.rs` (`is_inbox`
for the Inbox short-circuit), tests.

**Repeated context** — free-function-over-`&dyn StorageBackend` is the pattern
for cross-surface domain logic; decorators are for per-operation transformation.

---

## INBOX_ID

**Identification** — `pub const INBOX_ID: Uuid = Uuid::nil();` marker
`// md:INBOX_ID`.

**What it does** — The Inbox's fixed identity: the nil UUID on every device, so
it never needs to sync an id and can be addressed before it exists.

**Dependencies** — `uuid`.

**Used by** — `ensure_inbox`, `is_inbox`, `resequence_inbox`, the daemon and
clients (nil UUID = the Inbox in every API), `models::Note` field docs.

**Repeated context** — none.

---

## INBOX_TITLE

**Identification** — `pub const INBOX_TITLE: &str = "Inbox";` marker
`// md:INBOX_TITLE`.

**What it does** — The Inbox's title, applied when `ensure_inbox` creates it.

**Dependencies** — none.

**Used by** — `ensure_inbox`; the `ensure_inbox_is_idempotent_and_fixed` test.

**Repeated context** — project premise: the inbox is called "Inbox" everywhere —
no other name for it remains in either repo.

---

## PIN_MAX

**Identification** — `pub const PIN_MAX: u32 = 999;` marker `// md:PIN_MAX`.

**What it does** — Highest sort key of the pinned band (`1..=PIN_MAX`), and
therefore also the maximum number of pinned notes per notebook.

**Dependencies** — none.

**Used by** — `reorder_note` (band validation), `lowest_free_pinned_key`,
`MAX_PINNED`.

**Repeated context** — none.

---

## MAX_PINNED

**Identification** — `pub const MAX_PINNED: usize = PIN_MAX as usize;` marker
`// md:MAX_PINNED`.

**What it does** — Maximum pinned notes per notebook — the pinned band simply
has no more keys.

**Dependencies** — `PIN_MAX`.

**Used by** — `pin_note`'s `Conflict` message; the API surfaces' docs.

**Repeated context** — none.

---

## NORMAL_START

**Identification** — `pub const NORMAL_START: u32 = Note::DEFAULT_SORT_KEY;`
marker `// md:NORMAL_START`.

**What it does** — First sort key of the normal (unpinned) band (1000).

**Dependencies** — `Note::DEFAULT_SORT_KEY`.

**Used by** — `place_new_note`, `reorder_note`, `next_normal_key`, tests.

**Repeated context** — none.

---

## RESEQUENCE_STEP

**Identification** — `const RESEQUENCE_STEP: u32 = 1000;` marker
`// md:RESEQUENCE_STEP`.

**What it does** — Spacing used when the Inbox is resequenced — leaves room
above and between notes so the next resequence is far away.

**Dependencies** — none.

**Used by** — `resequence_inbox`.

**Repeated context** — none.

---

## fn ensure_inbox

**Identification** — `pub async fn ensure_inbox(backend: &dyn StorageBackend) -> Result<(), StorageError>`;
marker `// md:fn ensure_inbox`.

**What it does** — Creates the Inbox system notebook if this store doesn't have
it yet: `read_notebook(INBOX_ID)` → exists = done; `NotFound` = build a
`Notebook::new(INBOX_TITLE)` with its id overwritten to `INBOX_ID` and create
it (logging the creation); any other error propagates. **Idempotent — call at
every startup.** Two devices creating it concurrently converge like any other
notebook conflict (both sides write the same fixed id).

**Dependencies** — `INBOX_ID`, `INBOX_TITLE`, `NotebookRepository`, `tracing`.

**Used by** — `keeplin-daemon/src/main.rs` at startup; tests.

**Repeated context** — the API surfaces refuse to delete the Inbox
(`InvalidInput`/HTTP 400); this function is why it always exists to protect.

---

## fn is_inbox

**Identification** — `pub fn is_inbox(id: Uuid) -> bool`; marker
`// md:fn is_inbox`.

**What it does** — Whether `id` names the Inbox (`id == INBOX_ID`). The API
surfaces use it to refuse deleting the Inbox; `linking::LinkingBackend` uses it
to keep Inbox notes out of the linking graph (they carry no alias, emit no
links, and are never a link target).

**Dependencies** — `INBOX_ID`.

**Used by** — `place_new_note`, `pin_note`, `reorder_note`, `linking.rs` (×7
call sites), the daemon surfaces.

**Repeated context** — none.

---

## fn place_new_note

**Identification** — `pub async fn place_new_note(backend: &dyn StorageBackend, note: &mut Note) -> Result<(), StorageError>`;
marker `// md:fn place_new_note`.

**What it does** — Assigns a brand-new note its initial position, honouring a
caller-chosen `sort_key` when one was set (`!= 0` → early return). Reads the
notebook's `NotebookSortProfile`; in the **Inbox** the note goes to the top:
`min(existing) - 1`, but when the minimum would fall to the `0` sentinel
(`min <= 1`) the Inbox is resequenced first and the note lands above the new
minimum; an empty Inbox starts at `NORMAL_START`. In a **normal notebook** it
enters the normal band after its current last note (`next_normal_key`). Call
**before** `create_note` — the daemon does on every create surface.

**Dependencies** — `notebook_sort_profile`, `is_inbox`, `resequence_inbox`,
`next_normal_key`.

**Used by** — the daemon's create paths; `reconcile_notebook_move`; the tests'
`create_placed` helper.

**Repeated context** — the `0` sentinel must never be *assigned* — it is
reserved as "never positioned" for pre-ordering records.

---

## fn pin_note

**Identification** — `pub async fn pin_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError>`;
marker `// md:fn pin_note`.

**What it does** — Moves a note into its notebook's pinned band at the lowest
free key and returns the updated note. Rejects an Inbox note with
`InvalidInput` (the Inbox is a single manually ordered list — no pinning);
pinning an already-pinned note is a no-op returning it unchanged; a full band
(`MAX_PINNED` notes) is `Conflict`. Reads through `read_live_note` (a tombstone
is `NotFound` — a pin must never revive a deleted note).

**Dependencies** — `read_live_note`, `is_inbox`, `notebook_sort_profile`,
`lowest_free_pinned_key`, `update_note`.

**Used by** — the daemon's pin surface; tests.

**Repeated context** — the daemon maps `InvalidInput` → 400/`INVALID_ARGUMENT`
and `Conflict` → 409/`ALREADY_EXISTS` — this function's error choices target
that mapping.

---

## fn unpin_note

**Identification** — `pub async fn unpin_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError>`;
marker `// md:fn unpin_note`.

**What it does** — Moves a pinned note to the **end** of its notebook's normal
band (`next_normal_key`); unpinning an unpinned note is a no-op returning it
unchanged.

**Dependencies** — `read_live_note`, `notebook_sort_profile`,
`next_normal_key`, `update_note`.

**Used by** — the daemon's unpin surface; tests.

**Repeated context** — none.

---

## fn reconcile_notebook_move

**Identification** — `pub async fn reconcile_notebook_move(backend: &dyn StorageBackend, current_notebook_id: Uuid, note: &mut Note) -> Result<(), StorageError>`;
marker `// md:fn reconcile_notebook_move`.

**What it does** — Reconciles a user-facing update whose `notebook_id` differs
from where the note currently lives. Position (`sort_key`) and pinned state
belong to a *specific* notebook — the pinned band is per-notebook and a key is
only meaningful among that notebook's notes — so on a move both are reset
(`is_pinned = false`, `sort_key = 0`) and the note is re-placed in the
destination via `place_new_note` (top of the Inbox, or end of a normal
notebook's normal band). Moving into the Inbox clears any pinned state; a
same-notebook edit is a **no-op**, so a plain edit keeps the manual position.
`current_notebook_id` is the notebook the stored note is in now (the caller has
just read it). Call from the generic "user edits a note" path **only** — the
pin/unpin/reorder ops set `sort_key`/`is_pinned` deliberately and call
`update_note` directly, so they must not go through here.

**Dependencies** — `place_new_note`.

**Used by** — the daemon's generic note-update path (REST and gRPC); the tests'
`move_note` helper.

**Repeated context** — none.

---

## fn star_note

**Identification** — `pub async fn star_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError>`;
marker `// md:fn star_note`.

**What it does** — Stars a note (global flag; position never changes).
Idempotent. Thin wrapper over `set_starred(…, true)`.

**Dependencies** — `set_starred`.

**Used by** — the daemon's star surface.

**Repeated context** — none.

---

## fn unstar_note

**Identification** — `pub async fn unstar_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError>`;
marker `// md:fn unstar_note`.

**What it does** — Removes the star. Idempotent. Wrapper over
`set_starred(…, false)`.

**Dependencies** — `set_starred`.

**Used by** — the daemon's unstar surface.

**Repeated context** — none.

---

## fn set_starred

**Identification** — private
`async fn set_starred(backend: &dyn StorageBackend, id: Uuid, starred: bool) -> Result<Note, StorageError>`;
marker `// md:fn set_starred`.

**What it does** — The shared read-modify-write: `read_live_note`, no-op when
the flag already matches (returns the note unchanged, avoiding a version bump),
otherwise set and `update_note`.

**Dependencies** — `read_live_note`, `update_note`.

**Used by** — `star_note`, `unstar_note`.

**Repeated context** — no-op-on-equal is deliberate: an idempotent user action
must not spam the change journal with identical writes.

---

## fn reorder_note

**Identification** — `pub async fn reorder_note(backend: &dyn StorageBackend, id: Uuid, new_sort_key: u32) -> Result<Note, StorageError>`;
marker `// md:fn reorder_note`.

**What it does** — Gives a note a new manual position **within its current
band**: a pinned note accepts `1..=PIN_MAX` (unpin it instead of renumbering it
out), a normal note `>= NORMAL_START`, an Inbox note any key `>= 1` (`0` is the
legacy sentinel). An out-of-band key is `InvalidInput` with a message naming the
expected band. Same-key is a no-op. Duplicate keys are allowed — ordering stays
deterministic (`id` breaks ties).

**Dependencies** — `read_live_note`, `is_inbox`, `PIN_MAX`, `NORMAL_START`,
`update_note`.

**Used by** — the daemon's reorder surface; tests.

**Repeated context** — none.

---

## fn resequence_inbox

**Identification** — `pub async fn resequence_inbox(backend: &dyn StorageBackend) -> Result<u32, StorageError>`;
marker `// md:fn resequence_inbox`.

**What it does** — Renumbers every live Inbox note to `1000, 2000, 3000, …` in
its current order (paging through `list_notes_in_notebook` with the default page
size), skipping notes already at their target key, and returns the new minimum
(`RESEQUENCE_STEP`). Called when top-insertion has consumed all room above the
first note; the wide spacing pushes the next resequence far away. Each
renumbered note goes through `update_note`, so the moves version and sync
normally.

**Dependencies** — `list_notes_in_notebook`, `update_note`, `RESEQUENCE_STEP`,
`Note::effective_sort_key`.

**Used by** — `place_new_note` (its only caller).

**Repeated context** — none.

---

## fn lowest_free_pinned_key

**Identification** — private `fn lowest_free_pinned_key(used: &[u32]) -> Option<u32>`;
marker `// md:fn lowest_free_pinned_key`.

**What it does** — The lowest key in `1..=PIN_MAX` not present in `used` (which
the profile returns sorted ascending), or `None` when the band is full. Linear
gap-scan: the candidate starts at 1 and advances past each occupied key.

**Dependencies** — `PIN_MAX`.

**Used by** — `pin_note`; unit test
`lowest_free_pinned_key_fills_gaps_and_detects_full`.

**Repeated context** — none.

---

## fn next_normal_key

**Identification** — private `fn next_normal_key(profile: &NotebookSortProfile) -> u32`;
marker `// md:fn next_normal_key`.

**What it does** — The key after the last note of the normal band:
`max_normal_key + 1` (saturating), or `NORMAL_START` when the band is empty.

**Dependencies** — `NotebookSortProfile`, `NORMAL_START`.

**Used by** — `place_new_note`, `unpin_note`.

**Repeated context** — none.

---

## fn read_live_note

**Identification** — private `async fn read_live_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError>`;
marker `// md:fn read_live_note`.

**What it does** — Reads a note for a user-facing read-modify-write, rejecting a
tombstone (`deleted_at.is_some()`) as `NotFound` — ordering edits must never
revive a deleted note. Mirrors `linking.rs`'s equivalent helper.

**Dependencies** — `read_note`.

**Used by** — `pin_note`, `unpin_note`, `set_starred`, `reorder_note`.

**Repeated context** — some backends return tombstones from `read_note` (needed
for resolution); user-facing ops re-check.

---

## mod tests

**Identification** — `#[cfg(test)]` test module; marker `// md:mod tests`. Four
helpers + ten tests against a real `FsBackend` in a tempdir.

**What it does** — End-to-end coverage of placement, pinning, reordering,
starring, moves, resequencing, and the pure gap-scan.

**Dependencies** — `super::*`, `storage::fs::FsBackend`,
`storage::{NoteRepository, NotebookRepository}`, `tempfile`, `tokio`.

**Used by** — CI.

**Repeated context** — none.

### fn backend

**Identification** — helper `async fn backend() -> FsBackend`; marker
`// md:mod tests > fn backend`.

**What it does** — `FsBackend` in a leaked tempdir (`std::mem::forget` keeps the
directory alive for the test's duration).

### fn create_placed

**Identification** — helper; marker `// md:mod tests > fn create_placed`.

**What it does** — Creates a note through the daemon's placement rule
(`place_new_note` then `create_note`) — what every API create does.

### fn move_note

**Identification** — helper; marker `// md:mod tests > fn move_note`.

**What it does** — Moves a note to `dest` through the daemon's generic-update
path (read → set `notebook_id` → `reconcile_notebook_move` → `update_note`),
the sequence both API surfaces run.

### fn titles

**Identification** — helper `fn titles(page: &[Note]) -> Vec<&str>`; marker
`// md:mod tests > fn titles`.

**What it does** — Projects a page to its titles for order assertions.

### fn ensure_inbox_is_idempotent_and_fixed

**Identification** — tokio test; marker
`// md:mod tests > fn ensure_inbox_is_idempotent_and_fixed`.

**What it does** — Two `ensure_inbox` calls; the notebook exists with
`INBOX_ID`/`INBOX_TITLE`.

### fn placement_inbox_top_notebook_bottom

**Identification** — tokio test; marker
`// md:mod tests > fn placement_inbox_top_notebook_bottom`.

**What it does** — Three Inbox creates list newest-first (top-insert); two
notebook creates land at `NORMAL_START`, `NORMAL_START + 1` and list in creation
order.

### fn pin_unpin_round_trip_and_inbox_rejection

**Identification** — tokio test; marker
`// md:mod tests > fn pin_unpin_round_trip_and_inbox_rejection`.

**What it does** — Pin moves a note to key 1 and lists it first; unpin appends
it to the normal band; pinning an Inbox note is `InvalidInput`.

### fn reorder_respects_bands

**Identification** — tokio test; marker
`// md:mod tests > fn reorder_respects_bands`.

**What it does** — Normal-band reorder changes listing order; a normal note
cannot take a pinned key and vice versa (`InvalidInput` both ways); a pinned
note reorders within `1..=PIN_MAX`.

### fn starring_is_global_and_never_moves_the_note

**Identification** — tokio test; marker
`// md:mod tests > fn starring_is_global_and_never_moves_the_note`.

**What it does** — Stars notes in the Inbox and a notebook: `sort_key`
unchanged, `list_starred_notes` spans notebooks, unstar removes from the list.

### fn inbox_top_insert_survives_underflow_by_resequencing

**Identification** — tokio test; marker
`// md:mod tests > fn inbox_top_insert_survives_underflow_by_resequencing`.

**What it does** — Pushes the Inbox head to key 1, creates another note: the
newcomer never gets the 0 sentinel and lists on top (resequencing happened
underneath).

### fn moving_a_note_replaces_it_in_the_destination_band

**Identification** — tokio test; marker
`// md:mod tests > fn moving_a_note_replaces_it_in_the_destination_band`.

**What it does** — An Inbox note whose key (999, from top-insertion) falls
inside the pinned numeric range moves to a notebook: it is re-placed in the
normal band (never auto-pinned) and leaves the Inbox.

### fn moving_a_pinned_note_into_the_inbox_unpins_it

**Identification** — tokio test; marker
`// md:mod tests > fn moving_a_pinned_note_into_the_inbox_unpins_it`.

**What it does** — A pinned notebook note moved into the Inbox arrives
unpinned.

### fn a_same_notebook_edit_keeps_the_position

**Identification** — tokio test; marker
`// md:mod tests > fn a_same_notebook_edit_keeps_the_position`.

**What it does** — A title-only edit through the reconcile path keeps
`sort_key` — no re-placement on a plain edit.

### fn lowest_free_pinned_key_fills_gaps_and_detects_full

**Identification** — unit test; marker
`// md:mod tests > fn lowest_free_pinned_key_fills_gaps_and_detects_full`.

**What it does** — Gap-scan cases: empty → 1, `[1,2,3]` → 4, `[1,3]` → 2,
`[2,3]` → 1, full band → `None`, all-but-`PIN_MAX` → `PIN_MAX`.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `is_inbox()` — defined here (EXTRACTED; 7 cross-file edge(s))
- `place_new_note()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `pin_note()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `unpin_note()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `reconcile_notebook_move()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `star_note()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `unstar_note()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `set_starred()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `reorder_note()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `read_live_note()` — defined here (EXTRACTED; 3 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs` — error types (EXTRACTED: references×11; e.g. `StorageError`)
- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: references×12; e.g. `Note`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: references×12; e.g. `StorageBackend`, `NotebookSortProfile`)
- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: imports_from×1, references×3; e.g. `FsBackend`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/linking.rs` — `LinkingBackend` decorator + reference resolution (EXTRACTED: calls×7; e.g. `.upsert_note()`, `.resolve_note_seg()`, `.prepare()`)
- `keeplin-daemon/src/main.rs`, `rest.rs`, `server.rs` — startup `ensure_inbox` and the pin/reorder/star endpoints (INFERRED: fully-qualified `keeplin_core::ordering::…` paths the AST pass does not link)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `const INBOX_ID` | `// md:INBOX_ID` |
| 3 | `const INBOX_TITLE` | `// md:INBOX_TITLE` |
| 4 | `const PIN_MAX` | `// md:PIN_MAX` |
| 5 | `const MAX_PINNED` | `// md:MAX_PINNED` |
| 6 | `const NORMAL_START` | `// md:NORMAL_START` |
| 7 | `const RESEQUENCE_STEP` | `// md:RESEQUENCE_STEP` |
| 8 | `fn ensure_inbox` | `// md:fn ensure_inbox` |
| 9 | `fn is_inbox` | `// md:fn is_inbox` |
| 10 | `fn place_new_note` | `// md:fn place_new_note` |
| 11 | `fn pin_note` | `// md:fn pin_note` |
| 12 | `fn unpin_note` | `// md:fn unpin_note` |
| 13 | `fn reconcile_notebook_move` | `// md:fn reconcile_notebook_move` |
| 14 | `fn star_note` | `// md:fn star_note` |
| 15 | `fn unstar_note` | `// md:fn unstar_note` |
| 16 | `fn set_starred` | `// md:fn set_starred` |
| 17 | `fn reorder_note` | `// md:fn reorder_note` |
| 18 | `fn resequence_inbox` | `// md:fn resequence_inbox` |
| 19 | `fn lowest_free_pinned_key` | `// md:fn lowest_free_pinned_key` |
| 20 | `fn next_normal_key` | `// md:fn next_normal_key` |
| 21 | `fn read_live_note` | `// md:fn read_live_note` |
| 22 | `mod tests` | `// md:mod tests` |
| 23–26 | helpers `backend`, `create_placed`, `move_note`, `titles` | `// md:mod tests > fn <name>` |
| 27–36 | the ten tests | `// md:mod tests > fn <name>` |
