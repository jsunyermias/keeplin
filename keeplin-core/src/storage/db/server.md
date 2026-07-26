# `storage/db/server.rs` — server capability probing and entity history

Self-contained companion for `keeplin-core/src/storage/db/server.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must
be able to understand it without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports the server-capability and history code needs. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Everything that talks to a remote Keeplin server about history: the capability probe and its cache, the HTTP base derivation, the server-side history fetch with local fallback, and the `HistoryRepository` implementation.

**Dependencies** — every binding above is either a crate this block's siblings call directly or a
path relocated from the pre-split `storage/db.rs`; expects: the symbols to keep the
signatures the block bodies below already rely on, since a changed signature fails to
compile rather than degrading silently.

**Used by** — the sibling modules of this directory module, and `crate::storage::db` through
`mod.rs`.

**Repeated context** — the directory module keeps `DbBackend`'s fields private in `mod.rs`;
Rust makes them visible to every descendant module, so siblings read them without any
widening. Items defined in one sibling and used by another carry `pub(super)`.

---

## ServerVersion

**Identification** — private deserialise struct; marker `// md:ServerVersion`.

**Code** — complete and verbatim:

```rust
// md:ServerVersion
#[derive(Debug, serde::Deserialize)]
struct ServerVersion {
    timestamp: DateTime<Utc>,
    device_id: String,
    entity: Option<serde_json::Value>,
}
```

**What it does** — One version as served by keeplin-srv's history endpoints
(`GET /api/{notes,notebooks}/:id/history`): the edit's instant, the authoring
sync device, and the snapshot exactly as pushed (`None` = tombstone). Encrypted
fields are still ciphertext here; `EncryptedBackend` decrypts on the way up,
same as for the local journal.

**Used by** — `server_entity_history`.

---

## CapabilityCache

**Identification** — private enum; marker `// md:CapabilityCache`.

**Code** — complete and verbatim:

```rust
// md:CapabilityCache
pub(super) enum CapabilityCache {
    Unknown,
    Unavailable,
    Known(Vec<String>),
}
```

**What it does** — Cached `GET /version` outcome (keeplin#114): `Unknown` (not
fetched — a lazy probe may retry), `Unavailable` (no `/version`; capabilities
indeterminate), `Known(Vec<String>)`.

**Used by** — the `server_capabilities` field, `server_has_capability`.

---

## fn http_base_of

**Identification** — `fn http_base_of(server_url: &str) -> Option<String>`;
marker `// md:fn http_base_of`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Derives the HTTP base from the WebSocket URL (`ws`→`http`,
`wss`→`https`, the `/api/sync` relay path stripped); `None` for empty or
non-WebSocket URLs (offline). A free function so `DbBackend::new` can run the
handshake before `self` exists.

**Used by** — `new`, `server_http_base`.

---

## impl DbBackend (server history)

**Identification** — the second inherent impl; marker
`// md:impl DbBackend (server history)`. Four methods.

**Code** — container: members documented as sub-blocks below: fn server_http_base, fn server_has_capability, fn server_entity_history, fn entity_history.

---

### fn server_http_base

**Identification** — marker
`// md:impl DbBackend (server history) > fn server_http_base`.

**Code** — complete and verbatim:

```rust
    // md:impl DbBackend (server history) > fn server_http_base
    fn server_http_base(&self) -> Option<String> {
        http_base_of(&self.server_url)
    }
```

**What it does** — `http_base_of(&self.server_url)`.

---

### fn server_has_capability

**Identification** — marker
`// md:impl DbBackend (server history) > fn server_has_capability`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Whether the server advertises `capability` at
`GET /version`, fetched once and cached: `Some(true/false)` when the server has
`/version`; `None` when it doesn't (older server) — the caller falls back to
feature-specific probing.

**Used by** — `server_entity_history`.

---

### fn server_entity_history

**Identification** — marker
`// md:impl DbBackend (server history) > fn server_entity_history`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Fetches an entity's history from the server (the durable
**cross-device** record). `None` (→ local fallback) when: the 404 latch is set;
capability negotiation says the server lacks `history` (which also sets the
latch); no server configured; a transient network error (does **not** latch);
any HTTP error; malformed JSON. A definitive 404 latches
`history_unsupported` so future reads skip the round-trip (issue #113).
Unparseable snapshots are skipped rather than mislabelled as deletes.

---

### fn entity_history

**Identification** — marker
`// md:impl DbBackend (server history) > fn entity_history`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Past versions newest-first: server journal first (a fresh
device sees every device's history, cross-device rollback works), local
`entity_changes` fallback (this device's own changes only). `limit = 0` →
`DEFAULT_HISTORY_LIMIT`. Local mapping: create/update → snapshot (unparseable
→ skip), delete → `entity: None`.

**Used by** — the `HistoryRepository` impl.

---

## impl HistoryRepository for DbBackend

**Identification** — marker `// md:impl HistoryRepository for DbBackend`;
per-method markers `> fn <name>`.

**Code** — container: members documented as sub-blocks below: fn note_history, fn notebook_history.

**What it does** — thin typed wrappers.

---

### fn note_history

**Identification** — marker
`// md:impl HistoryRepository for DbBackend > fn note_history`.

**Code** — complete and verbatim:

```rust
    // md:impl HistoryRepository for DbBackend > fn note_history
    async fn note_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Note>>, StorageError> {
        self.entity_history::<Note>("note", id, limit).await
    }
```

**What it does** — `entity_history::<Note>("note", …)`.

---

### fn notebook_history

**Identification** — marker
`// md:impl HistoryRepository for DbBackend > fn notebook_history`.

**Code** — complete and verbatim:

```rust
    // md:impl HistoryRepository for DbBackend > fn notebook_history
    async fn notebook_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Notebook>>, StorageError> {
        self.entity_history::<Notebook>("notebook", id, limit).await
    }
```

**What it does** — `entity_history::<Notebook>("notebook", …)`.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `DbBackend` — extended here with the blocks below (INFERRED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/db/mod.rs` — owns `DbBackend` and its fields (INFERRED)
- `keeplin-core/src/storage/db/convert.rs` — shared encoding helpers (INFERRED)
- `keeplin-core/src/storage/backend.rs` — the repository traits and shared types (INFERRED)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/storage/db/mod.rs` — declares this submodule (INFERRED)

**Invariants** (the rules this file must keep true — restated here even if stated
elsewhere)

- The split is a relocation: `storage::db::DbBackend` stays the public path, so no caller outside this directory module changes.

---

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | ServerVersion | `// md:ServerVersion` |
| 3 | CapabilityCache | `// md:CapabilityCache` |
| 4 | fn http_base_of | `// md:fn http_base_of` |
| 5 | impl DbBackend (server history) | `// md:impl DbBackend (server history)` |
| 6 | fn server_http_base | `// md:impl DbBackend (server history) > fn server_http_base` |
| 7 | fn server_has_capability | `// md:impl DbBackend (server history) > fn server_has_capability` |
| 8 | fn server_entity_history | `// md:impl DbBackend (server history) > fn server_entity_history` |
| 9 | fn entity_history | `// md:impl DbBackend (server history) > fn entity_history` |
| 10 | impl HistoryRepository for DbBackend | `// md:impl HistoryRepository for DbBackend` |
| 11 | fn note_history | `// md:impl HistoryRepository for DbBackend > fn note_history` |
| 12 | fn notebook_history | `// md:impl HistoryRepository for DbBackend > fn notebook_history` |
