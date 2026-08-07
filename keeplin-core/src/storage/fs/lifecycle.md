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

**Code** — container: members documented as sub-blocks below: fn new, fn sweep_orphan_tmp_files, fn scan_sync_conflicts, fn sweep_tmp_in_dir, fn format_version_path, fn has_store_content, fn ensure_format_version, fn read_or_create_device_id.

---

### fn new

**Identification** — `pub async fn new(root) -> Result<Self, StorageError>`;
marker `// md:impl FsBackend > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn new
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let root: PathBuf = root.into();
        let fresh = !root.join(".keeplin").join("format_version").exists()
            && !Self::has_store_content(&root).await?;

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

        let device_id = Self::read_or_create_device_id(&root).await?;
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
deleted), determines format freshness from the stamp and actual store content,
loads or creates the device id independently, and runs `ensure_format_version`.

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

### fn has_store_content

**Identification** — marker `// md:impl FsBackend > fn has_store_content`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn has_store_content
    pub(super) async fn has_store_content(root: &Path) -> Result<bool, StorageError> {
        for dir in [
            "notes",
            "resources",
            "logs",
            "notebooks",
            "tags",
            "note_tags",
            ".keeplin/offsets",
        ] {
            match tokio::fs::read_dir(root.join(dir)).await {
                Ok(mut entries) => {
                    if entries.next_entry().await?.is_some() {
                        return Ok(true);
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        }
        Ok(false)
    }
```

**What it does** — Reports content only when a known Keeplin data directory
contains an entry. It covers current note-scoped data, the legacy global
`resources` pool, logs, notebooks, tags, note-tag associations, and sync
offsets. Empty directory scaffolding does not count, including directories
created during backend initialization. Missing directories are empty; other
directory-read failures abort startup rather than risk classifying content as fresh.

**Dependencies** —

- `tokio::fs::read_dir` and `ReadDir::next_entry` — probe known data roots without modifying them; expects missing directories to be empty and other I/O failures to remain distinguishable and fatal.

**Used by** — `FsBackend::new`, before it creates the standard directory tree.

**Repeated context** — Format freshness is independent of device identity.

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
            let stamp = tokio::fs::read_to_string(&path).await?;
            stamp.trim().parse::<u32>().map_err(|_| {
                StorageError::InvalidState(format!(
                    "on-disk format stamp is unparsable; expected version {}. Retain the \
                     untouched store for manual recovery, start a new store, or restore a \
                     backup already in the expected format",
                    Self::FORMAT_VERSION
                ))
            })?
        } else {
            return Err(StorageError::InvalidState(format!(
                "on-disk format stamp is missing (implied version 1); expected version {}. \
                 Retain the untouched store for manual recovery, start a new store, or restore \
                 a backup already in the expected format",
                Self::FORMAT_VERSION
            )));
        };

        if current > Self::FORMAT_VERSION {
            return Err(StorageError::InvalidState(format!(
                "on-disk format version {current} is newer than this build supports \
                 (max {}); upgrade keeplin to open this store",
                Self::FORMAT_VERSION
            )));
        }

        if current < Self::FORMAT_VERSION {
            return Err(StorageError::InvalidState(format!(
                "on-disk format version {current} is older than the expected version {}. Retain \
                 the untouched store for manual recovery, start a new store, or restore a \
                 backup already in the expected format",
                Self::FORMAT_VERSION
            )));
        }

        Ok(())
    }
```

**What it does** — Stamps a genuinely fresh store directly at `FORMAT_VERSION`
(8). An existing store opens only when its stamp parses and equals the current
version. A missing, unparsable, or older stamp is refused without writing it;
the error identifies the stamp state, expected version, and the three recovery
choices required by ADR 0016. A stamp **newer** than this build retains the
existing downgrade refusal unchanged. No historical migration dispatcher
remains because versions 1 through 7 have no authorized data transformation.

---

### fn read_or_create_device_id

**Identification** — marker
`// md:impl FsBackend > fn read_or_create_device_id`.

**Code** — complete and verbatim:

```rust
    // md:impl FsBackend > fn read_or_create_device_id
    pub(super) async fn read_or_create_device_id(root: &Path) -> Result<String, StorageError> {
        let path = root.join(".keeplin").join("device_id");
        if path.exists() {
            let id = tokio::fs::read_to_string(&path).await?;
            Ok(id.trim().to_string())
        } else {
            let id = new_id().to_string();
            tokio::fs::write(&path, &id).await?;
            Ok(id)
        }
    }
```

**What it does** — Reads `.keeplin/device_id`, or generates and persists a UUID
v4. The id names this device's log file and is the Argon2id salt for
`EncryptedBackend`; it must stay stable. It does not classify the filesystem
format or decide whether the store is fresh.

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
- A format stamp always identifies an existing format; without one, only a store with no entries in known data directories is fresh.
- Existing filesystem formats below version 8 are refused without relabelling or migration.

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
| 8 | `fn has_store_content` | `// md:impl FsBackend > fn has_store_content` |
| 9 | `fn ensure_format_version` | `// md:impl FsBackend > fn ensure_format_version` |
| 10 | `fn read_or_create_device_id` | `// md:impl FsBackend > fn read_or_create_device_id` |
