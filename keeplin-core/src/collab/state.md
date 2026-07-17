# `collab/state.rs` — client line state and body↔lines translation

## Purpose

The in-memory mirror of one collaborative note's server-side line entities, plus the translation
between the flat markdown **body** frontends edit and the line **ops** the channel speaks. Pure logic,
no I/O: materialise a body from lines, apply an incoming server op, and diff a locally edited body
into the ops that reproduce it.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `NoteLines` | struct | one note's mirror: `order: Vec<Uuid>`, `lines: HashMap<Uuid, LineSnapshot>`, `vv` |

Nothing here is durable — `NoteLines` is rebuilt from each `Welcome` snapshot on every (re)connect.

## Public API

| Function | Description |
|----------|-------------|
| `NoteLines::from_snapshot(snapshot)` | build the mirror from a `Welcome` `NoteLinesSnapshot` |
| `materialize() -> String` | the flat body: live (non-tombstoned) lines, in `order`, joined with `\n` |
| `apply(op: &LineOp)` | apply one already-resolved server op to the mirror |
| `diff_body(body, device) -> Vec<LineOp>` | diff the current live lines against an edited body; returns the ops and applies them locally (optimistic echo) |

## `apply` — trusting the server

Server ops arrive **already validated and in `server_seq` order**, so `apply` applies them directly
without re-resolving: insert at the anchor's position, update/tombstone by id, or splice a `Move`.
Every branch merges the op's `vv` into the relevant entity's vector so the mirror's causal position
tracks the server's.

## `diff_body` — the load-bearing algorithm

Turns a freshly edited flat body into a minimal op set:

1. Split the body on `\n` (empty body → no lines).
2. Trim the common **prefix** and **suffix** of unchanged lines against the current live lines.
3. In the changed middle, pair old and new lines **positionally**: differing pairs become `Update`s,
   surplus old lines become `Delete`s (tombstones), surplus new lines become `Insert`s after the last
   paired/prefix line.
4. Apply every generated op to the mirror before returning, so the local view matches what was sent.

**The insert invariant** is the subtle part: inserts resolve against the *order* entity, so several
inserts in one edit must carry **strictly increasing** vectors. `diff_body` clones the order vv once
(`order_vv`) and `bump`s it per insert — a single edit adding N lines emits N ops whose order
components strictly increase. Cloning `self.vv` fresh for each insert instead would make every op
carry the same vector, and the server would drop all but the first as replays (this was a real bug —
see keeplin-srv PR #90).

## Helpers

| Function | Description |
|----------|-------------|
| `merge_into(target, other)` | element-wise max of two version vectors (least upper bound) |
| `bump(vv, device)` | increment `device`'s component by one |

## Design notes

- **Positional pairing, not a full LCS diff**: cheap and good enough for line editing; a moved block
  shows up as deletes+inserts rather than a `Move` (the explicit `Move` op is used by reorder UX, not
  inferred here).
- **Tombstones stay in the mirror** so a later reconnect's snapshot and this mirror converge; they are
  simply skipped by `materialize` and `live`.
- The `device` argument is the vv actor — the **device** id, matching the protocol's authorship rule.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `NoteLines` — defined here (EXTRACTED; 4 cross-file edge(s))
- `.from_snapshot()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.apply()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.diff_body()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.materialize()` — defined here (EXTRACTED; file-local)
- `.live()` — defined here (EXTRACTED; file-local)
- `merge_into()` — defined here (EXTRACTED; file-local)
- `bump()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/collab/protocol.rs` — collaborative channel wire types (EXTRACTED: references×4; e.g. `LineSnapshot`, `NoteLinesSnapshot`, `LineOp`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/collab/mod.rs` — client of the keeplin-srv collaborative channel (EXTRACTED: imports_from×1, references×1; e.g. `mod.rs`, `Shared`)
- `keeplin-core/tests/collab_client.rs` — collaborative client tests (state machine + mock server e2e) (EXTRACTED: imports_from×1; e.g. `collab_client.rs`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Pure logic, no I/O — must stay unit-testable in isolation.
- Applying the same ops in the same order on any mirror must materialise byte-identical bodies (deterministic replay).
- Diffing a body then applying the resulting ops to the mirror must reproduce exactly that body.

## Related files

- `collab/protocol.md` — `LineOp`, `LineSnapshot`, `NoteLinesSnapshot`, `VersionVector`.
- `collab/mod.md` — calls `diff_body` on local edits and `apply`/`from_snapshot` on inbound frames.
- `keeplin-core/src/storage/note_log.md` — the version-vector semantics shared with the rest of the app.
