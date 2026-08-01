// md:Overview
use std::path::Path;

use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::error::StorageError;

use super::FsBackend;

// md:fn atomic_write
pub(super) async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let tmp = path.with_extension("tmp");
    let result = async {
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&tmp, path).await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

// md:impl FsBackend
impl FsBackend {
    // md:impl FsBackend > fn write_sidecar
    pub(super) async fn write_sidecar<T: serde::Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), StorageError> {
        let mut bytes = serde_json::to_vec(value)
            .map_err(|e| StorageError::InvalidState(format!("ndjson encode: {e}")))?;
        bytes.push(b'\n');
        atomic_write(path, &bytes).await
    }

    // md:impl FsBackend > fn read_sidecar
    pub(super) async fn read_sidecar<T: serde::de::DeserializeOwned>(
        &self,
        path: &Path,
        id: Uuid,
    ) -> Result<T, StorageError> {
        if !path.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let bytes = tokio::fs::read(path).await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| StorageError::CorruptedData(format!("ndjson decode: {e}")))
    }
}
