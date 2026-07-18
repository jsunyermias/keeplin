// md:Overview
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{now, Note, Notebook},
    storage::{EntityVersion, StorageBackend},
};

// md:REVERT_SCAN_LIMIT
const REVERT_SCAN_LIMIT: u32 = 10_000;

// md:fn state_at
pub fn state_at<T>(versions: &[EntityVersion<T>], at: DateTime<Utc>) -> Option<&EntityVersion<T>> {
    versions.iter().find(|v| v.timestamp <= at)
}

// md:fn revert_note
pub async fn revert_note(
    backend: &dyn StorageBackend,
    id: Uuid,
    at: DateTime<Utc>,
) -> Result<Note, StorageError> {
    let versions = backend.note_history(id, REVERT_SCAN_LIMIT).await?;
    let target = state_at(&versions, at).ok_or_else(|| StorageError::NotFound(id.to_string()))?;
    match &target.entity {
        Some(note) => {
            let mut restored = note.clone();
            restored.updated_at = now();
            restored.deleted_at = None;
            backend.update_note(restored).await
        }
        None => {
            if let Err(e) = backend.delete_note(id).await {
                if !matches!(e, StorageError::NotFound(_)) {
                    return Err(e);
                }
            }
            backend.read_note(id).await
        }
    }
}

// md:fn revert_notebook
pub async fn revert_notebook(
    backend: &dyn StorageBackend,
    id: Uuid,
    at: DateTime<Utc>,
) -> Result<Notebook, StorageError> {
    let versions = backend.notebook_history(id, REVERT_SCAN_LIMIT).await?;
    let target = state_at(&versions, at).ok_or_else(|| StorageError::NotFound(id.to_string()))?;
    match &target.entity {
        Some(notebook) => {
            let mut restored = notebook.clone();
            restored.updated_at = now();
            restored.deleted_at = None;
            backend.update_notebook(restored).await
        }
        None => {
            if let Err(e) = backend.delete_notebook(id).await {
                if !matches!(e, StorageError::NotFound(_)) {
                    return Err(e);
                }
            }
            backend.read_notebook(id).await
        }
    }
}

// md:fn revert_notes_to
pub async fn revert_notes_to(
    backend: &dyn StorageBackend,
    ids: &[Uuid],
    at: DateTime<Utc>,
) -> Result<Vec<Note>, StorageError> {
    let mut reverted = Vec::with_capacity(ids.len());
    for &id in ids {
        reverted.push(revert_note(backend, id, at).await?);
    }
    Ok(reverted)
}

// md:fn revert_notebook_notes_to
pub async fn revert_notebook_notes_to(
    backend: &dyn StorageBackend,
    notebook_id: Uuid,
    at: DateTime<Utc>,
) -> Result<Vec<Note>, StorageError> {
    let mut ids = Vec::new();
    let mut token = None;
    loop {
        let (page, next) = backend
            .list_notes_in_notebook(notebook_id, 0, token)
            .await?;
        ids.extend(page.into_iter().map(|n| n.id));
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    revert_notes_to(backend, &ids, at).await
}

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::fs::FsBackend;
    use crate::storage::{HistoryRepository, NoteRepository};
    use chrono::TimeZone;
    use std::time::Duration;
    use tempfile::tempdir;

    // md:mod tests > fn ver
    fn ver(secs: i64, entity: Option<u32>) -> EntityVersion<u32> {
        EntityVersion {
            timestamp: Utc.timestamp_opt(secs, 0).unwrap(),
            device_id: "d".into(),
            entity,
        }
    }

    // md:mod tests > fn state_at_picks_newest_at_or_before
    #[test]
    fn state_at_picks_newest_at_or_before() {
        let versions = vec![ver(30, Some(3)), ver(20, Some(2)), ver(10, Some(1))];
        assert_eq!(
            state_at(&versions, Utc.timestamp_opt(25, 0).unwrap())
                .unwrap()
                .entity,
            Some(2)
        );
        assert_eq!(
            state_at(&versions, Utc.timestamp_opt(30, 0).unwrap())
                .unwrap()
                .entity,
            Some(3)
        );
        assert!(state_at(&versions, Utc.timestamp_opt(5, 0).unwrap()).is_none());
    }

    // md:mod tests > fn fs
    async fn fs() -> FsBackend {
        FsBackend::new(tempdir().unwrap().keep()).await.unwrap()
    }

    // md:mod tests > fn note_history_lists_versions_newest_first
    #[tokio::test]
    async fn note_history_lists_versions_newest_first() {
        let be = fs().await;
        let n = be.create_note(Note::new("t", "v1")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        let mut edited = n.clone();
        edited.body = "v2".into();
        be.update_note(edited).await.unwrap();

        let hist = be.note_history(n.id, 0).await.unwrap();
        assert_eq!(hist.len(), 2, "create + update = two versions");
        assert_eq!(hist[0].entity.as_ref().unwrap().body, "v2", "newest first");
        assert_eq!(hist[1].entity.as_ref().unwrap().body, "v1");
        assert!(hist[0].timestamp >= hist[1].timestamp);
    }

    // md:mod tests > fn revert_restores_an_earlier_version
    #[tokio::test]
    async fn revert_restores_an_earlier_version() {
        let be = fs().await;
        let n = be.create_note(Note::new("t", "v1")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        let mut edited = n.clone();
        edited.body = "v2".into();
        be.update_note(edited).await.unwrap();

        let hist = be.note_history(n.id, 0).await.unwrap();
        let reverted = revert_note(&be, n.id, hist[1].timestamp).await.unwrap();
        assert_eq!(reverted.body, "v1", "revert re-applied the old body");
        assert_eq!(be.read_note(n.id).await.unwrap().body, "v1");
        assert_eq!(be.note_history(n.id, 0).await.unwrap().len(), 3);
    }

    // md:mod tests > fn revert_to_a_deleted_instant_deletes_the_note
    #[tokio::test]
    async fn revert_to_a_deleted_instant_deletes_the_note() {
        let be = fs().await;
        let n = be.create_note(Note::new("t", "v1")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        be.delete_note(n.id).await.unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        let mut revived = n.clone();
        revived.body = "back".into();
        be.update_note(revived).await.unwrap();
        assert!(be.read_note(n.id).await.unwrap().deleted_at.is_none());

        let hist = be.note_history(n.id, 0).await.unwrap();
        let tomb = hist
            .iter()
            .find(|v| v.entity.is_none())
            .expect("a tombstone version exists");
        let reverted = revert_note(&be, n.id, tomb.timestamp).await.unwrap();
        assert!(
            reverted.deleted_at.is_some(),
            "reverting to a deleted instant deletes the note"
        );
    }

    // md:mod tests > fn batch_revert_of_a_notebook_rolls_back_every_note
    #[tokio::test]
    async fn batch_revert_of_a_notebook_rolls_back_every_note() {
        let be = fs().await;
        let nb = Uuid::from_u128(0xB007);
        let mut a = Note::new("a", "a1");
        a.notebook_id = nb;
        let a = be.create_note(a).await.unwrap();
        let mut b = Note::new("b", "b1");
        b.notebook_id = nb;
        let b = be.create_note(b).await.unwrap();
        let cutoff = now();
        tokio::time::sleep(Duration::from_millis(2)).await;

        for (n, body) in [(&a, "a2"), (&b, "b2")] {
            let mut e = n.clone();
            e.body = body.into();
            be.update_note(e).await.unwrap();
        }

        let reverted = revert_notebook_notes_to(&be, nb, cutoff).await.unwrap();
        assert_eq!(reverted.len(), 2);
        assert_eq!(be.read_note(a.id).await.unwrap().body, "a1");
        assert_eq!(be.read_note(b.id).await.unwrap().body, "b1");
    }
}
