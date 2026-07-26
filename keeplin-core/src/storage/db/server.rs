// md:Overview

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Note, Notebook},
};

use crate::storage::backend::DEFAULT_HISTORY_LIMIT;
use crate::storage::{EntityVersion, HistoryRepository};

use super::DbBackend;

// md:ServerVersion
#[derive(Debug, serde::Deserialize)]
struct ServerVersion {
    timestamp: DateTime<Utc>,
    device_id: String,
    entity: Option<serde_json::Value>,
}

// md:CapabilityCache
pub(super) enum CapabilityCache {
    Unknown,
    Unavailable,
    Known(Vec<String>),
}

// md:fn http_base_of
pub(super) fn http_base_of(server_url: &str) -> Option<String> {
    let (scheme, rest) = if let Some(rest) = server_url.strip_prefix("wss://") {
        ("https://", rest)
    } else {
        ("http://", server_url.strip_prefix("ws://")?)
    };
    let rest = rest.strip_suffix("/api/sync").unwrap_or(rest);
    Some(format!("{scheme}{}", rest.trim_end_matches('/')))
}

// md:impl DbBackend (server history)
impl DbBackend {
    // md:impl DbBackend (server history) > fn server_http_base
    fn server_http_base(&self) -> Option<String> {
        http_base_of(&self.server_url)
    }

    // md:impl DbBackend (server history) > fn server_has_capability
    async fn server_has_capability(&self, capability: &str) -> Option<bool> {
        let mut cache = self.server_capabilities.lock().await;
        if let CapabilityCache::Unknown = &*cache {
            *cache = match self.server_http_base() {
                Some(base) => {
                    let url = format!("{base}/version");
                    match self.http.get(&url).send().await {
                        Ok(r) if r.status().is_success() => {
                            match r.json::<serde_json::Value>().await {
                                Ok(v) => {
                                    let caps = v
                                        .get("capabilities")
                                        .and_then(|c| c.as_array())
                                        .map(|a| {
                                            a.iter()
                                                .filter_map(|x| x.as_str().map(String::from))
                                                .collect::<Vec<_>>()
                                        })
                                        .unwrap_or_default();
                                    CapabilityCache::Known(caps)
                                }
                                Err(_) => CapabilityCache::Unavailable,
                            }
                        }
                        _ => CapabilityCache::Unavailable,
                    }
                }
                None => CapabilityCache::Unavailable,
            };
        }
        match &*cache {
            CapabilityCache::Known(caps) => Some(caps.iter().any(|c| c == capability)),
            _ => None,
        }
    }

    // md:impl DbBackend (server history) > fn server_entity_history
    async fn server_entity_history<T: serde::de::DeserializeOwned>(
        &self,
        entity_type: &str,
        id: Uuid,
        cap: u32,
    ) -> Option<Vec<EntityVersion<T>>> {
        use std::sync::atomic::Ordering;
        if self.history_unsupported.load(Ordering::Relaxed) {
            return None;
        }
        if self.server_has_capability("history").await == Some(false) {
            self.history_unsupported.store(true, Ordering::Relaxed);
            return None;
        }
        let base = self.server_http_base()?;
        let url = format!("{base}/api/{entity_type}s/{id}/history?limit={cap}");
        let response = match self
            .http
            .get(&url)
            .bearer_auth(&self.auth_token)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(%url, "server history unreachable, using local journal: {e}");
                return None;
            }
        };
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            self.history_unsupported.store(true, Ordering::Relaxed);
            tracing::debug!(%url, "server has no history endpoint; using the local journal");
            return None;
        }
        let response = match response.error_for_status() {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(%url, "server history error, using local journal: {e}");
                return None;
            }
        };
        let versions: Vec<ServerVersion> = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(%url, "malformed server history, using local journal: {e}");
                return None;
            }
        };
        Some(
            versions
                .into_iter()
                .filter_map(|v| {
                    let entity = match v.entity {
                        Some(raw) => Some(serde_json::from_value::<T>(raw).ok()?),
                        None => None,
                    };
                    Some(EntityVersion {
                        timestamp: v.timestamp,
                        device_id: v.device_id,
                        entity,
                    })
                })
                .collect(),
        )
    }

    // md:impl DbBackend (server history) > fn entity_history
    async fn entity_history<T: serde::de::DeserializeOwned>(
        &self,
        entity_type: &str,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<T>>, StorageError> {
        let cap = if limit == 0 {
            DEFAULT_HISTORY_LIMIT
        } else {
            limit
        };
        if let Some(versions) = self.server_entity_history::<T>(entity_type, id, cap).await {
            return Ok(versions);
        }
        let _read_guard = self.lock.read().await;
        let mut rows = self
            .conn
            .query(
                "SELECT operation, changed_at, data
                 FROM entity_changes
                 WHERE entity_type = ?1 AND entity_id = ?2
                 ORDER BY id DESC
                 LIMIT ?3",
                libsql::params![entity_type, id.to_string(), cap as i64],
            )
            .await?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            let operation: String = row.get(0)?;
            let changed_at = Self::parse_required_dt(row.get::<String>(1)?)?;
            let data_str: Option<String> = row.get(2)?;
            let entity = match operation.as_str() {
                "create" | "update" => {
                    match data_str
                        .as_deref()
                        .and_then(|s| serde_json::from_str::<T>(s).ok())
                    {
                        Some(e) => Some(e),
                        None => continue,
                    }
                }
                "delete" => None,
                _ => continue,
            };
            out.push(EntityVersion {
                timestamp: changed_at,
                device_id: self.device_id.clone(),
                entity,
            });
        }
        Ok(out)
    }
}

// md:impl HistoryRepository for DbBackend
#[async_trait]
impl HistoryRepository for DbBackend {
    // md:impl HistoryRepository for DbBackend > fn note_history
    async fn note_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Note>>, StorageError> {
        self.entity_history::<Note>("note", id, limit).await
    }

    // md:impl HistoryRepository for DbBackend > fn notebook_history
    async fn notebook_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Notebook>>, StorageError> {
        self.entity_history::<Notebook>("notebook", id, limit).await
    }
}
