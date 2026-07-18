// md:Overview
use crate::{
    error::SyncError,
    models::{now, Change},
    storage::StorageBackend,
};

// md:SyncStage
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStage {
    Collecting,
    Sending,
    Receiving,
    Applying,
    Done,
}

// md:fn run_sync
pub async fn run_sync<B, F>(backend: &B, mut report: F) -> Result<Vec<Change>, SyncError>
where
    B: StorageBackend + ?Sized,
    F: FnMut(SyncStage, usize),
{
    let last_sync = backend.get_last_sync_time().await?;
    tracing::info!(last_sync = %last_sync, "Starting sync");

    let sync_ts = now();

    report(SyncStage::Collecting, 0);
    let local_changes = backend.get_changes_since(last_sync).await?;
    tracing::info!(count = local_changes.len(), "Local changes collected");

    report(SyncStage::Sending, local_changes.len());
    backend.send_changes(local_changes).await?;
    tracing::info!("Local changes sent");

    report(SyncStage::Receiving, 0);
    let remote_changes = backend.receive_changes().await?;
    tracing::info!(count = remote_changes.len(), "Remote changes received");

    report(SyncStage::Applying, remote_changes.len());
    for change in &remote_changes {
        backend.apply_change(change.clone()).await?;
    }
    tracing::debug!(applied = remote_changes.len(), "Remote changes applied");

    backend.update_sync_time(sync_ts).await?;
    tracing::info!(new_sync_ts = %sync_ts, "Sync complete");

    report(SyncStage::Done, remote_changes.len());
    Ok(remote_changes)
}

// md:SyncEngine
pub struct SyncEngine<T: StorageBackend> {
    pub backend: T,
}

// md:impl SyncEngine
impl<T: StorageBackend> SyncEngine<T> {
    // md:impl SyncEngine > fn new
    pub fn new(backend: T) -> Self {
        Self { backend }
    }

    // md:impl SyncEngine > fn sync
    pub async fn sync(&self) -> Result<Vec<Change>, SyncError> {
        run_sync(&self.backend, |_, _| {}).await
    }
}
