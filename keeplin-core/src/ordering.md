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

## Structure

| Item | Description |
|------|-------------|
| `INBOX_ID` / `INBOX_TITLE` | The Inbox's fixed identity: the **nil UUID** and the title `"Pizarra"`, identical on every device |
| `PIN_MAX` (999) / `MAX_PINNED` | Highest pinned key = maximum pinned notes per notebook |
| `NORMAL_START` (1000) | First key of the normal (unpinned) band |
| `ensure_inbox` | Create the Inbox notebook if absent; idempotent, called at daemon startup |
| `is_inbox` | Whether an id is the Inbox (the surfaces use it to refuse deleting it) |
| `place_new_note` | Assign a brand-new note its initial `sort_key` before `create_note` |
| `pin_note` / `unpin_note` | Move a note into / out of the pinned band |
| `star_note` / `unstar_note` | Toggle the global star (never moves the note) |
| `reorder_note` | Change a note's key **within its current band** |
| `reconcile_notebook_move` | On a generic update that changes `notebook_id`, re-place the note in the destination band |
| `resequence_inbox` | Renumber the Inbox to `1000, 2000, …` when top-insertion runs out of room |

## How it works

**The band model.** Within a notebook, notes order by `(effective sort_key ASC, id ASC)`:

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

## Invariants & edge cases

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

## Concurrency & sync

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

## Related files

- `keeplin-core/src/models.rs` — the `Note` fields (`is_pinned`, `is_starred`, `sort_key`,
  `notebook_id`) and `effective_sort_key` / `DEFAULT_SORT_KEY`.
- `keeplin-core/src/storage/backend.rs` — `list_notes_in_notebook`, `list_starred_notes`,
  `notebook_sort_profile`, and the `NotebookSortProfile` summary.
- `keeplin-daemon/src/server.rs`, `rest.rs` — the RPCs/routes that call these functions, and
  the generic update path that calls `reconcile_notebook_move`.
- `README.md` — "Pinning, ordering & the Inbox" (the user-facing description).
