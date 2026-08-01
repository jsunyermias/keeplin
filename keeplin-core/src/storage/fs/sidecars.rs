// md:Overview
use std::path::Path;

use chrono::{DateTime, Utc};

use crate::error::StorageError;
use crate::storage::note_log::{self, resolve, VersionVector, Winner};

use super::FsBackend;

// md:impl FsBackend
impl FsBackend {
    // md:impl FsBackend > fn sidecar_vv
    pub(super) async fn sidecar_vv(&self, path: &Path) -> Result<VersionVector, StorageError> {
        #[derive(serde::Deserialize)]
        struct VvProbe {
            #[serde(default)]
            vv: VersionVector,
        }
        if !path.exists() {
            return Ok(VersionVector::new());
        }
        let bytes = tokio::fs::read(path).await?;
        let probe: VvProbe = serde_json::from_slice(&bytes)
            .map_err(|e| StorageError::CorruptedData(format!("ndjson decode: {e}")))?;
        Ok(probe.vv)
    }

    // md:impl FsBackend > fn next_sidecar_vv
    pub(super) async fn next_sidecar_vv(&self, path: &Path) -> Result<VersionVector, StorageError> {
        let mut vv = self.sidecar_vv(path).await?;
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    // md:impl FsBackend > fn sidecar_incoming_wins
    pub(super) async fn sidecar_incoming_wins(
        &self,
        path: &Path,
        incoming_vv: &VersionVector,
        incoming_updated: DateTime<Utc>,
        incoming_writer: &str,
    ) -> Result<bool, StorageError> {
        #[derive(serde::Deserialize)]
        struct MetaProbe {
            updated_at: DateTime<Utc>,
            #[serde(default)]
            vv: VersionVector,
            #[serde(default)]
            last_writer: String,
        }
        if !path.exists() {
            return Ok(true);
        }
        let bytes = tokio::fs::read(path).await?;
        let m: MetaProbe = serde_json::from_slice(&bytes)
            .map_err(|e| StorageError::CorruptedData(format!("ndjson decode: {e}")))?;
        Ok(matches!(
            resolve(
                &m.vv,
                m.updated_at,
                &m.last_writer,
                incoming_vv,
                incoming_updated,
                incoming_writer,
            ),
            Winner::Incoming
        ))
    }
}
