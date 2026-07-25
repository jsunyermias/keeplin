// md:Overview
use thiserror::Error;

// md:StorageError
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(String),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Corrupted data: {0}")]
    CorruptedData(String),

    #[error("Too large: {0}")]
    TooLarge(String),
}

// md:impl From libsql for StorageError
impl From<libsql::Error> for StorageError {
    fn from(e: libsql::Error) -> Self {
        let mut msg = e.to_string();
        let mut src: Option<&dyn std::error::Error> = std::error::Error::source(&e);
        while let Some(cause) = src {
            msg.push_str(&format!("\n  caused by: {cause}"));
            src = cause.source();
        }
        StorageError::Database(msg)
    }
}

// md:impl From tungstenite for StorageError
impl From<tokio_tungstenite::tungstenite::Error> for StorageError {
    fn from(e: tokio_tungstenite::tungstenite::Error) -> Self {
        StorageError::WebSocket(e.to_string())
    }
}

// md:SyncError
#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("Conflict: local={local_id}, remote={remote_id}")]
    Conflict { local_id: String, remote_id: String },

    #[error("Sync failed: {0}")]
    Failed(String),
}
