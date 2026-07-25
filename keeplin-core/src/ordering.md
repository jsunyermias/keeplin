# `ordering.rs` — the Inbox, pinning, manual ordering, and starring

Self-contained companion for `keeplin-core/src/ordering.rs`. It documents **every code block of
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
use uuid::Uuid;

use crate::{
    error::StorageError,
    format,
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

**Dependencies** — `uuid`, `crate::{error, format, models, storage}`; `format`
supplies the notes-per-notebook cap (`check_notebook_capacity`,
`MAX_NOTES_PER_NOTEBOOK`) this module enforces at placement time — it expects the
predicate to stay pure and to take the count *before* the new note.

**Used by** — the daemon's create/update/pin/reorder/star surfaces (REST + gRPC
call the same free functions for identical behaviour), `linking.rs` (`is_inbox`
for the Inbox short-circuit), tests.

**Repeated context** — free-function-over-`&dyn StorageBackend` is the pattern
for cross-surface domain logic; decorators are for per-operation transformation.

---

## INBOX_ID

**Identification** — `pub const INBOX_ID: Uuid = Uuid::nil();` marker
`// md:INBOX_ID`.

**Code** — complete and verbatim:

```rust
// md:INBOX_ID
pub const INBOX_ID: Uuid = Uuid::nil();
```

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

**Code** — complete and verbatim:

```rust
// md:INBOX_TITLE
pub const INBOX_TITLE: &str = "Inbox";
```

**What it does** — The Inbox's title, applied when `ensure_inbox` creates it.

**Dependencies** — none.

**Used by** — `ensure_inbox`; the `ensure_inbox_is_idempotent_and_fixed` test.

**Repeated context** — project premise: the inbox is called "Inbox" everywhere —
no other name for it remains in either repo.

---

## PIN_MAX

**Identification** — `pub const PIN_MAX: u32 = 999;` marker `// md:PIN_MAX`.

**Code** — complete and verbatim:

```rust
// md:PIN_MAX
pub const PIN_MAX: u32 = 999;
```

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

**Code** — complete and verbatim:

```rust
// md:MAX_PINNED
pub const MAX_PINNED: usize = PIN_MAX as usize;
```

**What it does** — Maximum pinned notes per notebook — the pinned band simply
has no more keys.

**Dependencies** — `PIN_MAX`.

**Used by** — `pin_note`'s `Conflict` message; the API surfaces' docs.

**Repeated context** — none.

---

## NORMAL_START

**Identification** — `pub const NORMAL_START: u32 = Note::DEFAULT_SORT_KEY;`
marker `// md:NORMAL_START`.

**Code** — complete and verbatim:

```rust
// md:NORMAL_START
pub const NORMAL_START: u32 = Note::DEFAULT_SORT_KEY;
```

**What it does** — First sort key of the normal (unpinned) band (1000).

**Dependencies** — `Note::DEFAULT_SORT_KEY`.

**Used by** — `place_new_note`, `reorder_note`, `next_normal_key`, tests.

**Repeated context** — none.

---

## RESEQUENCE_STEP

**Identification** — `const RESEQUENCE_STEP: u32 = 1000;` marker
`// md:RESEQUENCE_STEP`.

**Code** — complete and verbatim:

```rust
// md:RESEQUENCE_STEP
const RESEQUENCE_STEP: u32 = 1000;
```

**What it does** — Spacing used when the Inbox is resequenced — leaves room
above and between notes so the next resequence is far away.

**Dependencies** — none.

**Used by** — `resequence_inbox`.

**Repeated context** — none.

---

## fn ensure_inbox

**Identification** — `pub async fn ensure_inbox(backend: &dyn StorageBackend) -> Result<(), StorageError>`;
marker `// md:fn ensure_inbox`.

**Code** — complete and verbatim:

```rust
// md:fn ensure_inbox
pub async fn ensure_inbox(backend: &dyn StorageBackend) -> Result<(), StorageError> {
    match backend.read_notebook(INBOX_ID).await {
        Ok(_) => Ok(()),
        Err(StorageError::NotFound(_)) => {
            let mut inbox = Notebook::new(INBOX_TITLE);
            inbox.id = INBOX_ID;
            backend.create_notebook(inbox).await?;
            tracing::info!("Created the Inbox system notebook (\"{INBOX_TITLE}\")");
            Ok(())
        }
        Err(e) => Err(e),
    }
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn is_inbox
pub fn is_inbox(id: Uuid) -> bool {
    id == INBOX_ID
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn place_new_note
pub async fn place_new_note(
    backend: &dyn StorageBackend,
    note: &mut Note,
) -> Result<(), StorageError> {
    if note.sort_key != 0 {
        return Ok(());
    }
    let profile = backend.notebook_sort_profile(note.notebook_id).await?;
    format::check_notebook_capacity(profile.live_notes)?;
    note.sort_key = if is_inbox(note.notebook_id) {
        match profile.min_key {
            Some(min) if min <= 1 => resequence_inbox(backend).await? - 1,
            Some(min) => min - 1,
            None => NORMAL_START,
        }
    } else {
        next_normal_key(&profile)
    };
    Ok(())
}
```

**What it does** — Assigns a brand-new note its initial position, honouring a
caller-chosen `sort_key` when one was set (`!= 0` → early return). Reads the
notebook's `NotebookSortProfile` and first enforces the **notes-per-notebook
cap** (issue keeplin#130): `format::check_notebook_capacity(profile.live_notes)`
refuses the note with `StorageError::TooLarge` once the destination already holds
`format::MAX_NOTES_PER_NOTEBOOK` (2²⁴) live notes. Because
`reconcile_notebook_move` resets `sort_key` to `0` and delegates here, that single
call covers both **creating** a note in a notebook and **moving** one into it, and
the Inbox is capped like any other notebook. A caller that pre-assigns a non-zero
`sort_key` skips placement entirely and therefore also skips the cap — that
early return is pre-existing behaviour, and no daemon surface uses it.
Then, in the **Inbox** the note goes to the top:
`min(existing) - 1`, but when the minimum would fall to the `0` sentinel
(`min <= 1`) the Inbox is resequenced first and the note lands above the new
minimum; an empty Inbox starts at `NORMAL_START`. In a **normal notebook** it
enters the normal band after its current last note (`next_normal_key`). Call
**before** `create_note` — the daemon does on every create surface.

**Dependencies** — `notebook_sort_profile` (expects `NotebookSortProfile::live_notes`
to count only non-deleted notes of that notebook — both backends build the profile
from live rows, and an over-count here would refuse notes early),
`format::check_notebook_capacity` (expects `>=` semantics: it is handed the count
*before* the new note), `is_inbox`, `resequence_inbox`, `next_normal_key`.

**Used by** — the daemon's create paths; `reconcile_notebook_move`; the tests'
`create_placed` helper.

**Repeated context** — the `0` sentinel must never be *assigned* — it is
reserved as "never positioned" for pre-ordering records.

---

## fn pin_note

**Identification** — `pub async fn pin_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError>`;
marker `// md:fn pin_note`.

**Code** — complete and verbatim:

```rust
// md:fn pin_note
pub async fn pin_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    let mut note = read_live_note(backend, id).await?;
    if is_inbox(note.notebook_id) {
        return Err(StorageError::InvalidInput(
            "Inbox notes cannot be pinned (the Inbox is a single manually ordered list)"
                .to_string(),
        ));
    }
    if note.is_pinned {
        return Ok(note);
    }
    let profile = backend.notebook_sort_profile(note.notebook_id).await?;
    let Some(key) = lowest_free_pinned_key(&profile.pinned_keys) else {
        return Err(StorageError::Conflict(format!(
            "cannot pin: the notebook already has {MAX_PINNED} pinned notes"
        )));
    };
    note.is_pinned = true;
    note.sort_key = key;
    backend.update_note(note).await
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn unpin_note
pub async fn unpin_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    let mut note = read_live_note(backend, id).await?;
    if !note.is_pinned {
        return Ok(note);
    }
    let profile = backend.notebook_sort_profile(note.notebook_id).await?;
    note.is_pinned = false;
    note.sort_key = next_normal_key(&profile);
    backend.update_note(note).await
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn reconcile_notebook_move
pub async fn reconcile_notebook_move(
    backend: &dyn StorageBackend,
    current_notebook_id: Uuid,
    note: &mut Note,
) -> Result<(), StorageError> {
    if note.notebook_id == current_notebook_id {
        return Ok(());
    }
    note.is_pinned = false;
    note.sort_key = 0;
    place_new_note(backend, note).await
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn star_note
pub async fn star_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    set_starred(backend, id, true).await
}
```

**What it does** — Stars a note (global flag; position never changes).
Idempotent. Thin wrapper over `set_starred(…, true)`.

**Dependencies** — `set_starred`.

**Used by** — the daemon's star surface.

**Repeated context** — none.

---

## fn unstar_note

**Identification** — `pub async fn unstar_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError>`;
marker `// md:fn unstar_note`.

**Code** — complete and verbatim:

```rust
// md:fn unstar_note
pub async fn unstar_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    set_starred(backend, id, false).await
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn set_starred
async fn set_starred(
    backend: &dyn StorageBackend,
    id: Uuid,
    starred: bool,
) -> Result<Note, StorageError> {
    let mut note = read_live_note(backend, id).await?;
    if note.is_starred == starred {
        return Ok(note);
    }
    note.is_starred = starred;
    backend.update_note(note).await
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn reorder_note
pub async fn reorder_note(
    backend: &dyn StorageBackend,
    id: Uuid,
    new_sort_key: u32,
) -> Result<Note, StorageError> {
    let mut note = read_live_note(backend, id).await?;
    let valid = if is_inbox(note.notebook_id) {
        new_sort_key >= 1
    } else if note.is_pinned {
        (1..=PIN_MAX).contains(&new_sort_key)
    } else {
        new_sort_key >= NORMAL_START
    };
    if !valid {
        let band = if is_inbox(note.notebook_id) {
            ">= 1 (Inbox)".to_string()
        } else if note.is_pinned {
            format!("1..={PIN_MAX} (pinned)")
        } else {
            format!(">= {NORMAL_START} (normal)")
        };
        return Err(StorageError::InvalidInput(format!(
            "sort_key {new_sort_key} is outside the note's band {band}"
        )));
    }
    if note.sort_key == new_sort_key {
        return Ok(note);
    }
    note.sort_key = new_sort_key;
    backend.update_note(note).await
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn resequence_inbox
pub async fn resequence_inbox(backend: &dyn StorageBackend) -> Result<u32, StorageError> {
    let mut token = None;
    let mut next = RESEQUENCE_STEP;
    loop {
        let (page, more) = backend.list_notes_in_notebook(INBOX_ID, 0, token).await?;
        for mut note in page {
            if note.effective_sort_key() != next {
                note.sort_key = next;
                backend.update_note(note).await?;
            }
            next = next.saturating_add(RESEQUENCE_STEP);
        }
        match more {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    Ok(RESEQUENCE_STEP)
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn lowest_free_pinned_key
fn lowest_free_pinned_key(used: &[u32]) -> Option<u32> {
    let mut candidate = 1u32;
    for &key in used {
        if key > candidate {
            break;
        }
        if key == candidate {
            candidate += 1;
        }
    }
    (candidate <= PIN_MAX).then_some(candidate)
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn next_normal_key
fn next_normal_key(profile: &NotebookSortProfile) -> u32 {
    profile
        .max_normal_key
        .map(|max| max.saturating_add(1))
        .unwrap_or(NORMAL_START)
}
```

**What it does** — The key after the last note of the normal band:
`max_normal_key + 1` (saturating), or `NORMAL_START` when the band is empty.

**Dependencies** — `NotebookSortProfile`, `NORMAL_START`.

**Used by** — `place_new_note`, `unpin_note`.

**Repeated context** — none.

---

## fn read_live_note

**Identification** — private `async fn read_live_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError>`;
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
helpers + eleven tests against a real `FsBackend` in a tempdir.

**Code** — container: members documented as sub-blocks below: fn backend, fn create_placed, fn move_note, fn titles, fn ensure_inbox_is_idempotent_and_fixed, fn placement_inbox_top_notebook_bottom, fn pin_unpin_round_trip_and_inbox_rejection, fn reorder_respects_bands, fn starring_is_global_and_never_moves_the_note, fn inbox_top_insert_survives_underflow_by_resequencing, fn moving_a_note_replaces_it_in_the_destination_band, fn moving_a_pinned_note_into_the_inbox_unpins_it, fn a_same_notebook_edit_keeps_the_position, fn the_sort_profile_counts_the_live_notes_the_notebook_cap_reads, fn lowest_free_pinned_key_fills_gaps_and_detects_full.

**What it does** — End-to-end coverage of placement, pinning, reordering,
starring, moves, resequencing, the live-note counting the notes-per-notebook cap
reads, and the pure gap-scan.

**Dependencies** — `super::*`, `storage::fs::FsBackend`,
`storage::{NoteRepository, NotebookRepository}`, `tempfile`, `tokio`.

**Used by** — CI.

**Repeated context** — none.

### fn backend

**Identification** — helper `async fn backend() -> FsBackend`; marker
`// md:mod tests > fn backend`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn backend
    async fn backend() -> FsBackend {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        FsBackend::new(&path).await.unwrap()
    }
```

**What it does** — `FsBackend` in a leaked tempdir (`std::mem::forget` keeps the
directory alive for the test's duration).

### fn create_placed

**Identification** — helper; marker `// md:mod tests > fn create_placed`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn create_placed
    async fn create_placed(be: &FsBackend, title: &str, notebook: Uuid) -> Note {
        let mut note = Note::new(title, "");
        note.notebook_id = notebook;
        place_new_note(be, &mut note).await.unwrap();
        be.create_note(note).await.unwrap()
    }
```

**What it does** — Creates a note through the daemon's placement rule
(`place_new_note` then `create_note`) — what every API create does.

### fn move_note

**Identification** — helper; marker `// md:mod tests > fn move_note`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn move_note
    async fn move_note(be: &FsBackend, id: Uuid, dest: Uuid) -> Note {
        let mut note = be.read_note(id).await.unwrap();
        let current = note.notebook_id;
        note.notebook_id = dest;
        reconcile_notebook_move(be, current, &mut note)
            .await
            .unwrap();
        be.update_note(note).await.unwrap()
    }
```

**What it does** — Moves a note to `dest` through the daemon's generic-update
path (read → set `notebook_id` → `reconcile_notebook_move` → `update_note`),
the sequence both API surfaces run.

### fn titles

**Identification** — helper `fn titles(page: &[Note]) -> Vec<&str>`; marker
`// md:mod tests > fn titles`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn titles
    fn titles(page: &[Note]) -> Vec<&str> {
        page.iter().map(|n| n.title.as_str()).collect()
    }
```

**What it does** — Projects a page to its titles for order assertions.

### fn ensure_inbox_is_idempotent_and_fixed

**Identification** — tokio test; marker
`// md:mod tests > fn ensure_inbox_is_idempotent_and_fixed`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn ensure_inbox_is_idempotent_and_fixed
    #[tokio::test]
    async fn ensure_inbox_is_idempotent_and_fixed() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        ensure_inbox(&be).await.unwrap();
        let inbox = be.read_notebook(INBOX_ID).await.unwrap();
        assert_eq!(inbox.title, INBOX_TITLE);
        assert_eq!(inbox.id, INBOX_ID);
    }
```

**What it does** — Two `ensure_inbox` calls; the notebook exists with
`INBOX_ID`/`INBOX_TITLE`.

### fn placement_inbox_top_notebook_bottom

**Identification** — tokio test; marker
`// md:mod tests > fn placement_inbox_top_notebook_bottom`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn placement_inbox_top_notebook_bottom
    #[tokio::test]
    async fn placement_inbox_top_notebook_bottom() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();

        create_placed(&be, "first", INBOX_ID).await;
        create_placed(&be, "second", INBOX_ID).await;
        create_placed(&be, "third", INBOX_ID).await;
        let (page, _) = be.list_notes_in_notebook(INBOX_ID, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["third", "second", "first"]);

        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let a = create_placed(&be, "a", nb.id).await;
        let b = create_placed(&be, "b", nb.id).await;
        assert_eq!(a.sort_key, NORMAL_START);
        assert_eq!(b.sort_key, NORMAL_START + 1);
        let (page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["a", "b"]);
    }
```

**What it does** — Three Inbox creates list newest-first (top-insert); two
notebook creates land at `NORMAL_START`, `NORMAL_START + 1` and list in creation
order.

### fn pin_unpin_round_trip_and_inbox_rejection

**Identification** — tokio test; marker
`// md:mod tests > fn pin_unpin_round_trip_and_inbox_rejection`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn pin_unpin_round_trip_and_inbox_rejection
    #[tokio::test]
    async fn pin_unpin_round_trip_and_inbox_rejection() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let a = create_placed(&be, "a", nb.id).await;
        let b = create_placed(&be, "b", nb.id).await;

        let pinned = pin_note(&be, b.id).await.unwrap();
        assert!(pinned.is_pinned);
        assert_eq!(pinned.sort_key, 1);
        let (page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["b", "a"], "pinned band lists first");

        let unpinned = unpin_note(&be, b.id).await.unwrap();
        assert!(!unpinned.is_pinned);
        assert!(unpinned.sort_key > a.effective_sort_key());
        let (page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(
            titles(&page),
            ["a", "b"],
            "unpin appends to the normal band"
        );

        let inbox_note = create_placed(&be, "inbox", INBOX_ID).await;
        let err = pin_note(&be, inbox_note.id).await.unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)), "got: {err:?}");
    }
```

**What it does** — Pin moves a note to key 1 and lists it first; unpin appends
it to the normal band; pinning an Inbox note is `InvalidInput`.

### fn reorder_respects_bands

**Identification** — tokio test; marker
`// md:mod tests > fn reorder_respects_bands`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn reorder_respects_bands
    #[tokio::test]
    async fn reorder_respects_bands() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let a = create_placed(&be, "a", nb.id).await;
        let b = create_placed(&be, "b", nb.id).await;

        reorder_note(&be, b.id, NORMAL_START).await.unwrap();
        reorder_note(&be, a.id, NORMAL_START + 5).await.unwrap();
        let (page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["b", "a"]);

        let err = reorder_note(&be, a.id, 5).await.unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)));
        let pinned = pin_note(&be, a.id).await.unwrap();
        let err = reorder_note(&be, pinned.id, NORMAL_START)
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)));
        reorder_note(&be, pinned.id, 42).await.unwrap();
        assert_eq!(be.read_note(a.id).await.unwrap().sort_key, 42);
    }
```

**What it does** — Normal-band reorder changes listing order; a normal note
cannot take a pinned key and vice versa (`InvalidInput` both ways); a pinned
note reorders within `1..=PIN_MAX`.

### fn starring_is_global_and_never_moves_the_note

**Identification** — tokio test; marker
`// md:mod tests > fn starring_is_global_and_never_moves_the_note`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn starring_is_global_and_never_moves_the_note
    #[tokio::test]
    async fn starring_is_global_and_never_moves_the_note() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let in_inbox = create_placed(&be, "inbox note", INBOX_ID).await;
        let in_nb = create_placed(&be, "nb note", nb.id).await;
        create_placed(&be, "unstarred", nb.id).await;

        let starred = star_note(&be, in_inbox.id).await.unwrap();
        assert_eq!(starred.sort_key, in_inbox.sort_key, "star never moves");
        star_note(&be, in_nb.id).await.unwrap();

        let (page, _) = be.list_starred_notes(0, None).await.unwrap();
        let mut got: Vec<&str> = titles(&page);
        got.sort_unstable();
        assert_eq!(got, ["inbox note", "nb note"]);

        unstar_note(&be, in_inbox.id).await.unwrap();
        let (page, _) = be.list_starred_notes(0, None).await.unwrap();
        assert_eq!(titles(&page), ["nb note"]);
    }
```

**What it does** — Stars notes in the Inbox and a notebook: `sort_key`
unchanged, `list_starred_notes` spans notebooks, unstar removes from the list.

### fn inbox_top_insert_survives_underflow_by_resequencing

**Identification** — tokio test; marker
`// md:mod tests > fn inbox_top_insert_survives_underflow_by_resequencing`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn inbox_top_insert_survives_underflow_by_resequencing
    #[tokio::test]
    async fn inbox_top_insert_survives_underflow_by_resequencing() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let first = create_placed(&be, "old-top", INBOX_ID).await;
        reorder_note(&be, first.id, 1).await.unwrap();

        let newcomer = create_placed(&be, "new-top", INBOX_ID).await;
        assert!(newcomer.sort_key >= 1, "never the 0 sentinel");
        let (page, _) = be.list_notes_in_notebook(INBOX_ID, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["new-top", "old-top"]);
    }
```

**What it does** — Pushes the Inbox head to key 1, creates another note: the
newcomer never gets the 0 sentinel and lists on top (resequencing happened
underneath).

### fn moving_a_note_replaces_it_in_the_destination_band

**Identification** — tokio test; marker
`// md:mod tests > fn moving_a_note_replaces_it_in_the_destination_band`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn moving_a_note_replaces_it_in_the_destination_band
    #[tokio::test]
    async fn moving_a_note_replaces_it_in_the_destination_band() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();

        let first = create_placed(&be, "first", INBOX_ID).await;
        let second = create_placed(&be, "second", INBOX_ID).await;
        assert_eq!(first.sort_key, NORMAL_START);
        assert_eq!(second.sort_key, NORMAL_START - 1, "top-insert lands at 999");

        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        create_placed(&be, "existing", nb.id).await;

        let moved = move_note(&be, second.id, nb.id).await;
        assert_eq!(moved.notebook_id, nb.id);
        assert!(!moved.is_pinned, "a moved note is never auto-pinned");
        assert!(
            moved.sort_key >= NORMAL_START,
            "re-placed into the normal band, not the pinned range (got {})",
            moved.sort_key
        );

        let (nb_page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(titles(&nb_page), ["existing", "second"]);
        let (inbox_page, _) = be.list_notes_in_notebook(INBOX_ID, 0, None).await.unwrap();
        assert_eq!(titles(&inbox_page), ["first"], "gone from the Inbox");
    }
```

**What it does** — An Inbox note whose key (999, from top-insertion) falls
inside the pinned numeric range moves to a notebook: it is re-placed in the
normal band (never auto-pinned) and leaves the Inbox.

### fn moving_a_pinned_note_into_the_inbox_unpins_it

**Identification** — tokio test; marker
`// md:mod tests > fn moving_a_pinned_note_into_the_inbox_unpins_it`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn moving_a_pinned_note_into_the_inbox_unpins_it
    #[tokio::test]
    async fn moving_a_pinned_note_into_the_inbox_unpins_it() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let n = create_placed(&be, "n", nb.id).await;
        assert!(pin_note(&be, n.id).await.unwrap().is_pinned);

        let moved = move_note(&be, n.id, INBOX_ID).await;
        assert!(!moved.is_pinned);
        assert_eq!(moved.notebook_id, INBOX_ID);
        let (inbox_page, _) = be.list_notes_in_notebook(INBOX_ID, 0, None).await.unwrap();
        assert_eq!(titles(&inbox_page), ["n"]);
    }
```

**What it does** — A pinned notebook note moved into the Inbox arrives
unpinned.

### fn a_same_notebook_edit_keeps_the_position

**Identification** — tokio test; marker
`// md:mod tests > fn a_same_notebook_edit_keeps_the_position`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn a_same_notebook_edit_keeps_the_position
    #[tokio::test]
    async fn a_same_notebook_edit_keeps_the_position() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let a = create_placed(&be, "a", nb.id).await;
        create_placed(&be, "b", nb.id).await;

        let mut edit = be.read_note(a.id).await.unwrap();
        let key_before = edit.sort_key;
        edit.title = "a-edited".into();
        reconcile_notebook_move(&be, a.notebook_id, &mut edit)
            .await
            .unwrap();
        let saved = be.update_note(edit).await.unwrap();
        assert_eq!(
            saved.sort_key, key_before,
            "no re-placement on a plain edit"
        );

        let (page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["a-edited", "b"]);
    }
```

**What it does** — A title-only edit through the reconcile path keeps
`sort_key` — no re-placement on a plain edit.

### fn the_sort_profile_counts_the_live_notes_the_notebook_cap_reads

**Identification** — `#[tokio::test]` integration-style unit test; marker
`// md:mod tests > fn the_sort_profile_counts_the_live_notes_the_notebook_cap_reads`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn the_sort_profile_counts_the_live_notes_the_notebook_cap_reads
    #[tokio::test]
    async fn the_sort_profile_counts_the_live_notes_the_notebook_cap_reads() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        assert_eq!(be.notebook_sort_profile(nb.id).await.unwrap().live_notes, 0);

        create_placed(&be, "a", nb.id).await;
        let b = create_placed(&be, "b", nb.id).await;
        assert_eq!(be.notebook_sort_profile(nb.id).await.unwrap().live_notes, 2);

        move_note(&be, b.id, INBOX_ID).await;
        assert_eq!(be.notebook_sort_profile(nb.id).await.unwrap().live_notes, 1);
        assert_eq!(
            be.notebook_sort_profile(INBOX_ID).await.unwrap().live_notes,
            1
        );

        be.delete_note(b.id).await.unwrap();
        assert_eq!(
            be.notebook_sort_profile(INBOX_ID).await.unwrap().live_notes,
            0
        );

        assert!(format::check_notebook_capacity(format::MAX_NOTES_PER_NOTEBOOK - 1).is_ok());
        let full = format::check_notebook_capacity(format::MAX_NOTES_PER_NOTEBOOK).unwrap_err();
        assert_eq!(full.code(), format::CODE_NOTEBOOK_FULL);
        assert!(matches!(
            StorageError::from(full),
            StorageError::TooLarge(_)
        ));
    }
```

**What it does** — Proves the input to the notes-per-notebook cap is the right
number, which is the half of the gate a boundary test cannot reach: creating 2²⁴
notes to hit the real limit is not a test anyone can run, so the limit's edge is
pinned in `format.rs` and the **counting** is pinned here. It asserts
`live_notes` is `0` for an empty notebook, tracks creates, follows a **move**
(the destination gains, the source loses — the path `reconcile_notebook_move`
takes), and drops back to `0` after a soft delete, proving tombstones are
excluded. The tail re-asserts that the value the profile feeds
`check_notebook_capacity` produces `CODE_NOTEBOOK_FULL` and converts to
`StorageError::TooLarge`, the error the daemon turns into HTTP 413.

**Dependencies** — `NotebookSortProfile::live_notes`, `create_placed`,
`move_note`, `delete_note`, `format::check_notebook_capacity`,
`LimitViolation::code`, `StorageError::TooLarge`.

**Used by** — CI only.

**Repeated context** — a soft-deleted note is not live: both backends build the
sort profile from rows with no `deleted_at`, and the cap counts live notes only.

### fn lowest_free_pinned_key_fills_gaps_and_detects_full

**Identification** — unit test; marker
`// md:mod tests > fn lowest_free_pinned_key_fills_gaps_and_detects_full`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn lowest_free_pinned_key_fills_gaps_and_detects_full
    #[test]
    fn lowest_free_pinned_key_fills_gaps_and_detects_full() {
        assert_eq!(lowest_free_pinned_key(&[]), Some(1));
        assert_eq!(lowest_free_pinned_key(&[1, 2, 3]), Some(4));
        assert_eq!(lowest_free_pinned_key(&[1, 3]), Some(2));
        assert_eq!(lowest_free_pinned_key(&[2, 3]), Some(1));
        let full: Vec<u32> = (1..=PIN_MAX).collect();
        assert_eq!(lowest_free_pinned_key(&full), None);
        let almost: Vec<u32> = (1..PIN_MAX).collect();
        assert_eq!(lowest_free_pinned_key(&almost), Some(PIN_MAX));
    }
```

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
| 1 | `Overview` | `// md:Overview` |
| 2 | `INBOX_ID` | `// md:INBOX_ID` |
| 3 | `INBOX_TITLE` | `// md:INBOX_TITLE` |
| 4 | `PIN_MAX` | `// md:PIN_MAX` |
| 5 | `MAX_PINNED` | `// md:MAX_PINNED` |
| 6 | `NORMAL_START` | `// md:NORMAL_START` |
| 7 | `RESEQUENCE_STEP` | `// md:RESEQUENCE_STEP` |
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
| 22 | `mod tests` (container) | `// md:mod tests` |
| 23 | `fn backend` | `// md:mod tests > fn backend` |
| 24 | `fn create_placed` | `// md:mod tests > fn create_placed` |
| 25 | `fn move_note` | `// md:mod tests > fn move_note` |
| 26 | `fn titles` | `// md:mod tests > fn titles` |
| 27 | `fn ensure_inbox_is_idempotent_and_fixed` | `// md:mod tests > fn ensure_inbox_is_idempotent_and_fixed` |
| 28 | `fn placement_inbox_top_notebook_bottom` | `// md:mod tests > fn placement_inbox_top_notebook_bottom` |
| 29 | `fn pin_unpin_round_trip_and_inbox_rejection` | `// md:mod tests > fn pin_unpin_round_trip_and_inbox_rejection` |
| 30 | `fn reorder_respects_bands` | `// md:mod tests > fn reorder_respects_bands` |
| 31 | `fn starring_is_global_and_never_moves_the_note` | `// md:mod tests > fn starring_is_global_and_never_moves_the_note` |
| 32 | `fn inbox_top_insert_survives_underflow_by_resequencing` | `// md:mod tests > fn inbox_top_insert_survives_underflow_by_resequencing` |
| 33 | `fn moving_a_note_replaces_it_in_the_destination_band` | `// md:mod tests > fn moving_a_note_replaces_it_in_the_destination_band` |
| 34 | `fn moving_a_pinned_note_into_the_inbox_unpins_it` | `// md:mod tests > fn moving_a_pinned_note_into_the_inbox_unpins_it` |
| 35 | `fn a_same_notebook_edit_keeps_the_position` | `// md:mod tests > fn a_same_notebook_edit_keeps_the_position` |
| 36 | `fn the_sort_profile_counts_the_live_notes_the_notebook_cap_reads` | `// md:mod tests > fn the_sort_profile_counts_the_live_notes_the_notebook_cap_reads` |
| 37 | `fn lowest_free_pinned_key_fills_gaps_and_detects_full` | `// md:mod tests > fn lowest_free_pinned_key_fills_gaps_and_detects_full` |