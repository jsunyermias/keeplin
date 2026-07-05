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

## Related files

- `collab/protocol.md` — `LineOp`, `LineSnapshot`, `NoteLinesSnapshot`, `VersionVector`.
- `collab/mod.md` — calls `diff_body` on local edits and `apply`/`from_snapshot` on inbound frames.
- `keeplin-core/src/storage/note_log.md` — the version-vector semantics shared with the rest of the app.
