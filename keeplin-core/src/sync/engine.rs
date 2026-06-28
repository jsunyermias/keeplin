//! The `SyncEngine` — drives a complete push-then-pull synchronisation cycle.
//!
//! This module is intentionally thin: it sequences six operations against a
//! [`StorageBackend`] and handles the sync-timestamp bookkeeping. All real work
//! (collecting changes, sending, receiving, applying) is delegated to the backend.
//!
//! The cycle itself lives in the free function [`run_sync`], which takes a progress
//! callback so callers that want to surface per-stage progress (such as the gRPC
//! daemon's streaming `Sync` RPC) and callers that do not (such as [`SyncEngine`])
//! share one implementation. This guarantees the watermark and ordering logic exists
//! in exactly one place.

use crate::{
    error::SyncError,
    models::{now, Change},
    storage::StorageBackend,
};

/// The stage a synchronisation cycle has reached, reported through the [`run_sync`]
/// progress callback. The variants mirror the natural ordering of the cycle so a UI
/// can render a determinate progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStage {
    /// About to collect local changes recorded since the last sync.
    Collecting,
    /// About to push the collected local changes to the remote peer.
    Sending,
    /// About to pull changes the remote peer has for this device.
    Receiving,
    /// About to apply the pulled remote changes to the local store.
    Applying,
    /// The cycle finished successfully.
    Done,
}

/// Runs one complete push-then-pull synchronisation cycle against `backend`, invoking
/// `report(stage, count)` immediately before each stage begins (and once more with
/// [`SyncStage::Done`] on success).
///
/// The six steps are:
/// 1. Read the last-sync timestamp from the backend.
/// 2. Collect all local changes recorded after that timestamp.
/// 3. Push the local changes to the remote peer.
/// 4. Pull all changes the remote peer has available.
/// 5. Apply each remote change to the local store in order.
/// 6. Persist the watermark captured at the *start* of the cycle as the new last-sync
///    timestamp.
///
/// The `count` passed to the callback is the number of changes relevant to the stage
/// (local changes for [`SyncStage::Sending`], remote changes for [`SyncStage::Applying`]
/// and [`SyncStage::Done`], and `0` otherwise).
///
/// # Returns
///
/// The list of remote changes that were applied to the local store during this cycle.
///
/// # Errors
///
/// Returns [`SyncError::Storage`] if any storage operation fails. The caller is
/// responsible for scheduling retries; a failed cycle leaves the last-sync timestamp
/// unchanged so the next cycle re-collects and re-applies any missed changes.
pub async fn run_sync<B, F>(backend: &B, mut report: F) -> Result<Vec<Change>, SyncError>
where
    B: StorageBackend + ?Sized,
    F: FnMut(SyncStage, usize),
{
    // Step 1: Read the timestamp of the most recent successful sync so we know which
    // local changes are "new" and have not yet been sent to the remote peer.
    let last_sync = backend.get_last_sync_time().await?;
    tracing::info!(last_sync = %last_sync, "Starting sync");

    // Capture the new watermark *before* collecting changes. Any mutation recorded while
    // this cycle runs (steps 2–5) will have a `changed_at` greater than `sync_ts`, so it
    // is guaranteed to be collected on the next cycle. Capturing the watermark at the end
    // (after collection) would instead skip over changes written during the cycle,
    // silently dropping them from every future sync.
    let sync_ts = now();

    // Step 2: Collect all local changes recorded after the last-sync timestamp. These are
    // the mutations that other devices have not yet seen.
    report(SyncStage::Collecting, 0);
    let local_changes = backend.get_changes_since(last_sync).await?;
    tracing::info!(count = local_changes.len(), "Local changes collected");

    // Step 3: Push the local changes to the remote peer so that other devices can receive
    // them on their next sync cycle.
    report(SyncStage::Sending, local_changes.len());
    backend.send_changes(local_changes).await?;
    tracing::info!("Local changes sent");

    // Step 4: Pull all changes that the remote peer has accumulated from other devices
    // since our last pull.
    report(SyncStage::Receiving, 0);
    let remote_changes = backend.receive_changes().await?;
    tracing::info!(count = remote_changes.len(), "Remote changes received");

    // Step 5: Apply each remote change to the local store in the order they arrived.
    // apply_change is idempotent, so re-running this step after a partial failure is safe.
    report(SyncStage::Applying, remote_changes.len());
    for change in &remote_changes {
        backend.apply_change(change.clone()).await?;
    }
    tracing::debug!(applied = remote_changes.len(), "Remote changes applied");

    // Step 6: Persist the watermark captured at the start of this cycle as the new
    // last-sync point. Using the start time (not a fresh `now()`) ensures that any change
    // recorded *during* this cycle is still picked up on the next one.
    backend.update_sync_time(sync_ts).await?;
    tracing::info!(new_sync_ts = %sync_ts, "Sync complete");

    report(SyncStage::Done, remote_changes.len());
    Ok(remote_changes)
}

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
    /// This is a thin wrapper over [`run_sync`] with a no-op progress callback. See
    /// [`run_sync`] for the full description of the cycle, return value, and errors.
    pub async fn sync(&self) -> Result<Vec<Change>, SyncError> {
        run_sync(&self.backend, |_, _| {}).await
    }
}
