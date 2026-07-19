// md:Overview
use std::sync::Arc;

use chrono::{DateTime, Utc};
use keeplin_core::models::{Change, Note};
use keeplin_core::storage::StorageBackend;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

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

// md:MAX_LIMIT
pub const MAX_LIMIT: usize = 500;

// md:fn rfc
fn rfc(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

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

// md:Index
struct Index {
    conn: libsql::Connection,
    _db: libsql::Database,
}

// md:impl Index
impl Index {
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

    // md:impl Index > fn clear
    async fn clear(&self) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM note_fts", ()).await?;
        Ok(())
    }

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
}

// md:fn bit
fn bit(b: bool) -> &'static str {
    if b {
        "1"
    } else {
        "0"
    }
}

// md:SearchHandle
#[derive(Clone)]
pub struct SearchHandle {
    index: Arc<Mutex<Index>>,
}

// md:impl SearchHandle
impl SearchHandle {
    // md:impl SearchHandle > fn search
    pub async fn search(&self, query: &SearchQuery) -> anyhow::Result<Vec<Uuid>> {
        self.index.lock().await.query(query).await
    }
}

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

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;

    // md:mod tests > fn note
    fn note(title: &str, body: &str) -> Note {
        let mut n = Note::new(title, body);
        n.updated_at = Utc::now();
        n
    }

    // md:mod tests > fn idx
    async fn idx() -> Index {
        Index::open().await.unwrap()
    }

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
}
