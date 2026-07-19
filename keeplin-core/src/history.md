# `history.rs` — change history reads + forward-revert

Self-contained companion for `keeplin-core/src/history.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be able
to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{now, Note, Notebook},
    storage::{EntityVersion, StorageBackend},
};
```

**What it does** — Change-history reads and **forward-revert** helpers. History
itself is exposed by `crate::storage::HistoryRepository`
(`note_history`/`notebook_history`, newest first, `limit = 0` →
`DEFAULT_HISTORY_LIMIT`), derived from each backend's change journal — there is no
separate history store, because every journalled change carries a full entity
snapshot. This module adds the roll-back operations on top, as **free functions over
a type-erased `&dyn StorageBackend`** so the daemon's REST/gRPC surfaces call them
without naming the backend.

*Forward revert (non-destructive)*: reverting to a past state never deletes the
intervening versions. It writes the old state back as a **new** edit —
`update_note`/`update_notebook` mint a fresh version vector that dominates
everything seen so far — so the revert converges under sync exactly like any other
edit and can itself be undone by reverting again. A version that was a **tombstone**
at the target instant reverts to a delete.

*"As of" semantics*: every revert targets an **instant**, not an opaque version id —
the state as of `at` is the newest recorded version with `timestamp <= at`.
Point-in-time and batch rollback (a whole notebook back to just before a bad change)
fall out of the same primitive; reverting one version is just reverting to that
version's own timestamp.

*Where the versions come from* (per backend): `FsBackend` notes — the per-note
per-device op logs (up to the 256-entry compaction threshold); `FsBackend` notebooks
— the global NDJSON journal (best-effort, it compacts to current state);
`DbBackend` — the server's journal first (`GET /api/…/:id/history`, so a fresh
device sees cross-device history), falling back to the local `entity_changes` table
offline. `EncryptedBackend` decrypts each version on the way up, so history reads
return plaintext.

**Dependencies** — `chrono`, `uuid`, `crate::error::StorageError`,
`crate::models::{now, Note, Notebook}`, `crate::storage::{EntityVersion,
StorageBackend}`.

**Used by** — `keeplin-daemon/src/rest.rs` and `server.rs` (the history/revert
endpoints and RPCs).

**Repeated context** — Retention is governed by the journal's own count + age
bounds: FS compacts a note log past 256 entries; keeplin-srv's
`journal_retention_days` prunes by age (0/disabled = count-only).

---

## REVERT_SCAN_LIMIT

**Identification** — `const REVERT_SCAN_LIMIT: u32 = 10_000;` marker
`// md:REVERT_SCAN_LIMIT`.

**Code** — complete and verbatim:

```rust
// md:REVERT_SCAN_LIMIT
const REVERT_SCAN_LIMIT: u32 = 10_000;
```

**What it does** — How many versions a revert scans back through. Deliberately much
larger than `crate::storage::DEFAULT_HISTORY_LIMIT` (a *display* cap) so a revert
can still reach a version that predates the last hundred edits.

**Dependencies** — none.

**Used by** — `revert_note`, `revert_notebook`.

**Repeated context** — `limit = 0` on the history methods means "default display
cap", so revert must pass an explicit large limit instead.

---

## fn state_at

**Identification** —
`pub fn state_at<T>(versions: &[EntityVersion<T>], at: DateTime<Utc>) -> Option<&EntityVersion<T>>`;
marker `// md:fn state_at`.

**Code** — complete and verbatim:

```rust
// md:fn state_at
pub fn state_at<T>(versions: &[EntityVersion<T>], at: DateTime<Utc>) -> Option<&EntityVersion<T>> {
    versions.iter().find(|v| v.timestamp <= at)
}
```

**What it does** — Pure helper: the entity state as of `at`, i.e. the first entry
(newest-first order, as the `*_history` methods return) whose `timestamp <= at`.
Returns `None` when every recorded version is newer than `at` — the entity did not
exist yet at that instant.

**Dependencies** — `EntityVersion<T>` (generic over the entity type: `Note`,
`Notebook`, or a test's `u32`).

**Used by** — `revert_note`, `revert_notebook`; unit test
`state_at_picks_newest_at_or_before`.

**Repeated context** — `EntityVersion.entity` is `Option<T>`: `Some` = a live
snapshot, `None` = a tombstone version (soft-delete-always means deletes are
recorded as versions too).

---

## fn revert_note

**Identification** —
`pub async fn revert_note(backend: &dyn StorageBackend, id: Uuid, at: DateTime<Utc>) -> Result<Note, StorageError>`;
marker `// md:fn revert_note`.

**Code** — complete and verbatim:

```rust
// md:fn revert_note
pub async fn revert_note(
    backend: &dyn StorageBackend,
    id: Uuid,
    at: DateTime<Utc>,
) -> Result<Note, StorageError> {
    let versions = backend.note_history(id, REVERT_SCAN_LIMIT).await?;
    let target = state_at(&versions, at).ok_or_else(|| StorageError::NotFound(id.to_string()))?;
    match &target.entity {
        Some(note) => {
            let mut restored = note.clone();
            restored.updated_at = now();
            restored.deleted_at = None;
            backend.update_note(restored).await
        }
        None => {
            if let Err(e) = backend.delete_note(id).await {
                if !matches!(e, StorageError::NotFound(_)) {
                    return Err(e);
                }
            }
            backend.read_note(id).await
        }
    }
}
```

**What it does** — Forward-reverts a note to its state as of `at` and returns the
resulting note. Reads up to `REVERT_SCAN_LIMIT` versions, locates the target with
`state_at` (`NotFound(id)` when the note has no version at or before `at`), then:

- **Live target** — clone the snapshot, set `updated_at = now()` (the deterministic
  LWW tiebreak input; the backend recomputes the version vector), clear
  `deleted_at` (a revive is an ordinary live edit), and `update_note`.
- **Tombstone target** — forward-delete via `delete_note`, ignoring `NotFound`
  (an already-deleted note is the intended end state), then `read_note` to return
  the final state.

**Dependencies** — `state_at`, `REVERT_SCAN_LIMIT`, `StorageBackend`'s
`note_history`/`update_note`/`delete_note`/`read_note`, `models::now`.

**Used by** — `revert_notes_to`; the daemon's note-revert endpoint/RPC; tests
`revert_restores_an_earlier_version`, `revert_to_a_deleted_instant_deletes_the_note`.

**Repeated context** — revert is forward-only: it never rewrites or deletes
journal entries — the revert itself becomes version N+1.

---

## fn revert_notebook

**Identification** —
`pub async fn revert_notebook(backend: &dyn StorageBackend, id: Uuid, at: DateTime<Utc>) -> Result<Notebook, StorageError>`;
marker `// md:fn revert_notebook`.

**Code** — complete and verbatim:

```rust
// md:fn revert_notebook
pub async fn revert_notebook(
    backend: &dyn StorageBackend,
    id: Uuid,
    at: DateTime<Utc>,
) -> Result<Notebook, StorageError> {
    let versions = backend.notebook_history(id, REVERT_SCAN_LIMIT).await?;
    let target = state_at(&versions, at).ok_or_else(|| StorageError::NotFound(id.to_string()))?;
    match &target.entity {
        Some(notebook) => {
            let mut restored = notebook.clone();
            restored.updated_at = now();
            restored.deleted_at = None;
            backend.update_notebook(restored).await
        }
        None => {
            if let Err(e) = backend.delete_notebook(id).await {
                if !matches!(e, StorageError::NotFound(_)) {
                    return Err(e);
                }
            }
            backend.read_notebook(id).await
        }
    }
}
```

**What it does** — Notebook twin of `revert_note`, structurally identical:
`notebook_history` → `state_at` → live target re-written via `update_notebook`
(fresh `updated_at`, cleared `deleted_at`) or tombstone target forward-deleted via
`delete_notebook` (ignoring `NotFound`) + `read_notebook`.

**Dependencies** — as `revert_note`, with the notebook-side trait methods.

**Used by** — the daemon's notebook-revert endpoint/RPC.

**Repeated context** — notebook history on `FsBackend` is best-effort (the global
journal compacts), so deep notebook rollback is primarily a `DbBackend`/server-mode
feature.

---

## fn revert_notes_to

**Identification** —
`pub async fn revert_notes_to(backend: &dyn StorageBackend, ids: &[Uuid], at: DateTime<Utc>) -> Result<Vec<Note>, StorageError>`;
marker `// md:fn revert_notes_to`.

**Code** — complete and verbatim:

```rust
// md:fn revert_notes_to
pub async fn revert_notes_to(
    backend: &dyn StorageBackend,
    ids: &[Uuid],
    at: DateTime<Utc>,
) -> Result<Vec<Note>, StorageError> {
    let mut reverted = Vec::with_capacity(ids.len());
    for &id in ids {
        reverted.push(revert_note(backend, id, at).await?);
    }
    Ok(reverted)
}
```

**What it does** — Batch forward-revert: rolls every listed note back to its state
as of `at`, sequentially, returning results in input order. A failure aborts the
batch and returns the error, leaving the notes reverted so far in their new state —
safe, because each revert is an ordinary convergent edit, so a re-run simply
continues.

**Dependencies** — `revert_note`.

**Used by** — `revert_notebook_notes_to`; the daemon's batch-revert surface.

**Repeated context** — idempotence-by-convergence (re-running an interrupted batch
is safe) is the project's standard answer to partial failure, mirroring idempotent
`apply_change` in sync.

---

## fn revert_notebook_notes_to

**Identification** —
`pub async fn revert_notebook_notes_to(backend: &dyn StorageBackend, notebook_id: Uuid, at: DateTime<Utc>) -> Result<Vec<Note>, StorageError>`;
marker `// md:fn revert_notebook_notes_to`.

**Code** — complete and verbatim:

```rust
// md:fn revert_notebook_notes_to
pub async fn revert_notebook_notes_to(
    backend: &dyn StorageBackend,
    notebook_id: Uuid,
    at: DateTime<Utc>,
) -> Result<Vec<Note>, StorageError> {
    let mut ids = Vec::new();
    let mut token = None;
    loop {
        let (page, next) = backend
            .list_notes_in_notebook(notebook_id, 0, token)
            .await?;
        ids.extend(page.into_iter().map(|n| n.id));
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    revert_notes_to(backend, &ids, at).await
}
```

**What it does** — Batch-reverts every note **currently** in `notebook_id` to its
state as of `at` — the roll-back companion to a destructive notebook-wide change
(e.g. the capability-cascade delete). Exhausts `list_notes_in_notebook` with
`page_size = 0` (backend default, 100) following the cursor to collect ids, then
delegates to `revert_notes_to`. Notes that have since moved *out* of the notebook
are not touched; pass their ids to `revert_notes_to` directly.

**Dependencies** — `StorageBackend::list_notes_in_notebook`, `revert_notes_to`.

**Used by** — the daemon's notebook-rollback surface; test
`batch_revert_of_a_notebook_rolls_back_every_note`.

**Repeated context** — all list APIs are cursor-paginated; an absent `next_token`
ends the listing.

---

## mod tests

**Identification** — `#[cfg(test)]` unit/integration-test module; marker
`// md:mod tests`. Two helpers + four tests, running against a real `FsBackend` in
a tempdir (no network).

**Code** — container: members documented as sub-blocks below: fn ver, fn state_at_picks_newest_at_or_before, fn fs, fn note_history_lists_versions_newest_first, fn revert_restores_an_earlier_version, fn revert_to_a_deleted_instant_deletes_the_note, fn batch_revert_of_a_notebook_rolls_back_every_note.

**What it does** — Covers the pure `state_at` rule and the end-to-end revert
behaviours on a real backend.

**Dependencies** — `super::*`, `storage::fs::FsBackend`,
`storage::{HistoryRepository, NoteRepository}`, `chrono::TimeZone`, `tempfile`,
`tokio` (async tests; 2 ms sleeps guarantee distinct version timestamps).

**Used by** — CI (`cargo test --workspace`).

**Repeated context** — project test convention: logic that needs a backend uses
`FsBackend` in a tempdir as the cheapest real implementation.

### fn ver

**Identification** — test helper `fn ver(secs: i64, entity: Option<u32>) -> EntityVersion<u32>`;
marker `// md:mod tests > fn ver`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn ver
    fn ver(secs: i64, entity: Option<u32>) -> EntityVersion<u32> {
        EntityVersion {
            timestamp: Utc.timestamp_opt(secs, 0).unwrap(),
            device_id: "d".into(),
            entity,
        }
    }
```

**What it does** — Builds an `EntityVersion<u32>` at second `secs` with device id
`"d"`, for exercising `state_at` without real entities.

**Dependencies** — `EntityVersion`, `chrono::TimeZone`.

**Used by** — `state_at_picks_newest_at_or_before`.

**Repeated context** — none.

### fn state_at_picks_newest_at_or_before

**Identification** — unit test; marker
`// md:mod tests > fn state_at_picks_newest_at_or_before`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn state_at_picks_newest_at_or_before
    #[test]
    fn state_at_picks_newest_at_or_before() {
        let versions = vec![ver(30, Some(3)), ver(20, Some(2)), ver(10, Some(1))];
        assert_eq!(
            state_at(&versions, Utc.timestamp_opt(25, 0).unwrap())
                .unwrap()
                .entity,
            Some(2)
        );
        assert_eq!(
            state_at(&versions, Utc.timestamp_opt(30, 0).unwrap())
                .unwrap()
                .entity,
            Some(3)
        );
        assert!(state_at(&versions, Utc.timestamp_opt(5, 0).unwrap()).is_none());
    }
```

**What it does** — With newest-first versions at t=30/20/10: `at=25` picks the
t=20 version, `at=30` picks t=30 (inclusive boundary), `at=5` yields `None`
(nothing existed yet).

**Dependencies** — `ver`, `state_at`.

**Used by** — CI only.

**Repeated context** — none.

### fn fs

**Identification** — test helper `async fn fs() -> FsBackend`; marker
`// md:mod tests > fn fs`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn fs
    async fn fs() -> FsBackend {
        FsBackend::new(tempdir().unwrap().keep()).await.unwrap()
    }
```

**What it does** — Creates an `FsBackend` rooted in a fresh tempdir
(`tempdir().unwrap().keep()` — the dir outlives the guard for the test's duration).

**Dependencies** — `FsBackend`, `tempfile`.

**Used by** — the three async tests below.

**Repeated context** — none.

### fn note_history_lists_versions_newest_first

**Identification** — tokio test; marker
`// md:mod tests > fn note_history_lists_versions_newest_first`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn note_history_lists_versions_newest_first
    #[tokio::test]
    async fn note_history_lists_versions_newest_first() {
        let be = fs().await;
        let n = be.create_note(Note::new("t", "v1")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        let mut edited = n.clone();
        edited.body = "v2".into();
        be.update_note(edited).await.unwrap();

        let hist = be.note_history(n.id, 0).await.unwrap();
        assert_eq!(hist.len(), 2, "create + update = two versions");
        assert_eq!(hist[0].entity.as_ref().unwrap().body, "v2", "newest first");
        assert_eq!(hist[1].entity.as_ref().unwrap().body, "v1");
        assert!(hist[0].timestamp >= hist[1].timestamp);
    }
```

**What it does** — Create a note ("v1"), edit it ("v2"), and assert
`note_history` returns two versions, newest ("v2") first, with non-decreasing
timestamps down the list.

**Dependencies** — `fs`, `NoteRepository`, `HistoryRepository`.

**Used by** — CI only.

**Repeated context** — none.

### fn revert_restores_an_earlier_version

**Identification** — tokio test; marker
`// md:mod tests > fn revert_restores_an_earlier_version`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn revert_restores_an_earlier_version
    #[tokio::test]
    async fn revert_restores_an_earlier_version() {
        let be = fs().await;
        let n = be.create_note(Note::new("t", "v1")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        let mut edited = n.clone();
        edited.body = "v2".into();
        be.update_note(edited).await.unwrap();

        let hist = be.note_history(n.id, 0).await.unwrap();
        let reverted = revert_note(&be, n.id, hist[1].timestamp).await.unwrap();
        assert_eq!(reverted.body, "v1", "revert re-applied the old body");
        assert_eq!(be.read_note(n.id).await.unwrap().body, "v1");
        assert_eq!(be.note_history(n.id, 0).await.unwrap().len(), 3);
    }
```

**What it does** — Create "v1", edit to "v2", revert to the first version's own
timestamp; assert the returned and re-read body is "v1" and — non-destructive —
history now has **three** versions (the revert stacked on top).

**Dependencies** — `fs`, `revert_note`.

**Used by** — CI only.

**Repeated context** — none.

### fn revert_to_a_deleted_instant_deletes_the_note

**Identification** — tokio test; marker
`// md:mod tests > fn revert_to_a_deleted_instant_deletes_the_note`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn revert_to_a_deleted_instant_deletes_the_note
    #[tokio::test]
    async fn revert_to_a_deleted_instant_deletes_the_note() {
        let be = fs().await;
        let n = be.create_note(Note::new("t", "v1")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        be.delete_note(n.id).await.unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        let mut revived = n.clone();
        revived.body = "back".into();
        be.update_note(revived).await.unwrap();
        assert!(be.read_note(n.id).await.unwrap().deleted_at.is_none());

        let hist = be.note_history(n.id, 0).await.unwrap();
        let tomb = hist
            .iter()
            .find(|v| v.entity.is_none())
            .expect("a tombstone version exists");
        let reverted = revert_note(&be, n.id, tomb.timestamp).await.unwrap();
        assert!(
            reverted.deleted_at.is_some(),
            "reverting to a deleted instant deletes the note"
        );
    }
```

**What it does** — Create, delete, then revive a note with a fresh edit (currently
live); find the tombstone version (`entity.is_none()`) mid-history and revert to
its instant; assert the note ends deleted (`deleted_at.is_some()`).

**Dependencies** — `fs`, `revert_note`.

**Used by** — CI only.

**Repeated context** — none.

### fn batch_revert_of_a_notebook_rolls_back_every_note

**Identification** — tokio test; marker
`// md:mod tests > fn batch_revert_of_a_notebook_rolls_back_every_note`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn batch_revert_of_a_notebook_rolls_back_every_note
    #[tokio::test]
    async fn batch_revert_of_a_notebook_rolls_back_every_note() {
        let be = fs().await;
        let nb = Uuid::from_u128(0xB007);
        let mut a = Note::new("a", "a1");
        a.notebook_id = nb;
        let a = be.create_note(a).await.unwrap();
        let mut b = Note::new("b", "b1");
        b.notebook_id = nb;
        let b = be.create_note(b).await.unwrap();
        let cutoff = now();
        tokio::time::sleep(Duration::from_millis(2)).await;

        for (n, body) in [(&a, "a2"), (&b, "b2")] {
            let mut e = n.clone();
            e.body = body.into();
            be.update_note(e).await.unwrap();
        }

        let reverted = revert_notebook_notes_to(&be, nb, cutoff).await.unwrap();
        assert_eq!(reverted.len(), 2);
        assert_eq!(be.read_note(a.id).await.unwrap().body, "a1");
        assert_eq!(be.read_note(b.id).await.unwrap().body, "b1");
    }
```

**What it does** — Two notes in one notebook at "a1"/"b1", capture `cutoff`, edit
both to "a2"/"b2", call `revert_notebook_notes_to(nb, cutoff)`; assert both
returned and both re-read bodies are back at "a1"/"b1".

**Dependencies** — `fs`, `revert_notebook_notes_to`.

**Used by** — CI only.

**Repeated context** — none.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `revert_note()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `revert_notebook()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `revert_notes_to()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `revert_notebook_notes_to()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `state_at()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `ver()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `fs()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `state_at_picks_newest_at_or_before()` — defined here (EXTRACTED; file-local)
- `note_history_lists_versions_newest_first()` — defined here (EXTRACTED; file-local)
- `revert_restores_an_earlier_version()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs` — error types (EXTRACTED: references×4; e.g. `StorageError`)
- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: references×4; e.g. `Note`, `Notebook`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: references×7; e.g. `EntityVersion`, `T`, `StorageBackend`)
- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: imports_from×1, references×1; e.g. `FsBackend`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph — the daemon calls these via fully-qualified `keeplin_core::history::…` paths the AST pass does not link; see `keeplin-daemon/src/rest.rs` and `server.rs`) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `const REVERT_SCAN_LIMIT` | `// md:REVERT_SCAN_LIMIT` |
| 3 | `fn state_at` | `// md:fn state_at` |
| 4 | `fn revert_note` | `// md:fn revert_note` |
| 5 | `fn revert_notebook` | `// md:fn revert_notebook` |
| 6 | `fn revert_notes_to` | `// md:fn revert_notes_to` |
| 7 | `fn revert_notebook_notes_to` | `// md:fn revert_notebook_notes_to` |
| 8 | `mod tests` | `// md:mod tests` |
| 9 | `fn ver` | `// md:mod tests > fn ver` |
| 10 | `fn state_at_picks_newest_at_or_before` | `// md:mod tests > fn state_at_picks_newest_at_or_before` |
| 11 | `fn fs` | `// md:mod tests > fn fs` |
| 12 | `fn note_history_lists_versions_newest_first` | `// md:mod tests > fn note_history_lists_versions_newest_first` |
| 13 | `fn revert_restores_an_earlier_version` | `// md:mod tests > fn revert_restores_an_earlier_version` |
| 14 | `fn revert_to_a_deleted_instant_deletes_the_note` | `// md:mod tests > fn revert_to_a_deleted_instant_deletes_the_note` |
| 15 | `fn batch_revert_of_a_notebook_rolls_back_every_note` | `// md:mod tests > fn batch_revert_of_a_notebook_rolls_back_every_note` |
