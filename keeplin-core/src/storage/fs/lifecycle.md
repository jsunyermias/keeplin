# `storage/fs/lifecycle.rs` — startup hygiene and on-disk format lifecycle

Self-contained companion for `keeplin-core/src/storage/fs/lifecycle.rs`. It documents every source block in source order with complete code embedded for every leaf.

---

## Overview

**Identification** — file-level module declarations and imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::error::StorageError;
use crate::models::new_id;

use super::FsBackend;
```

**What it does** — Owns startup hygiene and on-disk format lifecycle. This is a structural relocation from the former monolithic filesystem module; storage behavior, on-disk format version 8, serialization shapes, conflict resolution, and public API are unchanged.

**Dependencies** — every binding above is used below or exposes a sibling item as `pub(super)`; expects: pre-split signatures and behavior.

**Used by** — sibling `storage/fs/` modules and existing `FsBackend` callers.

**Repeated context** — `FsBackend::FORMAT_VERSION` remains 8.

---

## impl FsBackend

**Identification** — the first inherent impl; marker `// md:impl FsBackend`.
Constructor, sweeps, format versioning, path helpers, log machinery,
sidecar/association/resource versioning, the note merge pipeline. Contains the
constants `FORMAT_VERSION = 8`, `NOTE_LOG_COMPACT_THRESHOLD = 256`,
`GLOBAL_LOG_COMPACT_THRESHOLD = 512`, `GLOBAL_LOG_SOFT_BYTES = 64 KiB`
(documented with the methods that use them).

**Code** — container: members documented as sub-blocks below: fn new, fn sweep_orphan_tmp_files, fn scan_sync_conflicts, fn sweep_tmp_in_dir, fn format_version_path, fn ensure_format_version, fn apply_format_migration, fn read_or_create_device_id.

---

### fn new

**Identification** — `pub async fn new(root) -> Result<Self, StorageError>`;
marker `// md:impl FsBackend > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn new
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let root: PathBuf = root.into();

        for dir in &[
            "notes",
            ".keeplin",
            ".keeplin/offsets",
            "logs",
            "notebooks",
            "tags",
            "note_tags",
        ] {
            tokio::fs::create_dir_all(root.join(dir)).await?;
        }

        let removed = Self::sweep_orphan_tmp_files(&root).await;
        if removed > 0 {
            tracing::info!(
                removed,
                "Removed orphaned .tmp files left by interrupted writes"
            );
        }

        let conflicts = Self::scan_sync_conflicts(&root).await;
        if !conflicts.is_empty() {
            for path in &conflicts {
                tracing::error!(path = %path.display(), "Syncthing conflict copy detected");
            }
            tracing::error!(
                count = conflicts.len(),
                "Syncthing '*.sync-conflict-*' files exist in this store. Every keeplin \
                 file has a single writer, so conflict copies mean two devices are \
                 fighting over the same files — almost always because `.keeplin/` (this \
                 device's identity) was replicated instead of excluded via .stignore. \
                 Fix the Syncthing ignore rules (see README, 'Multi-device setup with \
                 Syncthing'), then reconcile each conflict copy manually before trusting \
                 further writes."
            );
        }

        let (device_id, fresh) = Self::read_or_create_device_id(&root).await?;
        let backend = Self {
            root,
            device_id,
            note_write_lock: Arc::new(Mutex::new(())),
            global_log_lock: Arc::new(Mutex::new(())),
            note_index: Arc::new(RwLock::new(None)),
        };
        backend.ensure_format_version(fresh).await?;
        Ok(backend)
    }
```

**What it does** — Creates the directory tree, sweeps orphaned `*.tmp` files,
scans for Syncthing `*.sync-conflict-*` copies (reported at **error** level —
in a single-writer-per-file store they are the signature of a replicated
`.keeplin/` directory, i.e. two devices sharing one identity; nothing is
deleted), loads or creates the device id, and runs `ensure_format_version`
(`fresh` = the id was just created).

**Used by** — `main.rs::build_storage` (default mode); tests everywhere.

---

### fn sweep_orphan_tmp_files

**Identification** — marker `// md:impl FsBackend > fn sweep_orphan_tmp_files`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn sweep_orphan_tmp_files
    pub(super) async fn sweep_orphan_tmp_files(root: &Path) -> usize {
        let mut removed = 0usize;
        for flat in ["notebooks", "tags", "logs", ".keeplin", ".keeplin/offsets"] {
            removed += Self::sweep_tmp_in_dir(&root.join(flat)).await;
        }
        if let Ok(mut rd) = tokio::fs::read_dir(root.join("notes")).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    removed += Self::sweep_tmp_in_dir(&entry.path()).await;
                    removed += Self::sweep_tmp_in_dir(&entry.path().join("resources")).await;
                }
            }
        }
        if let Ok(mut rd) = tokio::fs::read_dir(root.join("note_tags")).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    removed += Self::sweep_tmp_in_dir(&entry.path()).await;
                }
            }
        }
        removed
    }
```

**What it does** — Best-effort startup removal of `*.tmp` files orphaned by
interrupted atomic writes: the flat dirs, one level down inside `note_tags/`,
and — for `notes/` — both each note directory and its `resources/` subdirectory,
where attachment blobs and meta sidecars now live (issue #127). Syncthing's own
`.syncthing.*.tmp` in-flight temporaries are explicitly left alone. Errors
ignored — hygiene, never a startup blocker.

---

### fn scan_sync_conflicts

**Identification** — marker `// md:impl FsBackend > fn scan_sync_conflicts`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn scan_sync_conflicts
    pub(super) async fn scan_sync_conflicts(root: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut dirs: Vec<PathBuf> = [
            "",
            "notebooks",
            "tags",
            "logs",
            ".keeplin",
            ".keeplin/offsets",
        ]
        .iter()
        .map(|d| root.join(d))
        .collect();
        if let Ok(mut rd) = tokio::fs::read_dir(root.join("notes")).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    dirs.push(entry.path());
                    dirs.push(entry.path().join("resources"));
                }
            }
        }
        if let Ok(mut rd) = tokio::fs::read_dir(root.join("note_tags")).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    dirs.push(entry.path());
                }
            }
        }
        for dir in dirs {
            let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
                continue;
            };
            while let Ok(Some(entry)) = rd.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.contains(".sync-conflict-")
                    && entry
                        .file_type()
                        .await
                        .map(|t| t.is_file())
                        .unwrap_or(false)
                {
                    found.push(entry.path());
                }
            }
        }
        found
    }
```

**What it does** — Read-only collection of every `*.sync-conflict-*` file in
the managed directories (and root). Nothing is deleted — the copies may hold
the only good version; the caller logs the findings with remediation guidance
(fix `.stignore`, reconcile manually).

---

### fn sweep_tmp_in_dir

**Identification** — marker `// md:impl FsBackend > fn sweep_tmp_in_dir`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn sweep_tmp_in_dir
    pub(super) async fn sweep_tmp_in_dir(dir: &Path) -> usize {
        let mut removed = 0usize;
        let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
            return 0;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".tmp") || name.starts_with(".syncthing.") {
                continue;
            }
            if !entry
                .file_type()
                .await
                .map(|t| t.is_file())
                .unwrap_or(false)
            {
                continue;
            }
            if tokio::fs::remove_file(entry.path()).await.is_ok() {
                tracing::debug!(path = %entry.path().display(), "Removed orphaned temp file");
                removed += 1;
            }
        }
        removed
    }

    pub(super) const FORMAT_VERSION: u32 = 8;

    pub(super) const NOTE_LOG_COMPACT_THRESHOLD: usize = 256;

    pub(super) const GLOBAL_LOG_COMPACT_THRESHOLD: usize = 512;

    pub(super) const GLOBAL_LOG_SOFT_BYTES: u64 = 64 * 1024;
```

**What it does** — Non-recursive removal of orphaned `*.tmp` regular files in
one directory, skipping Syncthing temporaries.

---

### fn format_version_path

**Identification** — marker `// md:impl FsBackend > fn format_version_path`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn format_version_path
    pub(super) fn format_version_path(&self) -> PathBuf {
        self.root.join(".keeplin").join("format_version")
    }
```

**What it does** — `.keeplin/format_version`.

---

### fn ensure_format_version

**Identification** — marker `// md:impl FsBackend > fn ensure_format_version`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn ensure_format_version
    pub(super) async fn ensure_format_version(&self, fresh: bool) -> Result<(), StorageError> {
        let path = self.format_version_path();

        if fresh {
            tokio::fs::write(&path, Self::FORMAT_VERSION.to_string()).await?;
            return Ok(());
        }

        let current = if path.exists() {
            tokio::fs::read_to_string(&path)
                .await?
                .trim()
                .parse::<u32>()
                .unwrap_or(1)
        } else {
            1
        };

        if current > Self::FORMAT_VERSION {
            return Err(StorageError::InvalidState(format!(
                "on-disk format version {current} is newer than this build supports \
                 (max {}); upgrade keeplin to open this store",
                Self::FORMAT_VERSION
            )));
        }

        for version in (current + 1)..=Self::FORMAT_VERSION {
            self.apply_format_migration(version).await?;
            tokio::fs::write(&path, version.to_string()).await?;
            tracing::info!(version, "Applied filesystem format migration");
        }

        tokio::fs::write(&path, Self::FORMAT_VERSION.to_string()).await?;
        Ok(())
    }
```

**What it does** — Brings the store up to `FORMAT_VERSION` (8), stamping after
**each** step so a crash mid-ladder resumes from the last completed step. A
`fresh` store is stamped directly (no migration over empty data). A missing
stamp on an existing store means format `1`; a stamp **newer** than this build
is refused (`InvalidState`) so a downgrade cannot run against a layout it does
not understand. A final stamp write covers the already-current case.

---

### fn apply_format_migration

**Identification** — marker
`// md:impl FsBackend > fn apply_format_migration`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn apply_format_migration
    pub(super) async fn apply_format_migration(&self, version: u32) -> Result<(), StorageError> {
        match version {
            2..=8 => Ok(()),
            other => Err(StorageError::InvalidState(format!(
                "no filesystem migration defined for format version {other}"
            ))),
        }
    }
```

**What it does** — The per-version step. Every bump so far is a clean break with
no data transform, so v2–v8 are no-ops that only advance the stamp: v2 =
`LogEntry` serde aliases; v3/v4 = versioned associations + resource tombstones via
`serde(default)`; v5 = optional `EpochHeader` + `epoch:offset` cursors (a
pre-v5 log is epoch 0, a bare-integer cursor is `(0, offset)`); v8 = attachments
moved from the global `resources/{uuid}/` pool into
`notes/{note_id}/resources/{hash}.knrs` (issue #127) — the old pool is simply no
longer read, so no code migrates it. A future breaking change gets a real body
here.

---

### fn read_or_create_device_id

**Identification** — marker
`// md:impl FsBackend > fn read_or_create_device_id`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_or_create_device_id
    pub(super) async fn read_or_create_device_id(
        root: &Path,
    ) -> Result<(String, bool), StorageError> {
        let path = root.join(".keeplin").join("device_id");
        if path.exists() {
            let id = tokio::fs::read_to_string(&path).await?;
            Ok((id.trim().to_string(), false))
        } else {
            let id = new_id().to_string();
            tokio::fs::write(&path, &id).await?;
            Ok((id, true))
        }
    }
```

**What it does** — Reads `.keeplin/device_id`, or generates + persists a UUID
v4. Returns `(id, fresh)` — the file is the first thing written on init, so
its absence reliably means "never initialised" (used to stamp new stores at
the current format). The id names this device's log file and is the Argon2id
salt for `EncryptedBackend`; it must stay stable.

---

## Graph context

Repo-tooling metadata, not a code block.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `startup hygiene and on-disk format lifecycle` — defined here (INFERRED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/fs/mod.rs` and sibling `storage/fs/` modules — shared backend state and helpers (INFERRED)
- `keeplin-core/src/storage/backend.rs`, `models.rs`, `error.rs`, and `storage/note_log.rs` — unchanged storage contracts (INFERRED)

**Direct dependents** (files whose symbols reference this one)

- sibling `storage/fs/` modules and existing callers (INFERRED)

**Invariants** (the rules this file must keep true)

- This split does not change the on-disk format; `FsBackend::FORMAT_VERSION` remains 8.
- The public backend path remains `crate::storage::fs::FsBackend`.
- Filesystem behavior remains unchanged.

---

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|

| 1 | `Overview` | `// md:Overview` |
| 2 | `impl FsBackend` (container) | `// md:impl FsBackend` |
| 3 | `fn new` | `// md:impl FsBackend > fn new` |
| 4 | `fn sweep_orphan_tmp_files` | `// md:impl FsBackend > fn sweep_orphan_tmp_files` |
| 5 | `fn scan_sync_conflicts` | `// md:impl FsBackend > fn scan_sync_conflicts` |
| 6 | `fn sweep_tmp_in_dir` | `// md:impl FsBackend > fn sweep_tmp_in_dir` |
| 7 | `fn format_version_path` | `// md:impl FsBackend > fn format_version_path` |
| 8 | `fn ensure_format_version` | `// md:impl FsBackend > fn ensure_format_version` |
| 9 | `fn apply_format_migration` | `// md:impl FsBackend > fn apply_format_migration` |
| 10 | `fn read_or_create_device_id` | `// md:impl FsBackend > fn read_or_create_device_id` |
