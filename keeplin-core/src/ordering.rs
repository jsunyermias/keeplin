//! The Inbox system notebook, pinning, manual ordering, and starring.
//!
//! # The model
//!
//! Every note belongs to exactly one notebook (`Note::notebook_id` is never "none"); a
//! note created without choosing one lands in the **Inbox** — a system notebook with the
//! fixed nil UUID and the title "Pizarra", auto-created at daemon startup and protected
//! from user deletion by the API surfaces.
//!
//! Within a notebook, notes order by `(effective sort_key ASC, id ASC)`:
//!
//! - **Normal notebooks** have two bands: `1..=999` is the **pinned** band (rendered
//!   `0.001–0.999`, shown at the top, at most [`MAX_PINNED`] notes) and `>= 1000` the
//!   **normal** band. New notes enter the normal band after its current last note.
//! - **The Inbox** is one flat band with no pinning; new notes are inserted at the
//!   **top** (`min(existing) - 1`, resequencing first if the space above is exhausted).
//! - `sort_key == 0` is the legacy "never positioned" sentinel, ordered as
//!   [`Note::DEFAULT_SORT_KEY`] (the start of the normal band).
//!
//! **Starring** ([`star_note`]/[`unstar_note`]) is a global flag, fully orthogonal to
//! pinning and to the notebook — it never moves the note.
//!
//! # Where the logic lives
//!
//! Like [`crate::linking`]'s alias helpers, the operations here are free functions over
//! `&dyn StorageBackend` doing read-modify-write through the normal `update_note` path,
//! so version vectors, encryption, link derivation, and the live-change feed all apply
//! unchanged and the resulting state syncs like any other edit. Placement decisions read
//! one [`NotebookSortProfile`] — a compact per-notebook summary each backend computes
//! natively — instead of materializing the notebook's notes.
//!
//! # Concurrency and sync
//!
//! Sort keys are plain note fields, so concurrent edits resolve like any other note
//! conflict (whole-note version-vector resolution with the deterministic tiebreak). Two
//! devices pinning concurrently can pick the same key; ordering stays deterministic
//! (`id` breaks ties) and the next reorder can separate them. Old peers ignore the new
//! fields (serde/protobuf defaults), and notes they write keep `sort_key = 0`, which
//! orders them at the start of the normal band.

use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Note, Notebook},
    storage::{NotebookSortProfile, StorageBackend},
};

/// The Inbox's fixed identity: the nil UUID on every device, so it never needs to sync
/// an id and can be addressed before it exists.
pub const INBOX_ID: Uuid = Uuid::nil();

/// The Inbox's title.
pub const INBOX_TITLE: &str = "Pizarra";

/// Highest sort key of the pinned band (`1..=PIN_MAX`), and therefore also the maximum
/// number of pinned notes per notebook.
pub const PIN_MAX: u32 = 999;

/// Maximum pinned notes per notebook (the pinned band simply has no more keys).
pub const MAX_PINNED: usize = PIN_MAX as usize;

/// First sort key of the normal (unpinned) band.
pub const NORMAL_START: u32 = Note::DEFAULT_SORT_KEY;

/// Spacing used when the Inbox is resequenced (leaves room above and between notes).
const RESEQUENCE_STEP: u32 = 1000;

/// Create the Inbox system notebook if this store does not have it yet. Idempotent —
/// call it at every startup. Two devices creating it concurrently converge like any
/// other notebook conflict (both sides write the same fixed id).
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

/// Whether `id` names the Inbox. The API surfaces use this to refuse deleting it.
pub fn is_inbox(id: Uuid) -> bool {
    id == INBOX_ID
}

/// Assign a brand-new note its initial position, honouring a caller-chosen `sort_key`
/// when one was set (`!= 0`). Call before `create_note` (the daemon does, on every
/// create surface):
///
/// - In the **Inbox**, the note is inserted at the **top**: `min(existing) - 1`. When
///   the space above is exhausted (the minimum would fall to the `0` sentinel), the
///   Inbox is resequenced first (see [`resequence_inbox`]).
/// - In a **normal notebook**, the note enters the **normal band** after its current
///   last note: `max(normal band) + 1`, or [`NORMAL_START`] in an empty band.
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
            // `min - 1` must never reach the 0 sentinel; make room first.
            Some(min) if min <= 1 => resequence_inbox(backend).await? - 1,
            Some(min) => min - 1,
            None => NORMAL_START,
        }
    } else {
        next_normal_key(&profile)
    };
    Ok(())
}

/// Pin a note: move it into its notebook's pinned band at the lowest free key.
///
/// Returns the updated note. Fails with [`StorageError::InvalidInput`] on an Inbox note
/// (the Inbox has no pinning) and with [`StorageError::Conflict`] when the notebook
/// already has [`MAX_PINNED`] pinned notes. Pinning an already-pinned note is a no-op
/// that returns it unchanged.
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

/// Unpin a note: move it to the end of its notebook's normal band. Returns the updated
/// note; unpinning a note that is not pinned is a no-op that returns it unchanged.
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

/// Star a note (a global flag; the note's position never changes). Idempotent.
pub async fn star_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    set_starred(backend, id, true).await
}

/// Remove a note's star. Idempotent.
pub async fn unstar_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    set_starred(backend, id, false).await
}

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

/// Give a note a new manual position **within its current band**. Returns the updated
/// note.
///
/// - A **pinned** note accepts keys in `1..=`[`PIN_MAX`]; anything else is
///   [`StorageError::InvalidInput`] (unpin it instead of renumbering it out).
/// - A **normal** note accepts keys `>= `[`NORMAL_START`].
/// - An **Inbox** note accepts any key `>= 1` (`0` is the legacy sentinel).
///
/// Duplicate keys are allowed — ordering stays deterministic (`id` breaks ties).
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

/// Renumber every live Inbox note to `1000, 2000, 3000, …` in its current order,
/// returning the new minimum key. Called when top-insertion has consumed all the room
/// above the first note; the wide spacing makes the next resequence far away. Each
/// renumbered note goes through `update_note`, so the moves version and sync normally.
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

/// The lowest key in `1..=PIN_MAX` not present in `used` (which the profile returns
/// sorted ascending), or `None` when the band is full.
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

/// The key after the last note of the normal band: `max + 1`, or [`NORMAL_START`] when
/// the band is empty.
fn next_normal_key(profile: &NotebookSortProfile) -> u32 {
    profile
        .max_normal_key
        .map(|max| max.saturating_add(1))
        .unwrap_or(NORMAL_START)
}

/// Read a note for a user-facing read-modify-write, rejecting a tombstone as `NotFound`
/// (mirrors `linking`'s helper: ordering edits must never revive a deleted note).
async fn read_live_note(backend: &dyn StorageBackend, id: Uuid) -> Result<Note, StorageError> {
    let note = backend.read_note(id).await?;
    if note.deleted_at.is_some() {
        return Err(StorageError::NotFound(id.to_string()));
    }
    Ok(note)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::fs::FsBackend;
    use crate::storage::{NoteRepository, NotebookRepository};

    async fn backend() -> FsBackend {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        FsBackend::new(&path).await.unwrap()
    }

    /// Create a note through the daemon's placement rule (what every API create does).
    async fn create_placed(be: &FsBackend, title: &str, notebook: Uuid) -> Note {
        let mut note = Note::new(title, "");
        note.notebook_id = notebook;
        place_new_note(be, &mut note).await.unwrap();
        be.create_note(note).await.unwrap()
    }

    fn titles(page: &[Note]) -> Vec<&str> {
        page.iter().map(|n| n.title.as_str()).collect()
    }

    #[tokio::test]
    async fn ensure_inbox_is_idempotent_and_fixed() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        ensure_inbox(&be).await.unwrap();
        let inbox = be.read_notebook(INBOX_ID).await.unwrap();
        assert_eq!(inbox.title, INBOX_TITLE);
        assert_eq!(inbox.id, INBOX_ID);
    }

    /// New Inbox notes go to the top; new notebook notes go to the end of the normal band.
    #[tokio::test]
    async fn placement_inbox_top_notebook_bottom() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();

        // Inbox: each new note lands above the previous one.
        create_placed(&be, "first", INBOX_ID).await;
        create_placed(&be, "second", INBOX_ID).await;
        create_placed(&be, "third", INBOX_ID).await;
        let (page, _) = be.list_notes_in_notebook(INBOX_ID, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["third", "second", "first"]);

        // Normal notebook: each new note lands below the previous one.
        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let a = create_placed(&be, "a", nb.id).await;
        let b = create_placed(&be, "b", nb.id).await;
        assert_eq!(a.sort_key, NORMAL_START);
        assert_eq!(b.sort_key, NORMAL_START + 1);
        let (page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["a", "b"]);
    }

    /// Pinning moves a note into the pinned band (listed first); unpinning appends it to
    /// the normal band. Inbox notes cannot be pinned.
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

    /// Reordering stays inside the note's band and rejects keys outside it.
    #[tokio::test]
    async fn reorder_respects_bands() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let nb = be.create_notebook(Notebook::new("nb")).await.unwrap();
        let a = create_placed(&be, "a", nb.id).await;
        let b = create_placed(&be, "b", nb.id).await;

        // Normal band: move b above a.
        reorder_note(&be, b.id, NORMAL_START).await.unwrap();
        reorder_note(&be, a.id, NORMAL_START + 5).await.unwrap();
        let (page, _) = be.list_notes_in_notebook(nb.id, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["b", "a"]);

        // A normal note cannot take a pinned-band key, and vice versa.
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

    /// Starring is global and orthogonal: the starred list spans notebooks, and the
    /// note's position never changes.
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

    /// When top-insertion exhausts the space above the first Inbox note, the Inbox is
    /// resequenced and insertion keeps working — order preserved throughout.
    #[tokio::test]
    async fn inbox_top_insert_survives_underflow_by_resequencing() {
        let be = backend().await;
        ensure_inbox(&be).await.unwrap();
        let first = create_placed(&be, "old-top", INBOX_ID).await;
        // Simulate a long history of top-inserts: manually push the head to key 1.
        reorder_note(&be, first.id, 1).await.unwrap();

        let newcomer = create_placed(&be, "new-top", INBOX_ID).await;
        assert!(newcomer.sort_key >= 1, "never the 0 sentinel");
        let (page, _) = be.list_notes_in_notebook(INBOX_ID, 0, None).await.unwrap();
        assert_eq!(titles(&page), ["new-top", "old-top"]);
    }

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
