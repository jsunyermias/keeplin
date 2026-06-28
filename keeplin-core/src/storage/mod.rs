//! Storage layer for Keeplin.
//!
//! This module provides the [`StorageBackend`] trait that every storage implementation
//! must satisfy, plus two concrete implementations:
//!
//! - [`fs::FsBackend`] — stores data as JSON files on the local filesystem and uses
//!   per-device NDJSON change logs that Syncthing (or any compatible tool) can replicate
//!   across devices.
//! - [`db::DbBackend`] — stores data in a local LibSQL (SQLite-compatible) database and
//!   synchronises with a central server over a WebSocket connection.

mod backend;
pub mod db;
pub mod fs;
pub mod note_log;

pub use backend::{
    NoteRepository, NotebookRepository, ResourceRepository, StorageBackend, SyncBackend,
    TagRepository,
};
