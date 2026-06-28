use crate::{
    error::SyncError,
    models::{now, Change},
    storage::StorageBackend,
};

/// Orchestrates a single synchronisation cycle for any `StorageBackend`.
pub struct SyncEngine<T: StorageBackend> {
    pub backend: T,
}

impl<T: StorageBackend> SyncEngine<T> {
    pub fn new(backend: T) -> Self {
        Self { backend }
    }

    /// Run one full sync cycle.
    ///
    /// Returns the list of remote changes that were applied locally.
    pub async fn sync(&self) -> Result<Vec<Change>, SyncError> {
        // 1. Last known sync point
        let last_sync = self.backend.get_last_sync_time().await?;
        tracing::info!(last_sync = %last_sync, "Starting sync");

        // 2. Local changes since last sync
        let local_changes = self.backend.get_changes_since(last_sync).await?;
        tracing::info!(count = local_changes.len(), "Local changes collected");

        // 3. Push local changes to the remote peer
        self.backend.send_changes(local_changes).await?;
        tracing::info!("Local changes sent");

        // 4. Pull remote changes
        let remote_changes = self.backend.receive_changes().await?;
        tracing::info!(count = remote_changes.len(), "Remote changes received");

        // 5. Apply each remote change locally
        for change in &remote_changes {
            self.backend.apply_change(change.clone()).await?;
        }
        tracing::debug!(applied = remote_changes.len(), "Remote changes applied");

        // 6. Persist new sync timestamp
        let sync_ts = now();
        self.backend.update_sync_time(sync_ts).await?;
        tracing::info!(new_sync_ts = %sync_ts, "Sync complete");

        Ok(remote_changes)
    }
}
