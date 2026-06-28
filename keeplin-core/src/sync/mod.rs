//! Synchronisation engine for Keeplin.
//!
//! This module exposes [`SyncEngine`], which orchestrates a complete push-then-pull
//! synchronisation cycle for any [`crate::storage::StorageBackend`]. A single call to
//! `SyncEngine::sync()` collects local changes, pushes them to the remote peer, pulls
//! remote changes, applies them locally, and updates the last-sync timestamp.

mod engine;

pub use engine::{run_sync, SyncEngine, SyncStage};
