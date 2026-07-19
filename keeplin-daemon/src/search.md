# `search.rs` — daemon full-text search

Self-contained companion for `keeplin-daemon/src/search.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must
be able to understand it without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

```rust
use std::sync::Arc;

use chrono::{DateTime, Utc};
use keeplin_core::models::{Change, Note};
use keeplin_core::storage::StorageBackend;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;
```

**What it does** — Full-text search **in the daemon**, not on keeplin-srv: the
server may only ever hold ciphertext (at-rest encryption is a client concern),
so it cannot index content; the daemon sits above `EncryptedBackend` and sees
plaintext. `SearchIndex` semantics: an **in-memory SQLite FTS5** index of every
live note — title, body, denormalised tag and notebook names — plus structured
columns (notebook, to-do state, due date, starred, pinned, updated-at) for
filtering. Rebuilt at startup by scanning through the top of the decorator
stack (decrypted reads); kept live by draining the same `Change` broadcast
channel the WebSocket feed uses (plaintext — `EventBackend` sits outside
encryption); a `Lagged` receive triggers a full rebuild. Ephemeral: nothing is
persisted; a restart rebuilds. `SearchHandle` is the cloneable query view for
REST/gRPC.

**Dependencies** — `libsql` (in-memory FTS5), `tokio`, keeplin-core
model/storage types.

**Used by** — `main.rs` (starts it, passes the handle into REST/gRPC state);
the search endpoints in `rest.rs`/`server.rs`.

**Repeated context** — search is the canonical example of "the daemon is the
trust boundary": plaintext-only features live here, never on the server.

---

## SearchQuery

**Identification** — struct deriving `Debug, Clone, Default`; marker
`// md:SearchQuery`.

**What it does** — A parsed search request: `text` (free text against
title/body/tags/notebook; empty = no text filter, recency ordering),
`notebook_id`, `is_todo`, `todo_open` (`Some(true)` = open only,
`Some(false)` = completed only), `is_starred`, `is_pinned`,
`due_after`/`due_before`, `updated_after`/`updated_before`, `limit` (clamped
to `MAX_LIMIT`).

**Used by** — the surfaces build it from query params / proto fields;
`Index::query` consumes it.

---

## MAX_LIMIT

**Identification** — `pub const MAX_LIMIT: usize = 500;` marker
`// md:MAX_LIMIT`.

**What it does** — Upper bound on results per query.

**Used by** — `Index::query`'s clamp.

---

## fn rfc

**Identification** — `fn rfc(ts: DateTime<Utc>) -> String`; marker
`// md:fn rfc`.

**What it does** — RFC3339 UTC (micros, `Z`): lexicographic order equals
chronological, so range filters work as plain string comparisons.

**Used by** — `Index::upsert`/`query`.

---

## fn fts_match

**Identification** — `fn fts_match(text: &str) -> Option<String>`; marker
`// md:fn fts_match`.

**What it does** — Turns free text into an FTS5 `MATCH` expression (`None`
when empty): each whitespace token becomes a quoted prefix term (`"tok"*`,
inner quotes doubled), so user input can never inject FTS5 operators and
typing a prefix matches.

**Used by** — `Index::query`.

**Repeated context** — this quoting is the injection barrier for the only
user-controlled part of the SQL below.

---

## Index

**Identification** — private `struct Index`; marker `// md:Index`.

**What it does** — The in-memory FTS5 index: a `libsql::Connection` plus the
`Database` handle kept alive alongside it (each `:memory:` connection is its
own database). Shared behind `Arc<Mutex<…>>` by the query handle and the
maintenance task.

**Used by** — everything below.

---

## impl Index

**Identification** — inherent impl; marker `// md:impl Index`. Five methods.

### fn open

**Identification** — `async fn open() -> anyhow::Result<Self>`; marker
`// md:impl Index > fn open`.

**What it does** — Creates the `:memory:` database and the `note_fts` FTS5
virtual table: indexed columns `title, body, tags, notebook`; `UNINDEXED`
structured columns `note_id, notebook_id, is_todo, is_completed, todo_due,
is_starred, is_pinned, updated_at`.

### fn remove

**Identification** — marker `// md:impl Index > fn remove`.

**What it does** — Deletes a note's row by id.

### fn upsert

**Identification** —
`async fn upsert(&self, note: &Note, tags: &str, notebook: &str)`; marker
`// md:impl Index > fn upsert`.

**What it does** — FTS5 has no UPDATE: delete then insert the full row
(booleans as `"0"`/`"1"` via `bit`, timestamps via `rfc`, missing due date as
`""`).

### fn clear

**Identification** — marker `// md:impl Index > fn clear`.

**What it does** — Empties the table (before a rebuild).

### fn query

**Identification** —
`async fn query(&self, q: &SearchQuery) -> anyhow::Result<Vec<Uuid>>`; marker
`// md:impl Index > fn query`.

**What it does** — Builds the WHERE from the filters: the only bound
parameter is the `MATCH` expression; every other value is **server-validated**
(parsed into `Uuid`/bool/timestamp and re-serialised) before being inlined,
so inlining is injection-safe. `due_before` additionally excludes the empty
string (notes with no due date). With text: ordered by FTS `rank`; without:
`updated_at DESC`. `LIMIT` clamped to `1..=MAX_LIMIT`. Returns note ids
(unparseable ids skipped).

---

## fn bit

**Identification** — `fn bit(b: bool) -> &'static str`; marker `// md:fn bit`.

**What it does** — `"1"`/`"0"` for the text-typed FTS columns.

**Used by** — `upsert`, `query`.

---

## SearchHandle

**Identification** — `#[derive(Clone)] pub struct SearchHandle`; marker
`// md:SearchHandle`.

**What it does** — The cloneable query view (an `Arc<Mutex<Index>>`).

**Used by** — REST/gRPC search endpoints via the state.

---

## impl SearchHandle

**Identification** — inherent impl; marker `// md:impl SearchHandle`. One
method.

### fn search

**Identification** —
`pub async fn search(&self, query: &SearchQuery) -> anyhow::Result<Vec<Uuid>>`;
marker `// md:impl SearchHandle > fn search`.

**What it does** — Locks the index and runs `query`; ids best-match first.

---

## fn denormalize

**Identification** —
`async fn denormalize(backend: &Arc<dyn StorageBackend>, note: &Note) -> (String, String)`;
marker `// md:fn denormalize`.

**What it does** — Reads the note's tag names (space-joined) and notebook
title through the **plaintext** top of the stack; failures degrade to empty
strings.

**Used by** — `rebuild`, `index_note`.

---

## fn rebuild

**Identification** —
`async fn rebuild(index: &Arc<Mutex<Index>>, backend: &Arc<dyn StorageBackend>)`;
marker `// md:fn rebuild`.

**What it does** — Clears the index, then pages through `list_notes` upserting
every live note with denormalised names. Used at startup, on stream lag, and
when a rename/delete makes denormalised names stale in bulk. Failures are
logged, never fatal (search degrades, the daemon keeps running). The lock is
taken per note, so queries interleave with a rebuild.

**Used by** — `start`'s task, `apply_change`.

---

## fn index_note

**Identification** — marker `// md:fn index_note`.

**What it does** — Indexes one note: tombstoned → remove; live → denormalise
(lock released during the backend reads) and upsert. Errors logged.

**Used by** — `apply_change`, `reindex_id`.

---

## fn reindex_id

**Identification** — marker `// md:fn reindex_id`.

**What it does** — Re-reads a note by id and reindexes it; unreadable
(deleted/absent) → ensure it is removed from the index. Used when its
associations change.

**Used by** — `apply_change` (NoteTagAdd/Remove).

---

## fn start

**Identification** —
`pub async fn start(backend: Arc<dyn StorageBackend>, events: broadcast::Sender<Change>) -> Option<SearchHandle>`;
marker `// md:fn start`.

**What it does** — Builds the index (`None` on failure — search is simply
disabled, the daemon still runs), **subscribes to the change stream before
the initial rebuild** so nothing written during the rebuild is missed (a
duplicate upsert of the same note is harmless), and spawns the maintenance
task: rebuild, then loop on `recv` — `Ok` → `apply_change`; `Lagged(n)` →
warn + rebuild; `Closed` → exit. The rebuild runs inside the task so startup
is not blocked; early queries just see fewer results.

**Used by** — `main.rs`.

---

## fn apply_change

**Identification** — marker `// md:fn apply_change`.

**What it does** — One change → index maintenance: note create/update →
`index_note`; note delete → remove; `NoteTagAdd`/`Remove` → `reindex_id`
(exactly one note's denormalised tags changed); notebook/tag update/delete →
full `rebuild` (a rename alters denormalised names across many notes;
infrequent, so wholesale is fine); everything else ignored.

**Used by** — `start`'s task.

---

## mod tests

**Identification** — `#[cfg(test)] mod tests`; marker `// md:mod tests`. Two
helpers + six tests.

**What it does** — Pins prefix matching, denormalised-name matching,
structured filters, recency listing, the end-to-end rebuild + live-event
path, and removal.

### fn note

**Identification** — helper; marker `// md:mod tests > fn note`.

**What it does** — A fresh note with `updated_at = now`.

### fn idx

**Identification** — helper; marker `// md:mod tests > fn idx`.

**What it does** — An open `Index`.

### fn matches_title_and_body_by_prefix

**Identification** — marker
`// md:mod tests > fn matches_title_and_body_by_prefix`.

**What it does** — `shop`/`milk`/`brea` all match; `zzz` doesn't.

### fn matches_tag_and_notebook_names

**Identification** — marker
`// md:mod tests > fn matches_tag_and_notebook_names`.

**What it does** — Denormalised `urgent work` / `Projects` are searchable.

### fn structured_filters_narrow_results

**Identification** — marker
`// md:mod tests > fn structured_filters_narrow_results`.

**What it does** — `text + is_starred` returns only the starred twin.

### fn empty_query_lists_by_recency_with_filters

**Identification** — marker
`// md:mod tests > fn empty_query_lists_by_recency_with_filters`.

**What it does** — Empty text + `is_todo` filter returns the todo.

### fn indexes_from_rebuild_and_the_event_stream

**Identification** — tokio test; marker
`// md:mod tests > fn indexes_from_rebuild_and_the_event_stream`.

**What it does** — Over `EventBackend<FsBackend>`: a pre-existing note is
found via the startup rebuild; a post-start note via the live stream; a
delete removes it — each with a bounded 50×50 ms poll.

### fn remove_drops_the_note

**Identification** — marker `// md:mod tests > fn remove_drops_the_note`.

**What it does** — `remove` empties the query result.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `SearchQuery`, `SearchHandle`, `start()` — defined here (EXTRACTED)
- `Index`, `rebuild()`, `index_note()`, `apply_change()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/models.rs`, `storage/backend.rs` (EXTRACTED: references×5/×6)
- `keeplin-daemon/src/event_backend.rs` — in the e2e test (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/main.rs` — starts it (INFERRED)
- `keeplin-daemon/src/rest.rs` / `server.rs` — the search endpoints (INFERRED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `struct SearchQuery` | `// md:SearchQuery` |
| 3 | `const MAX_LIMIT` | `// md:MAX_LIMIT` |
| 4 | `fn rfc` | `// md:fn rfc` |
| 5 | `fn fts_match` | `// md:fn fts_match` |
| 6 | `struct Index` | `// md:Index` |
| 7 | `impl Index` (+ `open`, `remove`, `upsert`, `clear`, `query`) | `// md:impl Index` (+ `> fn …`) |
| 8 | `fn bit` | `// md:fn bit` |
| 9 | `struct SearchHandle` | `// md:SearchHandle` |
| 10 | `impl SearchHandle` (+ `search`) | `// md:impl SearchHandle` (+ `> fn search`) |
| 11–15 | `fn denormalize`, `fn rebuild`, `fn index_note`, `fn reindex_id`, `fn start`, `fn apply_change` | `// md:fn <name>` |
| 16 | `mod tests` (+ 2 helpers + 6 tests) | `// md:mod tests` (+ `> fn …`) |
