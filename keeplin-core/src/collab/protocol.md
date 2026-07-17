# `collab/protocol.rs` — collaborative channel wire types

## Purpose

The JSON wire types of keeplin-srv's collaborative channel (`GET /api/ws`), mirroring the server's
own `protocol.rs`. Pure type definitions (serde `Serialize`/`Deserialize`) — no logic. Messages are
tagged with `type`; line operations with `op`; both use `PascalCase` variant names to match the
server.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `LineId` | type alias | `Uuid` — a line's stable identity |
| `Cursor` | struct | a caret: `{ line_id, column }` |
| `LineSnapshot` | struct | one line as a full versioned entity (content, timestamps, `deleted_at` tombstone, `vv`, `last_writer`) |
| `NoteLinesSnapshot` | struct | full note state in `Welcome`: the versioned `order` + every `LineSnapshot` |
| `LineOp` | enum | one line operation: `Insert` / `Update` / `Delete` / `Move` |
| `PresenceInfo` | struct | a participant: `{ user_id, display_name, cursor }` |
| `CollabClientMsg` | enum | client → server: `Join` / `Leave` / `Op` / `Cursor` / `Ack` |
| `CollabServerMsg` | enum | server → client: `Welcome` / `Op` / `Presence` / `Error` |

## The op model

A note is a set of independently versioned **lines** plus a separately versioned **order**. Each
`LineOp` carries a version vector (`vv`) and a `last_writer` — and both the vv component that advances
and `last_writer` are this **device's** id (the concurrency actor in server mode), *not* the user id.
That is what lets the server validate an op's authorship and resolve concurrent edits deterministically.

- `Insert { after_line_id, line_id, content, … }` — a new line after an anchor (or at the head when
  `after_line_id` is `None`). Resolves against the **order** entity.
- `Update { line_id, content, … }` — new content for an existing line.
- `Delete { line_id, deleted_at, … }` — tombstone a line (kept for convergence).
- `Move { line_ids, after_line_id, … }` — reorder lines; resolves against the order entity.

## Message flow

1. Client sends `Join { note_id }`; server replies `Welcome { note_id, snapshot }` — the client
   rebuilds its mirror from the snapshot (no incremental catch-up).
2. Client sends `Op { note_id, ops }`; server validates, resolves, persists, and fans out
   `Op { server_seq, note_id, user_id, ops }` to every participant (sender included) in `server_seq`
   order.
3. `Cursor { note_id, cursor }` → server rebroadcasts `Presence { note_id, users }`.
4. `Error { code, message }` reports a rejected op (e.g. `forbidden`, `bad_writer`).

## Design notes

- This file is a **faithful mirror** of the server's `protocol.rs`; the two must stay in lockstep, so
  changes here are meaningless unless the server agrees. The `#[serde(tag = …, rename_all =
  "PascalCase")]` attributes are the contract.
- Snapshots carry tombstoned lines (`deleted_at: Some`) so a reconnecting client converges on deletes
  it may not have seen; `state.rs` filters them out when materialising the body.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `CollabClientMsg` — defined here (EXTRACTED; 4 cross-file edge(s))
- `PresenceInfo` — defined here (EXTRACTED; 3 cross-file edge(s))
- `LineOp` — defined here (EXTRACTED; 2 cross-file edge(s))
- `Cursor` — defined here (EXTRACTED; 1 cross-file edge(s))
- `LineSnapshot` — defined here (EXTRACTED; 1 cross-file edge(s))
- `NoteLinesSnapshot` — defined here (EXTRACTED; 1 cross-file edge(s))
- `CollabServerMsg` — defined here (EXTRACTED; 1 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/collab/mod.rs` — client of the keeplin-srv collaborative channel (EXTRACTED: references×8; e.g. `Shared`, `.presence()`, `.send_cursor()`)
- `keeplin-core/src/collab/state.rs` — client line state and body↔lines translation (EXTRACTED: references×4; e.g. `NoteLines`, `.from_snapshot()`, `.apply()`)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: references×1; e.g. `note_presence()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Pure serde types, no logic; shapes must stay byte-compatible with keeplin-srv's `src/protocol.rs` (messages tagged `type`, ops tagged `op`, PascalCase variants).
- Every op carries its own `vv`, `last_writer`, `updated_at` — the server resolves each op independently.
- A breaking change to these shapes requires bumping `PROTOCOL_VERSION` in `compat.rs` and keeplin-srv together.

## Related files

- `collab/mod.md` — sends `CollabClientMsg`, handles `CollabServerMsg`.
- `collab/state.md` — consumes `LineOp`/snapshots to maintain the body.
- `keeplin-core/src/storage/note_log.md` — `VersionVector`, the shared resolution primitive.
