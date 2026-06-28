//! The `SyncEngine` — drives a complete push-then-pull synchronisation cycle.
//!
//! This module is intentionally thin: it sequences six operations against a
//! [`StorageBackend`] and handles the sync-timestamp bookkeeping. All real work
//! (collecting changes, sending, receiving, applying) is delegated to the backend.

use crate::{
    error::SyncError,
    models::{now, Change},
    storage::StorageBackend,
};

/// Orchestrates a single synchronisation cycle for any [`StorageBackend`].
///
/// `SyncEngine<T>` is generic so the compiler produces a monomorphised, zero-cost
/// implementation for each concrete backend type — there is no runtime dispatch.
pub struct SyncEngine<T: StorageBackend> {
    /// The storage backend that this engine drives. Exposed as `pub` so callers can
    /// access the backend directly without going through the engine (e.g. to perform
    /// CRUD operations between sync cycles).
    pub backend: T,
}

impl<T: StorageBackend> SyncEngine<T> {
    /// Wraps `backend` in a new engine.
    pub fn new(backend: T) -> Self {
        Self { backend }
    }

    /// Runs one complete push-then-pull synchronisation cycle.
    ///
    /// The six steps are:
    /// 1. Read the last-sync timestamp from the backend.
    /// 2. Collect all local changes recorded after that timestamp.
    /// 3. Push the local changes to the remote peer.
    /// 4. Pull all changes the remote peer has available.
    /// 5. Apply each remote change to the local store in order.
    /// 6. Persist the current UTC time as the new last-sync timestamp.
    ///
    /// # Returns
    ///
    /// The list of remote changes that were applied to the local store during this cycle.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Storage`] if any storage operation fails. The caller is
    /// responsible for scheduling retries; a failed cycle leaves the last-sync timestamp
    /// unchanged so the next cycle will re-collect and re-apply any missed changes.
    pub async fn sync(&self) -> Result<Vec<Change>, SyncError> {
        // Step 1: Read the timestamp of the most recent successful sync so we know
        // which local changes are "new" and have not yet been sent to the remote peer.
        let last_sync = self.backend.get_last_sync_time().await?;
        tracing::info!(last_sync = %last_sync, "Starting sync");

        // Step 2: Collect all local changes recorded after the last-sync timestamp.
        // These are the mutations that other devices have not yet seen.
        let local_changes = self.backend.get_changes_since(last_sync).await?;
        tracing::info!(count = local_changes.len(), "Local changes collected");

        // Step 3: Push the local changes to the remote peer so that other devices can
        // receive them on their next sync cycle.
        self.backend.send_changes(local_changes).await?;
        tracing::info!("Local changes sent");

        // Step 4: Pull all changes that the remote peer has accumulated from other
        // devices since our last pull.
        let remote_changes = self.backend.receive_changes().await?;
        tracing::info!(count = remote_changes.len(), "Remote changes received");

        // Step 5: Apply each remote change to the local store in the order they arrived.
        // apply_change is idempotent, so re-running this step after a partial failure
        // is safe — no data corruption will occur.
        for change in &remote_changes {
            self.backend.apply_change(change.clone()).await?;
        }
        tracing::debug!(applied = remote_changes.len(), "Remote changes applied");

        // Step 6: Persist the current time as the new last-sync point. The next sync
        // cycle will only collect changes recorded after this moment, avoiding
        // re-sending changes that were already pushed in this cycle.
        let sync_ts = now();
        self.backend.update_sync_time(sync_ts).await?;
        tracing::info!(new_sync_ts = %sync_ts, "Sync complete");

        Ok(remote_changes)
    }
}
