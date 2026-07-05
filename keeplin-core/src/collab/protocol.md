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

## Related files

- `collab/mod.md` — sends `CollabClientMsg`, handles `CollabServerMsg`.
- `collab/state.md` — consumes `LineOp`/snapshots to maintain the body.
- `keeplin-core/src/storage/note_log.md` — `VersionVector`, the shared resolution primitive.
