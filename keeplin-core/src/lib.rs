//! Root of the `keeplin-core` library crate.
//!
//! This file declares the public sub-modules that make up the complete Keeplin storage and
//! synchronisation layer. It contains no logic itself; every concrete type and trait lives
//! in one of the sub-modules below.
//!
//! # Module overview
//!
//! - [`encryption`] — `EncryptedBackend<B>`: transparent AES-256-GCM at-rest encryption
//!   decorator that wraps any [`storage::StorageBackend`].
//! - [`error`] — `StorageError` and `SyncError`: all error types used across the crate.
//! - [`links`] — bookmark/link types and the pure `#…` reference + `###` bookmark grammar.
//! - [`linking`] — `LinkingBackend<B>`: decorator that derives bookmarks/links from each
//!   note body, resolves references, and enforces alias uniqueness.
//! - [`models`] — Domain types (`Note`, `Notebook`, `Tag`, `Resource`, `Change`, …).
//! - [`storage`] — `StorageBackend` trait plus `FsBackend` and `DbBackend` implementations.
//! - [`sync`] — `SyncEngine`: orchestrates a full push-then-pull sync cycle.

pub mod encryption;
pub mod error;
pub mod linking;
pub mod links;
pub mod models;
pub mod storage;
pub mod sync;
