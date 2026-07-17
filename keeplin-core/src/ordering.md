# `ordering.rs` — the Inbox, pinning, manual ordering, and starring

## Purpose

This module implements the note-organisation layer requested in issues #49–#55: a default
**Inbox** notebook, per-notebook **pinning**, **manual ordering** by numeric `sort_key`, and
a global **star** flag. It is a set of free functions over `&dyn StorageBackend` — not a
backend or a decorator — so the daemon's gRPC and REST surfaces call them the same way they
call `linking`'s alias helpers.

The underlying note fields (`is_pinned`, `is_starred`, `sort_key`, and the now-non-optional
`notebook_id`) live in `models.rs`; the query methods (`list_notes_in_notebook`,
`list_starred_notes`, `notebook_sort_profile`) live on `StorageBackend`. This module holds
only the **placement rules** that decide what those fields should be.

## Key constants

The module defines no types — only free functions (see [Public API](#public-api)) and the
constants that fix the band layout and the Inbox's identity:

| Constant | Value | Description |
|----------|-------|-------------|
| `INBOX_ID` | nil UUID | The Inbox's fixed id, identical on every device |
| `INBOX_TITLE` | `"Pizarra"` | The Inbox notebook's title |
| `PIN_MAX` / `MAX_PINNED` | `999` | Highest pinned key = maximum pinned notes per notebook |
| `NORMAL_START` | `1000` | First key of the normal (unpinned) band |

## Public API

### The Inbox / board (`Pizarra`)

The Inbox is the system notebook that acts as the default **board** for unfiled notes. It has
no pinning and is ordered manually as one flat band. It also sits **outside the linking graph**
(see `linking.md`): notes here carry no alias, emit no links, and are never a link target — so
moving a note into the Inbox clears its alias and outgoing links, and moving it out lets it
claim an alias again. `is_inbox` is the predicate `LinkingBackend` uses for all of this.

| Function | Signature (conceptual) | Description |
|----------|------------------------|-------------|
| `ensure_inbox` | `async fn(&dyn StorageBackend) -> Result<(), StorageError>` | Creates the Inbox notebook if it does not exist yet. Idempotent; called at daemon startup. |
| `is_inbox` | `fn(Uuid) -> bool` | Returns `true` for the fixed nil UUID used by the Inbox. The API surfaces use this to refuse deleting the system notebook. |
| `place_new_note` | `async fn(&dyn StorageBackend, &mut Note) -> Result<(), StorageError>` | Gives a brand-new note its initial `sort_key`. In the Inbox it inserts at the top (`min - 1`, resequencing first if the space above is exhausted). In a normal notebook it appends to the normal band. Honours an already-set (`!= 0`) caller-chosen key. |
| `resequence_inbox` | `async fn(&dyn StorageBackend) -> Result<u32, StorageError>` | Renumber every live Inbox note to `1000, 2000, …` in current order and returns the new minimum key. Triggered automatically when top-insertion would hit the `0` sentinel. |

### Pinning

| Function | Signature (conceptual) | Description |
|----------|------------------------|-------------|
| `pin_note` | `async fn(&dyn StorageBackend, id: Uuid) -> Result<Note, StorageError>` | Moves a note into its notebook's pinned band (`1..=999`) at the lowest free key. Returns the updated note. Fails with `InvalidInput` for Inbox notes (no pinning in the Inbox) and with `Conflict` when the notebook already has 999 pinned notes. Idempotent on already-pinned notes. |
| `unpin_note` | `async fn(&dyn StorageBackend, id: Uuid) -> Result<Note, StorageError>` | Moves a pinned note to the end of its notebook's normal band. Returns the updated note. Idempotent on non-pinned notes. |
| `reorder_note` | `async fn(&dyn StorageBackend, id: Uuid, new_sort_key: u32) -> Result<Note, StorageError>` | Gives a note a new manual position **within its current band**: pinned notes accept `1..=999`, normal notes accept `>= 1000`, and Inbox notes accept `>= 1`. Returns the updated note. Fails with `InvalidInput` for out-of-band keys. Idempotent when the key is unchanged. |
| `reconcile_notebook_move` | `async fn(&dyn StorageBackend, current_notebook_id: Uuid, &mut Note) -> Result<(), StorageError>` | Call from the generic note-update path when the caller has changed `note.notebook_id`. Resets `is_pinned` and `sort_key`, then re-places the note in the destination (top of the Inbox or end of the normal band). A no-op when the notebook does not change, so plain edits preserve manual position. |

### Starring

| Function | Signature (conceptual) | Description |
|----------|------------------------|-------------|
| `star_note` | `async fn(&dyn StorageBackend, id: Uuid) -> Result<Note, StorageError>` | Sets the global `is_starred` flag on a note. The note's notebook, `sort_key`, and pinned state never change. Idempotent. |
| `unstar_note` | `async fn(&dyn StorageBackend, id: Uuid) -> Result<Note, StorageError>` | Clears the global `is_starred` flag. Idempotent. |

### Listing starred notes

`StorageBackend::list_starred_notes(page_size, page_token)` returns every live starred note
across all notebooks in `(created_at, id)` order. This is a backend method (not a function in
`ordering.rs`) because it is a read-only query; both `DbBackend` and `FsBackend` implement it
natively. See `storage/backend.md`.

## The band model & placement rules

Within a notebook, notes order by `(effective sort_key ASC, id ASC)`:

- `1..=999` is the **pinned** band (at most 999 notes; rendered as `0.001–0.999`), shown at
  the top. `pin_note` picks the lowest free key via `lowest_free_pinned_key`; `unpin_note`
  appends to the normal band.
- `>= 1000` is the **normal** band. New notes append after the current last note
  (`max_normal_key + 1`).
- `sort_key == 0` is the **legacy "never positioned" sentinel** — every note written before
  this feature. `Note::effective_sort_key` maps it to `NORMAL_START` so old notes sort at the
  start of the normal band with no data rewrite.

**The Inbox** is one flat, manually ordered band with **no pinning**. New notes insert at the
**top** (`min(existing) - 1`). When that would reach the `0` sentinel, `resequence_inbox`
renumbers everything to `1000, 2000, …` first, buying room for many more top-inserts.

**Placement reads a summary, not the notebook.** Every placement decision consults one
`NotebookSortProfile` (`{ pinned_keys, min_key, max_normal_key }`) that each backend computes
natively (an indexed scan of sort keys, never the note bodies), so pinning or creating a note
never materialises the whole notebook.

### Invariants & edge cases

- **A note is never "unfiled".** `notebook_id` is a `Uuid`, never `Option`; a note with no
  chosen notebook has `notebook_id == INBOX_ID` (nil). Old records with `null`/missing
  `notebook_id` deserialize into the Inbox (see `models.rs`).
- **Pinned state is per-notebook.** A key in `1..=999` only means "pinned" *within that
  notebook*. `reconcile_notebook_move` therefore resets `is_pinned` and `sort_key` on a move
  and re-places the note — otherwise an Inbox note with key `999` would land inside the
  destination's pinned range without being pinned (the #53 loose end).
- **The Inbox cannot be pinned into.** `pin_note` rejects an Inbox note with
  `StorageError::InvalidInput`; the daemon maps that to `400` / `INVALID_ARGUMENT`.
- **Reorder stays in band.** `reorder_note` validates the new key against the note's current
  band (pinned `1..=999`, normal `>= 1000`, Inbox `>= 1`) and rejects out-of-band keys.
- **Duplicate keys are allowed.** Two notes may share a key; `id` breaks the tie
  deterministically, so ordering is always well-defined.

## How ordering syncs

The operations are plain read-modify-write through `update_note`, so version vectors,
encryption, link derivation, and the live-change feed all apply unchanged, and every move
**syncs like any other note edit**. `sort_key`/`is_pinned`/`is_starred` are ordinary note
fields resolved by whole-note version-vector resolution — there is no separate ordering CRDT.
Two devices pinning concurrently may pick the same key; ordering stays deterministic and the
next reorder can separate them. Old peers ignore the new fields (serde/protobuf defaults) and
their writes keep `sort_key = 0`.

## Design notes

- **Free functions, not a decorator.** Placement is policy the *daemon* applies on create and
  the ordering RPCs apply on demand — it must not fire on every low-level `update_note` (the
  pin/reorder ops call `update_note` directly and set the fields deliberately). Keeping the
  logic here, above the backend, makes that boundary explicit.
- **Integer bands over fractional keys.** A `u32` band with a resequence fallback is simpler
  to reason about (and to store/query/index) than fractional ranks, at the cost of an
  occasional Inbox renumber — which is bounded and rare thanks to the `1000` spacing.

## Graph context

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

**Invariants** (restated on purpose; a change to this file must keep these true)

- Free functions over `&dyn StorageBackend` — not a backend or decorator; gRPC and REST must call the same functions for identical behaviour.
- The Inbox is the system notebook with the nil UUID; every note lives in exactly one notebook, defaulting to the Inbox.
- Pinned band is `sort_key` 1–999 (max 999 pinned); manual ordering is by numeric `sort_key` within a notebook.

## Related files

- `keeplin-core/src/models.rs` — the `Note` fields (`is_pinned`, `is_starred`, `sort_key`,
  `notebook_id`) and `effective_sort_key` / `DEFAULT_SORT_KEY`.
- `keeplin-core/src/storage/backend.rs` — `list_notes_in_notebook`, `list_starred_notes`,
  `notebook_sort_profile`, and the `NotebookSortProfile` summary.
- `keeplin-daemon/src/server.rs`, `rest.rs` — the RPCs/routes that call these functions, and
  the generic update path that calls `reconcile_notebook_move`.
- `README.md` — "Pinning, ordering & the Inbox" (the user-facing description).
