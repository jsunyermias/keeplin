// md:Overview
use crate::{
    error::StorageError,
    models::{NoteTag, Resource},
    storage::StorageBackend,
};

// md:PAGE
const PAGE: u32 = 500;

// md:MigrationReport
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MigrationReport {
    pub notebooks: usize,
    pub tags: usize,
    pub notes: usize,
    pub note_tags: usize,
    pub resources: usize,
}

// md:fn migrate
pub async fn migrate(
    src: &dyn StorageBackend,
    dst: &dyn StorageBackend,
) -> Result<MigrationReport, StorageError> {
    let mut report = MigrationReport::default();

    for notebook in collect(|token| src.list_notebooks(PAGE, token)).await? {
        dst.create_notebook(notebook).await?;
        report.notebooks += 1;
    }

    for tag in collect(|token| src.list_tags(PAGE, token)).await? {
        dst.create_tag(tag).await?;
        report.tags += 1;
    }

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

    for meta in collect(|token| src.list_resources(PAGE, token)).await? {
        let (resource, data): (Resource, Vec<u8>) = src.read_resource(meta.id).await?;
        dst.create_resource(resource, data).await?;
        report.resources += 1;
    }

    Ok(report)
}

// md:fn collect
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
