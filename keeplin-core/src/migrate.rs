//! One-shot state migration between any two [`StorageBackend`]s.
//!
//! The two backends ([`crate::storage::fs::FsBackend`] and
//! [`crate::storage::db::DbBackend`]) use different, asymmetric sync channels — filesystem
//! version-vector logs replicated by Syncthing versus a WebSocket change journal — so their
//! `Change` streams are **not** interchangeable, and a raw `get_changes_since → apply_change`
//! bridge would silently drop notes in both directions.
//!
//! [`migrate`] sidesteps that entirely. It copies the **current live state** of the source
//! into the destination using the typed `create_*` methods, which every layer already
//! implements correctly:
//! - `create_note` persists id/timestamps/`alias`/`bookmarks`/`links` verbatim and, on
//!   `DbBackend`, rebuilds the `note_links` backlink projection.
//! - `create_note` on `FsBackend` writes a proper per-device version-vector log, so the note
//!   enters the filesystem model natively.
//! - When the source or destination is wrapped in [`crate::encryption::EncryptedBackend`],
//!   reads decrypt and writes encrypt automatically — so each side uses its own key.
//!
//! This makes migration work for every combination: `Fs ↔ Db`, plaintext ↔ encrypted, and a
//! different encryption key on each side.
//!
//! # Scope (deliberate limitations)
//!
//! - **Live state only.** The `list_*` methods exclude soft-deleted rows, so tombstones are
//!   not carried. A migration is a fresh start; deleted items stay deleted.
//! - **Empty destination.** Entities are inserted with their original ids; importing an id
//!   that already exists in the destination errors (e.g. `DbBackend::create_note` is a plain
//!   `INSERT`). Migrate into a fresh destination.
//! - This is a **one-shot copy**, not live sync. After it runs, each backend keeps using its
//!   own native replication.

use crate::{
    error::StorageError,
    models::{NoteTag, Resource},
    storage::StorageBackend,
};

/// How many entities to request per page while exhausting the paginated `list_*` methods.
const PAGE: u32 = 500;

/// Counts of the entities copied by a [`migrate`] run, for reporting to the operator.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MigrationReport {
    /// Notebooks copied.
    pub notebooks: usize,
    /// Tags copied.
    pub tags: usize,
    /// Notes copied.
    pub notes: usize,
    /// Note↔tag associations copied.
    pub note_tags: usize,
    /// Resources copied (metadata + binary payload).
    pub resources: usize,
}

/// Copy every live entity from `src` into `dst`.
///
/// Order matters so references resolve as entities land: notebooks and tags first (a note
/// carries a `notebook_id`), then notes, then note↔tag associations, then resources. Each
/// entity is written with the same typed `create_*`/`add_note_tag` call the API surfaces use,
/// so the destination stores it exactly as if a client had created it — including deriving
/// the destination's own indexes and applying its own at-rest encryption.
///
/// Returns a [`MigrationReport`] with per-entity counts. Fails fast on the first error
/// (leaving whatever has already been written in place); see the module docs for the
/// empty-destination expectation.
pub async fn migrate(
    src: &dyn StorageBackend,
    dst: &dyn StorageBackend,
) -> Result<MigrationReport, StorageError> {
    let mut report = MigrationReport::default();

    // Notebooks (before notes: a note may reference a notebook_id).
    for notebook in collect(|token| src.list_notebooks(PAGE, token)).await? {
        dst.create_notebook(notebook).await?;
        report.notebooks += 1;
    }

    // Tags (before note↔tag associations).
    for tag in collect(|token| src.list_tags(PAGE, token)).await? {
        dst.create_tag(tag).await?;
        report.tags += 1;
    }

    // Notes, then their tag associations. `alias`/`bookmarks`/`links` ride along as note
    // fields; the destination's `create_note` rebuilds any backlink index from them.
    let notes = collect(|token| src.list_notes(PAGE, token)).await?;
    for note in &notes {
        dst.create_note(note.clone()).await?;
        report.notes += 1;
    }
    for note in &notes {
        for tag in collect(|token| src.list_note_tags(note.id, PAGE, token)).await? {
            dst.add_note_tag(NoteTag {
                note_id: note.id,
                tag_id: tag.id,
            })
            .await?;
            report.note_tags += 1;
        }
    }

    // Resources: metadata comes from the list, the bytes from a full read.
    for meta in collect(|token| src.list_resources(PAGE, token)).await? {
        let (resource, data): (Resource, Vec<u8>) = src.read_resource(meta.id).await?;
        dst.create_resource(resource, data).await?;
        report.resources += 1;
    }

    Ok(report)
}

/// Exhaust a paginated `list_*` call into a single `Vec`.
///
/// `page` is any closure that takes an `Option<page_token>` and returns one
/// `(items, next_token)` page — matching every `list_*` method's shape — so a single helper
/// drives notebooks, tags, notes, note-tags, and resources alike.
async fn collect<T, F, Fut>(mut page: F) -> Result<Vec<T>, StorageError>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<T>, Option<String>), StorageError>>,
{
    let mut out = Vec::new();
    let mut token = None;
    loop {
        let (items, next) = page(token).await?;
        out.extend(items);
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    Ok(out)
}
