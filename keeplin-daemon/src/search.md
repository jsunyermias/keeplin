# `search.rs` — daemon full-text search

Self-contained companion for `keeplin-daemon/src/search.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must
be able to understand it without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

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

**Code** — complete and verbatim:

```rust
// md:SearchQuery
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub text: String,
    pub notebook_id: Option<Uuid>,
    pub is_todo: Option<bool>,
    pub todo_open: Option<bool>,
    pub is_starred: Option<bool>,
    pub is_pinned: Option<bool>,
    pub due_after: Option<DateTime<Utc>>,
    pub due_before: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
    pub updated_before: Option<DateTime<Utc>>,
    pub limit: usize,
}
```

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

**Code** — complete and verbatim:

```rust
// md:MAX_LIMIT
pub const MAX_LIMIT: usize = 500;
```

**What it does** — Upper bound on results per query.

**Used by** — `Index::query`'s clamp.

---

## fn rfc

**Identification** — `fn rfc(ts: DateTime<Utc>) -> String`; marker
`// md:fn rfc`.

**Code** — complete and verbatim:

```rust
// md:fn rfc
fn rfc(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}
```

**What it does** — RFC3339 UTC (micros, `Z`): lexicographic order equals
chronological, so range filters work as plain string comparisons.

**Used by** — `Index::upsert`/`query`.

---

## fn fts_match

**Identification** — `fn fts_match(text: &str) -> Option<String>`; marker
`// md:fn fts_match`.

**Code** — complete and verbatim:

```rust
// md:fn fts_match
fn fts_match(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split_whitespace()
        .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}
```

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

**Code** — complete and verbatim:

```rust
// md:Index
struct Index {
    conn: libsql::Connection,
    _db: libsql::Database,
}
```

**What it does** — The in-memory FTS5 index: a `libsql::Connection` plus the
`Database` handle kept alive alongside it (each `:memory:` connection is its
own database). Shared behind `Arc<Mutex<…>>` by the query handle and the
maintenance task.

**Used by** — everything below.

---

## impl Index

**Identification** — inherent impl; marker `// md:impl Index`. Five methods.

**Code** — container: members documented as sub-blocks below: fn open, fn remove, fn upsert, fn clear, fn query.

### fn open

**Identification** — `async fn open() -> anyhow::Result<Self>`; marker
`// md:impl Index > fn open`.

**Code** — complete and verbatim:

```rust
    // md:impl Index > fn open
    async fn open() -> anyhow::Result<Self> {
        let db = libsql::Builder::new_local(":memory:").build().await?;
        let conn = db.connect()?;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE note_fts USING fts5(
                 note_id UNINDEXED,
                 title, body, tags, notebook,
                 notebook_id UNINDEXED,
                 is_todo UNINDEXED,
                 is_completed UNINDEXED,
                 todo_due UNINDEXED,
                 is_starred UNINDEXED,
                 is_pinned UNINDEXED,
                 updated_at UNINDEXED
             );",
        )
        .await?;
        Ok(Self { conn, _db: db })
    }
```

**What it does** — Creates the `:memory:` database and the `note_fts` FTS5
virtual table: indexed columns `title, body, tags, notebook`; `UNINDEXED`
structured columns `note_id, notebook_id, is_todo, is_completed, todo_due,
is_starred, is_pinned, updated_at`.

### fn remove

**Identification** — marker `// md:impl Index > fn remove`.

**Code** — complete and verbatim:

```rust
    // md:impl Index > fn remove
    async fn remove(&self, note_id: Uuid) -> anyhow::Result<()> {
        self.conn
            .execute(
                "DELETE FROM note_fts WHERE note_id = ?1",
                libsql::params![note_id.to_string()],
            )
            .await?;
        Ok(())
    }
```

**What it does** — Deletes a note's row by id.

### fn upsert

**Identification** —
`async fn upsert(&self, note: &Note, tags: &str, notebook: &str)`; marker
`// md:impl Index > fn upsert`.

**Code** — complete and verbatim:

```rust
    // md:impl Index > fn upsert
    async fn upsert(&self, note: &Note, tags: &str, notebook: &str) -> anyhow::Result<()> {
        self.remove(note.id).await?;
        self.conn
            .execute(
                "INSERT INTO note_fts
                     (note_id, title, body, tags, notebook, notebook_id,
                      is_todo, is_completed, todo_due, is_starred, is_pinned, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                libsql::params![
                    note.id.to_string(),
                    note.title.clone(),
                    note.body.clone(),
                    tags.to_string(),
                    notebook.to_string(),
                    note.notebook_id.to_string(),
                    bit(note.is_todo),
                    bit(note.todo_completed.is_some()),
                    note.todo_due.map(rfc).unwrap_or_default(),
                    bit(note.is_starred),
                    bit(note.is_pinned),
                    rfc(note.updated_at),
                ],
            )
            .await?;
        Ok(())
    }
```

**What it does** — FTS5 has no UPDATE: delete then insert the full row
(booleans as `"0"`/`"1"` via `bit`, timestamps via `rfc`, missing due date as
`""`).

### fn clear

**Identification** — marker `// md:impl Index > fn clear`.

**Code** — complete and verbatim:

```rust
    // md:impl Index > fn clear
    async fn clear(&self) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM note_fts", ()).await?;
        Ok(())
    }
```

**What it does** — Empties the table (before a rebuild).

### fn query

**Identification** —
`async fn query(&self, q: &SearchQuery) -> anyhow::Result<Vec<Uuid>>`; marker
`// md:impl Index > fn query`.

**Code** — complete and verbatim:

```rust
    // md:impl Index > fn query
    async fn query(&self, q: &SearchQuery) -> anyhow::Result<Vec<Uuid>> {
        let limit = q.limit.clamp(1, MAX_LIMIT);
        let match_expr = fts_match(&q.text);

        let mut conds: Vec<String> = Vec::new();
        if match_expr.is_some() {
            conds.push("note_fts MATCH ?1".into());
        }
        if let Some(id) = q.notebook_id {
            conds.push(format!("notebook_id = '{id}'"));
        }
        if let Some(v) = q.is_todo {
            conds.push(format!("is_todo = '{}'", bit(v)));
        }
        if let Some(open) = q.todo_open {
            conds.push(format!("is_completed = '{}'", bit(!open)));
        }
        if let Some(v) = q.is_starred {
            conds.push(format!("is_starred = '{}'", bit(v)));
        }
        if let Some(v) = q.is_pinned {
            conds.push(format!("is_pinned = '{}'", bit(v)));
        }
        if let Some(t) = q.due_after {
            conds.push(format!("todo_due >= '{}'", rfc(t)));
        }
        if let Some(t) = q.due_before {
            conds.push(format!("todo_due <= '{}' AND todo_due <> ''", rfc(t)));
        }
        if let Some(t) = q.updated_after {
            conds.push(format!("updated_at >= '{}'", rfc(t)));
        }
        if let Some(t) = q.updated_before {
            conds.push(format!("updated_at <= '{}'", rfc(t)));
        }

        let where_clause = if conds.is_empty() {
            "1=1".to_string()
        } else {
            conds.join(" AND ")
        };
        let order = if match_expr.is_some() {
            "rank"
        } else {
            "updated_at DESC"
        };
        let sql = format!(
            "SELECT note_id FROM note_fts WHERE {where_clause} ORDER BY {order} LIMIT {limit}"
        );

        let mut rows = match &match_expr {
            Some(m) => self.conn.query(&sql, libsql::params![m.clone()]).await?,
            None => self.conn.query(&sql, ()).await?,
        };
        let mut ids = Vec::new();
        while let Some(row) = rows.next().await? {
            if let Ok(id) = Uuid::parse_str(&row.get::<String>(0)?) {
                ids.push(id);
            }
        }
        Ok(ids)
    }
```

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

**Code** — complete and verbatim:

```rust
// md:fn bit
fn bit(b: bool) -> &'static str {
    if b {
        "1"
    } else {
        "0"
    }
}
```

**What it does** — `"1"`/`"0"` for the text-typed FTS columns.

**Used by** — `upsert`, `query`.

---

## SearchHandle

**Identification** — `#[derive(Clone)] pub struct SearchHandle`; marker
`// md:SearchHandle`.

**Code** — complete and verbatim:

```rust
// md:SearchHandle
#[derive(Clone)]
pub struct SearchHandle {
    index: Arc<Mutex<Index>>,
}
```

**What it does** — The cloneable query view (an `Arc<Mutex<Index>>`).

**Used by** — REST/gRPC search endpoints via the state.

---

## impl SearchHandle

**Identification** — inherent impl; marker `// md:impl SearchHandle`. One
method.

**Code** — container: members documented as sub-blocks below: fn search.

### fn search

**Identification** —
`pub async fn search(&self, query: &SearchQuery) -> anyhow::Result<Vec<Uuid>>`;
marker `// md:impl SearchHandle > fn search`.

**Code** — complete and verbatim:

```rust
    // md:impl SearchHandle > fn search
    pub async fn search(&self, query: &SearchQuery) -> anyhow::Result<Vec<Uuid>> {
        self.index.lock().await.query(query).await
    }
```

**What it does** — Locks the index and runs `query`; ids best-match first.

---

## fn denormalize

**Identification** —
`async fn denormalize(backend: &Arc<dyn StorageBackend>, note: &Note) -> (String, String)`;
marker `// md:fn denormalize`.

**Code** — complete and verbatim:

```rust
// md:fn denormalize
async fn denormalize(backend: &Arc<dyn StorageBackend>, note: &Note) -> (String, String) {
    let tags = backend
        .list_note_tags(note.id, 0, None)
        .await
        .map(|(tags, _)| {
            tags.iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let notebook = backend
        .read_notebook(note.notebook_id)
        .await
        .map(|n| n.title)
        .unwrap_or_default();
    (tags, notebook)
}
```

**What it does** — Reads the note's tag names (space-joined) and notebook
title through the **plaintext** top of the stack; failures degrade to empty
strings.

**Used by** — `rebuild`, `index_note`.

---

## fn rebuild

**Identification** —
`async fn rebuild(index: &Arc<Mutex<Index>>, backend: &Arc<dyn StorageBackend>)`;
marker `// md:fn rebuild`.

**Code** — complete and verbatim:

```rust
// md:fn rebuild
async fn rebuild(index: &Arc<Mutex<Index>>, backend: &Arc<dyn StorageBackend>) {
    let guard = index.lock().await;
    if let Err(e) = guard.clear().await {
        tracing::warn!(error = %e, "search: clear before rebuild failed");
        return;
    }
    drop(guard);

    let mut token = None;
    loop {
        let (page, next) = match backend.list_notes(0, token).await {
            Ok(page) => page,
            Err(e) => {
                tracing::warn!(error = %e, "search: rebuild scan failed");
                return;
            }
        };
        for note in page {
            let (tags, notebook) = denormalize(backend, &note).await;
            let guard = index.lock().await;
            if let Err(e) = guard.upsert(&note, &tags, &notebook).await {
                tracing::warn!(error = %e, note = %note.id, "search: index note failed");
            }
        }
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    tracing::info!("search: index rebuilt");
}
```

**What it does** — Clears the index, then pages through `list_notes` upserting
every live note with denormalised names. Used at startup, on stream lag, and
when a rename/delete makes denormalised names stale in bulk. Failures are
logged, never fatal (search degrades, the daemon keeps running). The lock is
taken per note, so queries interleave with a rebuild.

**Used by** — `start`'s task, `apply_change`.

---

## fn index_note

**Identification** — marker `// md:fn index_note`.

**Code** — complete and verbatim:

```rust
// md:fn index_note
async fn index_note(index: &Arc<Mutex<Index>>, backend: &Arc<dyn StorageBackend>, note: &Note) {
    let guard = index.lock().await;
    let result = if note.deleted_at.is_some() {
        guard.remove(note.id).await
    } else {
        drop(guard);
        let (tags, notebook) = denormalize(backend, note).await;
        let guard = index.lock().await;
        guard.upsert(note, &tags, &notebook).await
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, note = %note.id, "search: index update failed");
    }
}
```

**What it does** — Indexes one note: tombstoned → remove; live → denormalise
(lock released during the backend reads) and upsert. Errors logged.

**Used by** — `apply_change`, `reindex_id`.

---

## fn reindex_id

**Identification** — marker `// md:fn reindex_id`.

**Code** — complete and verbatim:

```rust
// md:fn reindex_id
async fn reindex_id(index: &Arc<Mutex<Index>>, backend: &Arc<dyn StorageBackend>, id: Uuid) {
    match backend.read_note(id).await {
        Ok(note) => index_note(index, backend, &note).await,
        Err(_) => {
            let guard = index.lock().await;
            let _ = guard.remove(id).await;
        }
    }
}
```

**What it does** — Re-reads a note by id and reindexes it; unreadable
(deleted/absent) → ensure it is removed from the index. Used when its
associations change.

**Used by** — `apply_change` (NoteTagAdd/Remove).

---

## fn start

**Identification** —
`pub async fn start(backend: Arc<dyn StorageBackend>, events: broadcast::Sender<Change>) -> Option<SearchHandle>`;
marker `// md:fn start`.

**Code** — complete and verbatim:

```rust
// md:fn start
pub async fn start(
    backend: Arc<dyn StorageBackend>,
    events: broadcast::Sender<Change>,
) -> Option<SearchHandle> {
    let index = match Index::open().await {
        Ok(index) => Arc::new(Mutex::new(index)),
        Err(e) => {
            tracing::warn!(error = %e, "search: could not open index; search disabled");
            return None;
        }
    };
    let handle = SearchHandle {
        index: index.clone(),
    };

    let mut rx = events.subscribe();

    tokio::spawn(async move {
        rebuild(&index, &backend).await;
        loop {
            match rx.recv().await {
                Ok(change) => apply_change(&index, &backend, change).await,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(dropped = n, "search: change stream lagged; rebuilding");
                    rebuild(&index, &backend).await;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Some(handle)
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn apply_change
async fn apply_change(
    index: &Arc<Mutex<Index>>,
    backend: &Arc<dyn StorageBackend>,
    change: Change,
) {
    match change {
        Change::NoteCreate { note } | Change::NoteUpdate { note } => {
            index_note(index, backend, &note).await
        }
        Change::NoteDelete { id, .. } => {
            let guard = index.lock().await;
            let _ = guard.remove(id).await;
        }
        Change::NoteTagAdd { note_id, .. } | Change::NoteTagRemove { note_id, .. } => {
            reindex_id(index, backend, note_id).await
        }
        Change::NotebookUpdate { .. }
        | Change::NotebookDelete { .. }
        | Change::TagUpdate { .. }
        | Change::TagDelete { .. } => rebuild(index, backend).await,
        _ => {}
    }
}
```

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

**Code** — container: members documented as sub-blocks below: fn note, fn idx, fn matches_title_and_body_by_prefix, fn matches_tag_and_notebook_names, fn structured_filters_narrow_results, fn empty_query_lists_by_recency_with_filters, fn indexes_from_rebuild_and_the_event_stream, fn remove_drops_the_note.

**What it does** — Pins prefix matching, denormalised-name matching,
structured filters, recency listing, the end-to-end rebuild + live-event
path, and removal.

### fn note

**Identification** — helper; marker `// md:mod tests > fn note`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn note
    fn note(title: &str, body: &str) -> Note {
        let mut n = Note::new(title, body);
        n.updated_at = Utc::now();
        n
    }
```

**What it does** — A fresh note with `updated_at = now`.

### fn idx

**Identification** — helper; marker `// md:mod tests > fn idx`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn idx
    async fn idx() -> Index {
        Index::open().await.unwrap()
    }
```

**What it does** — An open `Index`.

### fn matches_title_and_body_by_prefix

**Identification** — marker
`// md:mod tests > fn matches_title_and_body_by_prefix`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn matches_title_and_body_by_prefix
    #[tokio::test]
    async fn matches_title_and_body_by_prefix() {
        let index = idx().await;
        let n = note("Shopping list", "milk and bread");
        index.upsert(&n, "", "").await.unwrap();

        for q in ["shop", "milk", "brea"] {
            let hits = index
                .query(&SearchQuery {
                    text: q.into(),
                    limit: 10,
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(hits, vec![n.id], "query {q:?} should match");
        }
        let none = index
            .query(&SearchQuery {
                text: "zzz".into(),
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(none.is_empty());
    }
```

**What it does** — `shop`/`milk`/`brea` all match; `zzz` doesn't.

### fn matches_tag_and_notebook_names

**Identification** — marker
`// md:mod tests > fn matches_tag_and_notebook_names`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn matches_tag_and_notebook_names
    #[tokio::test]
    async fn matches_tag_and_notebook_names() {
        let index = idx().await;
        let n = note("untitled", "no keywords here");
        index.upsert(&n, "urgent work", "Projects").await.unwrap();

        for q in ["urgent", "projects"] {
            let hits = index
                .query(&SearchQuery {
                    text: q.into(),
                    limit: 10,
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(hits, vec![n.id], "query {q:?} should match tag/notebook");
        }
    }
```

**What it does** — Denormalised `urgent work` / `Projects` are searchable.

### fn structured_filters_narrow_results

**Identification** — marker
`// md:mod tests > fn structured_filters_narrow_results`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn structured_filters_narrow_results
    #[tokio::test]
    async fn structured_filters_narrow_results() {
        let index = idx().await;
        let mut starred = note("alpha", "body");
        starred.is_starred = true;
        let plain = note("alpha", "body");
        index.upsert(&starred, "", "").await.unwrap();
        index.upsert(&plain, "", "").await.unwrap();

        let hits = index
            .query(&SearchQuery {
                text: "alpha".into(),
                is_starred: Some(true),
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(hits, vec![starred.id], "only the starred note matches");
    }
```

**What it does** — `text + is_starred` returns only the starred twin.

### fn empty_query_lists_by_recency_with_filters

**Identification** — marker
`// md:mod tests > fn empty_query_lists_by_recency_with_filters`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn empty_query_lists_by_recency_with_filters
    #[tokio::test]
    async fn empty_query_lists_by_recency_with_filters() {
        let index = idx().await;
        let mut todo = note("a", "b");
        todo.is_todo = true;
        let other = note("c", "d");
        index.upsert(&todo, "", "").await.unwrap();
        index.upsert(&other, "", "").await.unwrap();

        let hits = index
            .query(&SearchQuery {
                text: String::new(),
                is_todo: Some(true),
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(hits, vec![todo.id], "empty text + filter returns the todo");
    }
```

**What it does** — Empty text + `is_todo` filter returns the todo.

### fn indexes_from_rebuild_and_the_event_stream

**Identification** — tokio test; marker
`// md:mod tests > fn indexes_from_rebuild_and_the_event_stream`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn indexes_from_rebuild_and_the_event_stream
    #[tokio::test]
    async fn indexes_from_rebuild_and_the_event_stream() {
        use keeplin_core::storage::fs::FsBackend;

        let dir = tempfile::tempdir().unwrap();
        let fs = FsBackend::new(dir.path()).await.unwrap();
        let (events, _keep) = broadcast::channel(64);
        let backend: Arc<dyn StorageBackend> =
            Arc::new(crate::event_backend::EventBackend::new(fs, events.clone()));

        let pre = backend
            .create_note(Note::new("preexisting alpha", "b"))
            .await
            .unwrap();
        let handle = start(backend.clone(), events.clone()).await.unwrap();
        let live = backend
            .create_note(Note::new("live beta", "b"))
            .await
            .unwrap();

        let found = |term: &'static str, id: Uuid| {
            let handle = handle.clone();
            async move {
                for _ in 0..50 {
                    let hits = handle
                        .search(&SearchQuery {
                            text: term.into(),
                            limit: 10,
                            ..Default::default()
                        })
                        .await
                        .unwrap();
                    if hits.contains(&id) {
                        return true;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                false
            }
        };
        assert!(found("alpha", pre.id).await, "rebuild indexed the old note");
        assert!(
            found("beta", live.id).await,
            "event stream indexed the new note"
        );

        backend.delete_note(live.id).await.unwrap();
        for _ in 0..50 {
            let hits = handle
                .search(&SearchQuery {
                    text: "beta".into(),
                    limit: 10,
                    ..Default::default()
                })
                .await
                .unwrap();
            if !hits.contains(&live.id) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("deleted note stayed in the index");
    }
```

**What it does** — Over `EventBackend<FsBackend>`: a pre-existing note is
found via the startup rebuild; a post-start note via the live stream; a
delete removes it — each with a bounded 50×50 ms poll.

### fn remove_drops_the_note

**Identification** — marker `// md:mod tests > fn remove_drops_the_note`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn remove_drops_the_note
    #[tokio::test]
    async fn remove_drops_the_note() {
        let index = idx().await;
        let n = note("gone", "soon");
        index.upsert(&n, "", "").await.unwrap();
        index.remove(n.id).await.unwrap();
        let hits = index
            .query(&SearchQuery {
                text: "gone".into(),
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(hits.is_empty());
    }
```

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
| 1 | `Overview` | `// md:Overview` |
| 2 | `SearchQuery` | `// md:SearchQuery` |
| 3 | `MAX_LIMIT` | `// md:MAX_LIMIT` |
| 4 | `fn rfc` | `// md:fn rfc` |
| 5 | `fn fts_match` | `// md:fn fts_match` |
| 6 | `Index` | `// md:Index` |
| 7 | `impl Index` (container) | `// md:impl Index` |
| 8 | `fn open` | `// md:impl Index > fn open` |
| 9 | `fn remove` | `// md:impl Index > fn remove` |
| 10 | `fn upsert` | `// md:impl Index > fn upsert` |
| 11 | `fn clear` | `// md:impl Index > fn clear` |
| 12 | `fn query` | `// md:impl Index > fn query` |
| 13 | `fn bit` | `// md:fn bit` |
| 14 | `SearchHandle` | `// md:SearchHandle` |
| 15 | `impl SearchHandle` (container) | `// md:impl SearchHandle` |
| 16 | `fn search` | `// md:impl SearchHandle > fn search` |
| 17 | `fn denormalize` | `// md:fn denormalize` |
| 18 | `fn rebuild` | `// md:fn rebuild` |
| 19 | `fn index_note` | `// md:fn index_note` |
| 20 | `fn reindex_id` | `// md:fn reindex_id` |
| 21 | `fn start` | `// md:fn start` |
| 22 | `fn apply_change` | `// md:fn apply_change` |
| 23 | `mod tests` (container) | `// md:mod tests` |
| 24 | `fn note` | `// md:mod tests > fn note` |
| 25 | `fn idx` | `// md:mod tests > fn idx` |
| 26 | `fn matches_title_and_body_by_prefix` | `// md:mod tests > fn matches_title_and_body_by_prefix` |
| 27 | `fn matches_tag_and_notebook_names` | `// md:mod tests > fn matches_tag_and_notebook_names` |
| 28 | `fn structured_filters_narrow_results` | `// md:mod tests > fn structured_filters_narrow_results` |
| 29 | `fn empty_query_lists_by_recency_with_filters` | `// md:mod tests > fn empty_query_lists_by_recency_with_filters` |
| 30 | `fn indexes_from_rebuild_and_the_event_stream` | `// md:mod tests > fn indexes_from_rebuild_and_the_event_stream` |
| 31 | `fn remove_drops_the_note` | `// md:mod tests > fn remove_drops_the_note` |