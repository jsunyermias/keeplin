use thiserror::Error;

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

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Invalid state: {0}")]
    InvalidState(String),
}

impl From<libsql::Error> for StorageError {
    fn from(e: libsql::Error) -> Self {
        StorageError::Database(e.to_string())
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for StorageError {
    fn from(e: tokio_tungstenite::tungstenite::Error) -> Self {
        StorageError::WebSocket(e.to_string())
    }
}

impl From<reqwest::Error> for StorageError {
    fn from(e: reqwest::Error) -> Self {
        StorageError::Http(e.to_string())
    }
}

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("Conflict: local={local_id}, remote={remote_id}")]
    Conflict {
        local_id: String,
        remote_id: String,
    },

    #[error("Sync failed: {0}")]
    Failed(String),
}
