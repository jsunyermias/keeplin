# `migrate.rs` — one-shot state copy between backends

Self-contained companion for `keeplin-core/src/migrate.rs`. It documents **every code block of
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
use crate::{
    error::StorageError,
    models::{NoteTag, Resource},
    storage::StorageBackend,
};
```

**What it does** — One-shot state migration between any two `StorageBackend`s, in
either direction (`FsBackend ↔ DbBackend`), including across an encryption boundary.
Why a dedicated path instead of a raw `get_changes_since → apply_change` bridge: the
two backends have **asymmetric** sync channels, so their `Change` streams are not
interchangeable —

- `FsBackend::get_changes_since` reads only the global NDJSON journal
  (notebooks/tags/resources); notes live in per-note version-vector logs and are not
  emitted there.
- `FsBackend::apply_change` for a note ignores the payload and only re-materialises
  logs already on disk (the Syncthing assumption), so importing an unseen note is a
  silent no-op.
- `EncryptedBackend` passes `apply_change` through **without encrypting**, so it
  can't be a raw-change destination either.

`migrate` sidesteps all three by copying the **current live state** through the typed
`create_*` methods, which every layer implements correctly: verbatim
ids/timestamps/`alias`/`bookmarks`/`links`, a native per-device VV log on FS, the
rebuilt `note_links` backlink index on DB, decrypt-on-read/encrypt-on-write when a
side is wrapped in `EncryptedBackend` (each side uses its own key). Deliberate
limitations: live state only (no tombstones — `list_*` excludes soft-deleted rows; a
migration is a fresh start), empty destination required (original ids are inserted;
an existing id errors, e.g. `DbBackend::create_note` is a plain `INSERT`), and it is
a one-shot copy, not live sync.

**Dependencies** — `crate::error::StorageError`, `crate::models::{NoteTag,
Resource}`, `crate::storage::StorageBackend`.

**Used by** — the daemon's `keeplin-daemon migrate --from <a.toml> --to <b.toml>`
subcommand (`keeplin-daemon/src/main.rs`, which builds each side from its own
config); `keeplin-core/tests/migrate.rs`.

**Repeated context** — Soft-delete-always is a project premise everywhere *except*
here by design: a migration intentionally leaves tombstones behind. Dyn-trait
(`&dyn StorageBackend`) parameters are the project norm for backend-agnostic code.

---

## PAGE

**Identification** — `const PAGE: u32 = 500;` marker `// md:PAGE`.

**Code** — complete and verbatim:

```rust
// md:PAGE
const PAGE: u32 = 500;
```

**What it does** — How many entities to request per page while exhausting the
paginated `list_*` methods. Any value in `1..=MAX_PAGE_SIZE` (1000, from
`storage/mod.rs`) is correct; 500 balances round-trips against per-page memory.

**Dependencies** — none.

**Used by** — every `collect(…)` call in `migrate`.

**Repeated context** — all list APIs in the project are cursor-paginated; passing
`0` would mean "backend default" (100).

---

## MigrationReport

**Identification** — struct deriving `Debug, Default, Clone, Copy, PartialEq, Eq`;
marker `// md:MigrationReport`.

**Code** — complete and verbatim:

```rust
// md:MigrationReport
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MigrationReport {
    pub notebooks: usize,
    pub tags: usize,
    pub notes: usize,
    pub note_tags: usize,
    pub resources: usize,
}
```

**What it does** — Per-entity counts of what a `migrate` run copied, for reporting
to the operator: `notebooks`, `tags`, `notes`, `note_tags` (note↔tag associations),
`resources` (metadata + binary payload). Plain data, `Default` starts at zero.

**Dependencies** — none.

**Used by** — returned by `migrate`; printed by the daemon's `migrate` subcommand;
asserted on in `keeplin-core/tests/migrate.rs`.

**Repeated context** — none.

---

## fn migrate

**Identification** —
`pub async fn migrate(src: &dyn StorageBackend, dst: &dyn StorageBackend) -> Result<MigrationReport, StorageError>`;
marker `// md:fn migrate`.

**Code** — complete and verbatim:

```rust
// md:fn migrate
pub async fn migrate(
    src: &dyn StorageBackend,
    dst: &dyn StorageBackend,
) -> Result<MigrationReport, StorageError> {
    let mut report = MigrationReport::default();

    for notebook in collect(|token| src.list_notebooks(PAGE, token)).await? {
        dst.create_notebook(notebook).await?;
        report.notebooks += 1;
    }

    for tag in collect(|token| src.list_tags(PAGE, token)).await? {
        dst.create_tag(tag).await?;
        report.tags += 1;
    }

    let notes = collect(|token| src.list_notes(PAGE, token)).await?;
    for note in &notes {
        dst.create_note(note.clone()).await?;
        report.notes += 1;
    }
    for note in &notes {
        for tag in collect(|token| src.list_note_tags(note.id, PAGE, token)).await? {
            dst.add_note_tag(NoteTag {
                note_id: note.id,
                tag_id: tag.id,
            })
            .await?;
            report.note_tags += 1;
        }
    }

    for meta in collect(|token| src.list_resources(PAGE, token)).await? {
        let (resource, data): (Resource, Vec<u8>) = src.read_resource(meta.id).await?;
        dst.create_resource(resource, data).await?;
        report.resources += 1;
    }

    Ok(report)
}
```

**What it does** — Copies every live entity from `src` into `dst`. Order matters so
references resolve as entities land:

| Order | Entity | How |
|-------|--------|-----|
| 1 | notebooks | `list_notebooks` → `dst.create_notebook` (before notes: a note carries a `notebook_id`) |
| 2 | tags | `list_tags` → `dst.create_tag` (before note↔tag associations) |
| 3 | notes | `list_notes` → `dst.create_note(note.clone())` — `alias`/`bookmarks`/`links` ride along as note fields; the destination rebuilds any backlink index from them |
| 4 | note↔tag | per note, `src.list_note_tags` → `dst.add_note_tag(NoteTag { note_id, tag_id })` |
| 5 | resources | `list_resources` (metadata) + `src.read_resource` (bytes) → `dst.create_resource` |

Each write uses the same typed call the API surfaces use, so the destination stores
the entity exactly as if a client had created it — including its own indexes and its
own at-rest encryption. Returns the `MigrationReport` counts. **Fails fast** on the
first error, leaving whatever was already written in place (hence the
empty-destination expectation: a re-run after fixing the cause starts fresh).

**Dependencies** — `collect` (below), `PAGE`, `StorageBackend`'s
`list_*`/`create_*`/`add_note_tag`/`read_resource`, `NoteTag`, `Resource`.

**Used by** — `keeplin-daemon/src/main.rs` (the `migrate` subcommand);
`keeplin-core/tests/migrate.rs` (`fs_to_db_round_trip`, `db_to_fs_round_trip`,
`encrypted_fs_to_encrypted_db`).

**Repeated context** — Notes are the only entity with per-note VV logs on FS;
`create_note` on `FsBackend` writes a proper per-device log so the note enters the
filesystem model natively — this is the property that makes the typed-copy approach
correct where the raw-change bridge is not.

---

## fn collect

**Identification** — `async fn collect<T, F, Fut>(mut page: F) -> Result<Vec<T>, StorageError>`
where `F: FnMut(Option<String>) -> Fut`,
`Fut: Future<Output = Result<(Vec<T>, Option<String>), StorageError>>`; marker
`// md:fn collect`.

**Code** — complete and verbatim:

```rust
// md:fn collect
async fn collect<T, F, Fut>(mut page: F) -> Result<Vec<T>, StorageError>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<T>, Option<String>), StorageError>>,
{
    let mut out = Vec::new();
    let mut token = None;
    loop {
        let (items, next) = page(token).await?;
        out.extend(items);
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    Ok(out)
}
```

**What it does** — Exhausts a paginated `list_*` call into a single `Vec`: starts
with `token = None`, calls `page(token)` in a loop, extends the output with each
page's items, follows `next_token` until it is `None`. `page` is any closure
matching every `list_*` method's `(page_size-bound) Option<token> → (items,
next_token)` shape, so one helper drives notebooks, tags, notes, note-tags, and
resources alike. Propagates the first `StorageError`.

**Dependencies** — only the closure it is given.

**Used by** — `migrate` (its only caller; the function is file-private).

**Repeated context** — an empty `next_token` (`None`) is the universal
end-of-listing signal in this project's pagination contract.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `migrate()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `collect()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `MigrationReport` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs` — error types (EXTRACTED: references×2; e.g. `StorageError`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: references×2; e.g. `StorageBackend`, `T`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/tests/migrate.rs` — cross-backend migration tests (EXTRACTED: calls×3; e.g. `db_to_fs_round_trip()`, `encrypted_fs_to_encrypted_db()`, `fs_to_db_round_trip()`)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `const PAGE` | `// md:PAGE` |
| 3 | `struct MigrationReport` | `// md:MigrationReport` |
| 4 | `fn migrate` | `// md:fn migrate` |
| 5 | `fn collect` | `// md:fn collect` |
