# `tests/migrate.rs` — cross-backend migration tests

Self-contained companion for `keeplin-core/tests/migrate.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file
must be able to understand it without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Block header>`; grep it in either direction. Each section covers
**Identification**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the crate doc and the imports. Marker
`// md:Overview`.

```rust
use keeplin_core::{encryption::EncryptedBackend,
    links::{Bookmark, LinkSource, NoteLink}, migrate::migrate,
    models::{Note, NoteTag, Notebook, Resource, Tag},
    storage::{db::DbBackend, fs::FsBackend, StorageBackend}};
use tempfile::tempdir;
```

**What it does** — Integration tests for `keeplin_core::migrate::migrate` — the
one-shot copy of all live state from one backend into another, in both
directions and with encryption on both sides. Each test seeds a source backend
with one of every entity type, runs `migrate`, and asserts the destination holds
a faithful copy. The seed is written **directly** through the backend (no
`LinkingBackend`), so the tests check verbatim field fidelity rather than link
re-derivation.

**Repeated context** — migration is a one-shot copy of current live state, not
live sync: conflict resolution and tombstone propagation are covered by
`sync.rs`/`ws_sync.rs`/`db_backend.rs`, and Inbox/ordering placement rules by
the `ordering` unit tests — deliberately not here.

---

## Seeded

**Identification** — `struct Seeded`. Marker `// md:Seeded`.

**What it does** — The record of ids/values a seeded source produced
(`notebook_id`, `tag_id`, `note_a`, `note_b`, `resource_id`, `data`), handed to
`assert_migrated` to check the round-trip.

**Used by** — `seed` (returns), `assert_migrated` (consumes), all three tests.

---

## fn seed

**Identification** — `async fn seed(src: &dyn StorageBackend) -> Seeded`. Marker
`// md:fn seed`.

**What it does** — Populates `src` with: a notebook ("Work"), a tag ("urgent"),
note B ("Target" — created first so A can point at it), note A ("Source",
placed in the notebook, with `alias: "alpha"`, one pre-populated `Bookmark`
(`Anchor`/`Alias`) and one resolved `NoteLink` (`#target` → B)), a note↔tag
association, and a resource ("img", `image/png`) with non-UTF-8 binary bytes.
Navigation fields are pre-populated precisely so no `LinkingBackend` is needed —
which is exactly how `migrate` copies them.

**Used by** — all three tests.

---

## fn assert_migrated

**Identification** — `async fn assert_migrated(dst: &dyn StorageBackend, s:
&Seeded)`. Marker `// md:fn assert_migrated`.

**What it does** — Asserts the destination faithfully reproduces everything
`seed` wrote: notebook and tag titles; note A's title, notebook membership,
alias, bookmark fields, and resolved link; the surviving note↔tag association;
the resource metadata and its **exact** bytes; and that backlinks of B resolve
on the destination (built from the copied `links`).

**Used by** — all three tests.

---

## fn db

**Identification** — `async fn db(dir: &std::path::Path) -> DbBackend`. Marker
`// md:fn db`.

**What it does** — A fresh offline `DbBackend` at `{dir}/keeplin.db` (empty
server URL and token → no WebSocket, no handshake).

**Used by** — all three tests.

---

## fn fs_to_db_round_trip

**Identification** — `#[tokio::test]`. Marker `// md:fn fs_to_db_round_trip`.

**What it does** — Seed an `FsBackend`, migrate into a `DbBackend`: the
`MigrationReport` counts exactly 1 notebook, 1 tag, 2 notes, 1 note-tag,
1 resource, and `assert_migrated` passes.

---

## fn db_to_fs_round_trip

**Identification** — `#[tokio::test]`. Marker `// md:fn db_to_fs_round_trip`.

**What it does** — The reverse direction (`DbBackend` → `FsBackend`); same
fidelity (spot-checks notes/resources counts, then the full assertion).

---

## fn encrypted_fs_to_encrypted_db

**Identification** — `#[tokio::test]`. Marker
`// md:fn encrypted_fs_to_encrypted_db`.

**What it does** — Both sides wrapped in `EncryptedBackend` with **different**
passwords and salts: migration reads plaintext from the source and re-encrypts
under the destination's own key, so reads through the destination's encryption
decrypt correctly even though the ciphertexts differ.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `seed()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `assert_migrated()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `db()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `fs_to_db_round_trip()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `db_to_fs_round_trip()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `encrypted_fs_to_encrypted_db()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `Seeded` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/migrate.rs` — one-shot state copy between backends (EXTRACTED: calls×3; e.g. `migrate()`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: references×2; e.g. `StorageBackend`)
- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: references×1; e.g. `DbBackend`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Seeds one of every entity type and asserts a faithful copy in the destination — new entity kinds must be added to the seed when introduced.
- Covers crossing the encryption boundary in both directions.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | crate doc + imports | `// md:Overview` |
| 2 | `struct Seeded` | `// md:Seeded` |
| 3 | `fn seed` | `// md:fn seed` |
| 4 | `fn assert_migrated` | `// md:fn assert_migrated` |
| 5 | `fn db` | `// md:fn db` |
| 6–8 | the three `#[tokio::test]` fns | `// md:fn <name>` |
