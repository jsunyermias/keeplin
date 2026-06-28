//! Error types used throughout `keeplin-core`.
//!
//! All storage operations return [`StorageError`]. The synchronisation layer wraps that
//! in [`SyncError`] when it needs to add sync-specific failure cases (e.g. conflicts
//! detected between local and remote changes).
//!
//! Conversions from third-party error types (`libsql::Error`, `reqwest::Error`,
//! `tungstenite::Error`) are provided via `From` implementations so that callers can use
//! the `?` operator without manual mapping.

use thiserror::Error;

/// Every error that can arise from a storage operation.
///
/// Variants that wrap third-party errors (`Io`, `Serialization`, `WebSocket`, `Http`)
/// use `#[from]` so they are automatically constructed by the `?` operator.
/// Variants that carry a `String` (`Database`, `NotFound`, `Conflict`, `InvalidState`)
/// are constructed manually because their source types have no single `From` target.
#[derive(Debug, Error)]
pub enum StorageError {
    /// A filesystem I/O error occurred while reading or writing data on disk.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A JSON serialisation or deserialisation error occurred (e.g. a corrupt log entry).
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// A LibSQL or SQLite database error occurred.
    /// The `String` payload includes the full error chain (each nested cause is appended
    /// on a new line) so that the root cause is always visible in logs.
    #[error("Database error: {0}")]
    Database(String),

    /// A WebSocket protocol or connection error occurred during sync.
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    /// An HTTP request error occurred (e.g. a sync server call failed).
    #[error("HTTP error: {0}")]
    Http(String),

    /// The requested entity does not exist in the store (or was soft-deleted).
    /// The `String` payload contains a human-readable description of which entity was
    /// not found (e.g. `"note 3f4a…"`).
    #[error("Not found: {0}")]
    NotFound(String),

    /// A write conflict was detected between two concurrent mutations.
    #[error("Conflict: {0}")]
    Conflict(String),

    /// An operation failed because of an unexpected internal state.
    /// This variant is used for key-derivation errors and general unexpected conditions.
    #[error("Invalid state: {0}")]
    InvalidState(String),

    /// Stored data could not be decrypted because it is corrupt or was encrypted with
    /// a different key. This is raised when the AES-GCM authentication tag verification
    /// fails, which happens when the wrong password is used or when the ciphertext has
    /// been tampered with after it was written.
    #[error("Corrupted data: {0}")]
    CorruptedData(String),
}

impl From<libsql::Error> for StorageError {
    /// Converts a `libsql::Error` into a `StorageError::Database`.
    ///
    /// The entire error source chain is flattened into one `String` so that the
    /// underlying SQLite error message is not lost when the `libsql::Error` is dropped.
    fn from(e: libsql::Error) -> Self {
        let mut msg = e.to_string();
        // Walk the full source chain so nested SQLite error codes are included.
        let mut src: Option<&dyn std::error::Error> = std::error::Error::source(&e);
        while let Some(cause) = src {
            msg.push_str(&format!("\n  caused by: {cause}"));
            src = cause.source();
        }
        StorageError::Database(msg)
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for StorageError {
    /// Converts a `tungstenite::Error` (WebSocket protocol error) into
    /// `StorageError::WebSocket`.
    fn from(e: tokio_tungstenite::tungstenite::Error) -> Self {
        StorageError::WebSocket(e.to_string())
    }
}

impl From<reqwest::Error> for StorageError {
    /// Converts a `reqwest::Error` (HTTP client error) into `StorageError::Http`.
    fn from(e: reqwest::Error) -> Self {
        StorageError::Http(e.to_string())
    }
}

/// Errors specific to the synchronisation layer.
///
/// The sync layer builds on top of storage operations, so `SyncError` wraps
/// `StorageError` for the common case where a storage call fails during sync.
/// Sync-specific failure modes (conflicts, general failures) have their own variants.
#[derive(Debug, Error)]
pub enum SyncError {
    /// A storage operation failed during the sync cycle.
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    /// A remote change conflicts with a local change for the same entity.
    /// `local_id` and `remote_id` identify the conflicting records for diagnostic
    /// purposes (they may be the same entity UUID with different content).
    #[error("Conflict: local={local_id}, remote={remote_id}")]
    Conflict { local_id: String, remote_id: String },

    /// The sync cycle failed for a reason not covered by the other variants
    /// (e.g. the remote peer returned an unexpected response format).
    #[error("Sync failed: {0}")]
    Failed(String),
}
