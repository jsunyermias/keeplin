# `search.rs` — daemon full-text search

## Purpose

Full-text search over the user's notes, run **in the daemon** rather than on keeplin-srv. The server
may only ever hold ciphertext (at-rest encryption is a client concern), so it cannot index note
content; the daemon sits **above** the `EncryptedBackend` layer and sees plaintext. This module keeps
an in-memory SQLite FTS5 index of every live note and answers queries over `GET /api/search`.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `SearchQuery` | struct | a parsed request: free `text` + structured filters (notebook, to-do state, due/updated ranges, starred, pinned) + `limit` |
| `SearchHandle` | struct | cloneable query view handed to the REST surface; `search(&query) -> Vec<Uuid>` |
| `Index` | struct (private) | the in-memory FTS5 connection + maintenance ops |

## What is indexed

One FTS5 row per live note:

- **Full-text columns**: `title`, `body`, and the denormalised **tag names** and **notebook name**
  (read back through the plaintext top of the stack when the note is indexed).
- **Filter columns** (`UNINDEXED`): `notebook_id`, `is_todo`, `is_completed`, `todo_due`,
  `is_starred`, `is_pinned`, `updated_at`. Timestamps are stored as RFC3339 (UTC), which sorts
  lexicographically = chronologically, so range filters are plain string comparisons.

## How the index stays current

- **Rebuild at startup** (inside the spawned task, so startup is not blocked): scan the store via
  `backend.list_notes` — reads go through the decorator stack and come back decrypted.
- **Live updates**: drain the same `broadcast::Sender<Change>` the WebSocket feed uses. Because
  `EventBackend` sits outside encryption, those `Change`s carry plaintext. `NoteCreate/Update` reindex
  the note, `NoteDelete` removes it, `NoteTagAdd/Remove` reindex the one affected note, and a
  notebook/tag rename or delete triggers a full rebuild (infrequent, and it changes denormalised names
  in bulk).
- **On lag** (`RecvError::Lagged`, the channel dropped changes) the index rebuilds wholesale.
- The index is **ephemeral** (`:memory:`): a restart just rebuilds from the store, so there is no
  stale-on-disk state to reconcile.

## Query building

`fts_match` turns the free text into a safe FTS5 `MATCH`: each whitespace token becomes a quoted
prefix term (`"tok"*`), so user input can never inject FTS5 operators and typing a prefix matches. The
structured filters are inlined into the SQL — every one is server-validated (parsed into a
`Uuid`/`bool`/timestamp and re-serialised), so it is injection-safe; only the free text is a bound
parameter. With a text query, results are ordered by FTS `rank`; without one, by `updated_at DESC`.

## Design notes

- **Why the daemon, not keeplin-srv**: the chosen architecture (see the PR) — the server can't read
  encrypted content, and the daemon holds the key. The trade-off is that search is **per-device** (it
  covers the notes this daemon has), which is inherent to searching decrypted content client-side.
- **Why a decorator-free design**: instead of a full `StorageBackend` decorator (lots of delegation),
  the index consumes the existing event stream + an initial rebuild — the same plaintext source the
  live feed already uses.
- **Out of scope**: "users with access" (shares) is a keeplin-srv concept not present in the client
  `Note` model, so it is not a filter here.

## Related files

- `rest.md` — the `GET /api/search` endpoint and `AppState.search`.
- `event_backend.md` — the plaintext `Change` stream the index consumes.
- `main.rs` — builds the stack and calls `search::start` when the HTTP surface is enabled.
- `keeplin-core/src/encryption.md` — why the index must live above this layer.
