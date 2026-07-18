// md:Overview
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Note, Notebook},
    storage::{NotebookSortProfile, StorageBackend},
};

// md:INBOX_ID
pub const INBOX_ID: Uuid = Uuid::nil();

// md:INBOX_TITLE
pub const INBOX_TITLE: &str = "Inbox";

// md:PIN_MAX
pub const PIN_MAX: u32 = 999;

// md:MAX_PINNED
pub const MAX_PINNED: usize = PIN_MAX as usize;

// md:NORMAL_START
pub const NORMAL_START: u32 = Note::DEFAULT_SORT_KEY;

// md:RESEQUENCE_STEP
const RESEQUENCE_STEP: u32 = 1000;

// md:fn ensure_inbox
pub async fn ensure_inbox(backend: &dyn StorageBackend) -> Result<(), StorageError> {
    match backend.read_notebook(INBOX_ID).await {
        Ok(_) => Ok(()),
        Err(StorageError::NotFound(_)) => {
            let mut inbox = Notebook::new(INBOX_TITLE);
            inbox.id = INBOX_ID;
            backend.create_notebook(inbox).await?;
            tracing::info!("Created the Inbox system notebook (\"{INBOX_TITLE}\")");
            Ok(())
        }
        Err(e) => Err(e),
    }
}

// md:fn is_inbox
pub fn is_inbox(id: Uuid) -> bool {
    id == INBOX_ID
}

// md:fn place_new_note
pub async fn place_new_note(
    backend: &dyn StorageBackend,
    note: &mut Note,
) -> Result<(), StorageError> {
    if note.sort_key != 0 {
        return Ok(());
    }
    let profile = backend.notebook_sort_profile(note.notebook_id).await?;
    note.sort_key = if is_inbox(note.notebook_id) {
        match profile.min_key {
            Some(min) if min <= 1 => resequence_inbox(backend).await? - 1,
            Some(min) => min - 1,
            None => NORMAL_START,
        }
    } else {
        next_normal_key(&profile)
    };
    Ok(())
}

// md:fn pin_note
pub async fn pin_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    let mut note = read_live_note(backend, id).await?;
    if is_inbox(note.notebook_id) {
        return Err(StorageError::InvalidInput(
            "Inbox notes cannot be pinned (the Inbox is a single manually ordered list)"
                .to_string(),
        ));
    }
    if note.is_pinned {
        return Ok(note);
    }
    let profile = backend.notebook_sort_profile(note.notebook_id).await?;
    let Some(key) = lowest_free_pinned_key(&profile.pinned_keys) else {
        return Err(StorageError::Conflict(format!(
            "cannot pin: the notebook already has {MAX_PINNED} pinned notes"
        )));
    };
    note.is_pinned = true;
    note.sort_key = key;
    backend.update_note(note).await
}

// md:fn unpin_note
pub async fn unpin_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    let mut note = read_live_note(backend, id).await?;
    if !note.is_pinned {
        return Ok(note);
    }
    let profile = backend.notebook_sort_profile(note.notebook_id).await?;
    note.is_pinned = false;
    note.sort_key = next_normal_key(&profile);
    backend.update_note(note).await
}

// md:fn reconcile_notebook_move
pub async fn reconcile_notebook_move(
    backend: &dyn StorageBackend,
    current_notebook_id: Uuid,
    note: &mut Note,
) -> Result<(), StorageError> {
    if note.notebook_id == current_notebook_id {
        return Ok(());
    }
    note.is_pinned = false;
    note.sort_key = 0;
    place_new_note(backend, note).await
}

// md:fn star_note
pub async fn star_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    set_starred(backend, id, true).await
}

// md:fn unstar_note
pub async fn unstar_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    set_starred(backend, id, false).await
}

// md:fn set_starred
async fn set_starred(
    backend: &dyn StorageBackend,
    id: Uuid,
    starred: bool,
) -> Result<Note, StorageError> {
    let mut note = read_live_note(backend, id).await?;
    if note.is_starred == starred {
        return Ok(note);
    }
    note.is_starred = starred;
    backend.update_note(note).await
}

// md:fn reorder_note
pub async fn reorder_note(
    backend: &dyn StorageBackend,
    id: Uuid,
    new_sort_key: u32,
) -> Result<Note, StorageError> {
    let mut note = read_live_note(backend, id).await?;
    let valid = if is_inbox(note.notebook_id) {
        new_sort_key >= 1
    } else if note.is_pinned {
        (1..=PIN_MAX).contains(&new_sort_key)
    } else {
        new_sort_key >= NORMAL_START
    };
    if !valid {
        let band = if is_inbox(note.notebook_id) {
            ">= 1 (Inbox)".to_string()
        } else if note.is_pinned {
            format!("1..={PIN_MAX} (pinned)")
        } else {
            format!(">= {NORMAL_START} (normal)")
        };
        return Err(StorageError::InvalidInput(format!(
            "sort_key {new_sort_key} is outside the note's band {band}"
        )));
    }
    if note.sort_key == new_sort_key {
        return Ok(note);
    }
    note.sort_key = new_sort_key;
    backend.update_note(note).await
}

// md:fn resequence_inbox
pub async fn resequence_inbox(backend: &dyn StorageBackend) -> Result<u32, StorageError> {
    let mut token = None;
    let mut next = RESEQUENCE_STEP;
    loop {
        let (page, more) = backend.list_notes_in_notebook(INBOX_ID, 0, token).await?;
        for mut note in page {
            if note.effective_sort_key() != next {
                note.sort_key = next;
                backend.update_note(note).await?;
            }
            next = next.saturating_add(RESEQUENCE_STEP);
        }
        match more {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    Ok(RESEQUENCE_STEP)
}

// md:fn lowest_free_pinned_key
fn lowest_free_pinned_key(used: &[u32]) -> Option<u32> {
    let mut candidate = 1u32;
    for &key in used {
        if key > candidate {
            break;
        }
        if key == candidate {
            candidate += 1;
        }
    }
    (candidate <= PIN_MAX).then_some(candidate)
}

// md:fn next_normal_key
fn next_normal_key(profile: &NotebookSortProfile) -> u32 {
    profile
        .max_normal_key
        .map(|max| max.saturating_add(1))
        .unwrap_or(NORMAL_START)
}

// md:fn read_live_note
async fn read_live_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    let note = backend.read_note(id).await?;
    if note.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()));
    }
    Ok(note)
}

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::fs::FsBackend;
    use crate::storage::{NoteRepository, NotebookRepository};

    // md:mod tests > fn backend
    async fn backend() -> FsBackend {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        FsBackend::new(&path).await.unwrap()
    }

    // md:mod tests > fn create_placed
    async fn create_placed(be: &FsBackend, title: &str, notebook: Uuid) -> Note {
        let mut note = Note::new(title, "");
        note.notebook_id = notebook;
        place_new_note(be, &mut note).await.unwrap();
        be.create_note(note).await.unwrap()
    }

    // md:mod tests > fn move_note
    async fn move_note(be: &FsBackend, id: Uuid, dest: Uuid) -> Note {
        let mut note = be.read_note(id).await.unwrap();
        let current = note.notebook_id;
        note.notebook_id = dest;
        reconcile_notebook_move(be, current, &mut note)
            .await
            .unwrap();
        be.update_note(note).await.unwrap()
    }

    // md:mod tests > fn titles
    fn titles(page: &[Note]) -> Vec<&str> {
        page.iter().map(|n| n.title.as_str()).collect()
    }

    // md:mod tests > fn ensure_inbox_is_idempotent_and_fixed
    #[tokio::test]
    async fn ensure_inbox_is_idempotent_and_fixed() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        ensure_inbox(&be).await.unwrap();
        let inbox = be.read_notebook(INBOX_ID).await.unwrap();
        assert_eq!(inbox.title, INBOX_TITLE);
        assert_eq!(inbox.id, INBOX_ID);
    }

    // md:mod tests > fn placement_inbox_top_notebook_bottom
    #[tokio::test]
    async fn placement_inbox_top_notebook_bottom() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();

        create_placed(&be, "first", INBOX_ID).await;
        create_placed(&be, "second", INBOX_ID).await;
        create_placed(&be, "third", INBOX_ID).await;
        let (page, _) = be.list_notes_in_notebook(INBOX_ID, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["third", "second", "first"]);

        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let a = create_placed(&be, "a", nb.id).await;
        let b = create_placed(&be, "b", nb.id).await;
        assert_eq!(a.sort_key, NORMAL_START);
        assert_eq!(b.sort_key, NORMAL_START + 1);
        let (page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["a", "b"]);
    }

    // md:mod tests > fn pin_unpin_round_trip_and_inbox_rejection
    #[tokio::test]
    async fn pin_unpin_round_trip_and_inbox_rejection() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let a = create_placed(&be, "a", nb.id).await;
        let b = create_placed(&be, "b", nb.id).await;

        let pinned = pin_note(&be, b.id).await.unwrap();
        assert!(pinned.is_pinned);
        assert_eq!(pinned.sort_key, 1);
        let (page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["b", "a"], "pinned band lists first");

        let unpinned = unpin_note(&be, b.id).await.unwrap();
        assert!(!unpinned.is_pinned);
        assert!(unpinned.sort_key > a.effective_sort_key());
        let (page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(
            titles(&page),
            ["a", "b"],
            "unpin appends to the normal band"
        );

        let inbox_note = create_placed(&be, "inbox", INBOX_ID).await;
        let err = pin_note(&be, inbox_note.id).await.unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)), "got: {err:?}");
    }

    // md:mod tests > fn reorder_respects_bands
    #[tokio::test]
    async fn reorder_respects_bands() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let a = create_placed(&be, "a", nb.id).await;
        let b = create_placed(&be, "b", nb.id).await;

        reorder_note(&be, b.id, NORMAL_START).await.unwrap();
        reorder_note(&be, a.id, NORMAL_START + 5).await.unwrap();
        let (page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["b", "a"]);

        let err = reorder_note(&be, a.id, 5).await.unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)));
        let pinned = pin_note(&be, a.id).await.unwrap();
        let err = reorder_note(&be, pinned.id, NORMAL_START)
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)));
        reorder_note(&be, pinned.id, 42).await.unwrap();
        assert_eq!(be.read_note(a.id).await.unwrap().sort_key, 42);
    }

    // md:mod tests > fn starring_is_global_and_never_moves_the_note
    #[tokio::test]
    async fn starring_is_global_and_never_moves_the_note() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let in_inbox = create_placed(&be, "inbox note", INBOX_ID).await;
        let in_nb = create_placed(&be, "nb note", nb.id).await;
        create_placed(&be, "unstarred", nb.id).await;

        let starred = star_note(&be, in_inbox.id).await.unwrap();
        assert_eq!(starred.sort_key, in_inbox.sort_key, "star never moves");
        star_note(&be, in_nb.id).await.unwrap();

        let (page, _) = be.list_starred_notes(0, None).await.unwrap();
        let mut got: Vec<&str> = titles(&page);
        got.sort_unstable();
        assert_eq!(got, ["inbox note", "nb note"]);

        unstar_note(&be, in_inbox.id).await.unwrap();
        let (page, _) = be.list_starred_notes(0, None).await.unwrap();
        assert_eq!(titles(&page), ["nb note"]);
    }

    // md:mod tests > fn inbox_top_insert_survives_underflow_by_resequencing
    #[tokio::test]
    async fn inbox_top_insert_survives_underflow_by_resequencing() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let first = create_placed(&be, "old-top", INBOX_ID).await;
        reorder_note(&be, first.id, 1).await.unwrap();

        let newcomer = create_placed(&be, "new-top", INBOX_ID).await;
        assert!(newcomer.sort_key >= 1, "never the 0 sentinel");
        let (page, _) = be.list_notes_in_notebook(INBOX_ID, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["new-top", "old-top"]);
    }

    // md:mod tests > fn moving_a_note_replaces_it_in_the_destination_band
    #[tokio::test]
    async fn moving_a_note_replaces_it_in_the_destination_band() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();

        let first = create_placed(&be, "first", INBOX_ID).await;
        let second = create_placed(&be, "second", INBOX_ID).await;
        assert_eq!(first.sort_key, NORMAL_START);
        assert_eq!(second.sort_key, NORMAL_START - 1, "top-insert lands at 999");

        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        create_placed(&be, "existing", nb.id).await;

        let moved = move_note(&be, second.id, nb.id).await;
        assert_eq!(moved.notebook_id, nb.id);
        assert!(!moved.is_pinned, "a moved note is never auto-pinned");
        assert!(
            moved.sort_key >= NORMAL_START,
            "re-placed into the normal band, not the pinned range (got {})",
            moved.sort_key
        );

        let (nb_page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(titles(&nb_page), ["existing", "second"]);
        let (inbox_page, _) = be.list_notes_in_notebook(INBOX_ID, 0, None).await.unwrap();
        assert_eq!(titles(&inbox_page), ["first"], "gone from the Inbox");
    }

    // md:mod tests > fn moving_a_pinned_note_into_the_inbox_unpins_it
    #[tokio::test]
    async fn moving_a_pinned_note_into_the_inbox_unpins_it() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let n = create_placed(&be, "n", nb.id).await;
        assert!(pin_note(&be, n.id).await.unwrap().is_pinned);

        let moved = move_note(&be, n.id, INBOX_ID).await;
        assert!(!moved.is_pinned);
        assert_eq!(moved.notebook_id, INBOX_ID);
        let (inbox_page, _) = be.list_notes_in_notebook(INBOX_ID, 0, None).await.unwrap();
        assert_eq!(titles(&inbox_page), ["n"]);
    }

    // md:mod tests > fn a_same_notebook_edit_keeps_the_position
    #[tokio::test]
    async fn a_same_notebook_edit_keeps_the_position() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let a = create_placed(&be, "a", nb.id).await;
        create_placed(&be, "b", nb.id).await;

        let mut edit = be.read_note(a.id).await.unwrap();
        let key_before = edit.sort_key;
        edit.title = "a-edited".into();
        reconcile_notebook_move(&be, a.notebook_id, &mut edit)
            .await
            .unwrap();
        let saved = be.update_note(edit).await.unwrap();
        assert_eq!(
            saved.sort_key, key_before,
            "no re-placement on a plain edit"
        );

        let (page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["a-edited", "b"]);
    }

    // md:mod tests > fn lowest_free_pinned_key_fills_gaps_and_detects_full
    #[test]
    fn lowest_free_pinned_key_fills_gaps_and_detects_full() {
        assert_eq!(lowest_free_pinned_key(&[]), Some(1));
        assert_eq!(lowest_free_pinned_key(&[1, 2, 3]), Some(4));
        assert_eq!(lowest_free_pinned_key(&[1, 3]), Some(2));
        assert_eq!(lowest_free_pinned_key(&[2, 3]), Some(1));
        let full: Vec<u32> = (1..=PIN_MAX).collect();
        assert_eq!(lowest_free_pinned_key(&full), None);
        let almost: Vec<u32> = (1..PIN_MAX).collect();
        assert_eq!(lowest_free_pinned_key(&almost), Some(PIN_MAX));
    }
}
