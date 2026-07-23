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

**Code** — complete and verbatim:

```rust
// md:Overview

use keeplin_core::{
    encryption::EncryptedBackend,
    links::{Bookmark, LinkSource, NoteLink},
    migrate::migrate,
    models::{Note, NoteTag, Notebook, Resource, Tag, SYSTEM_RESOURCE_NOTE_ID},
    storage::{db::DbBackend, fs::FsBackend, StorageBackend},
};
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

**Code** — complete and verbatim:

```rust
// md:Seeded
struct Seeded {
    notebook_id: uuid::Uuid,
    tag_id: uuid::Uuid,
    note_a: uuid::Uuid,
    note_b: uuid::Uuid,
    resource_id: uuid::Uuid,
    data: Vec<u8>,
}
```

**What it does** — The record of ids/values a seeded source produced
(`notebook_id`, `tag_id`, `note_a`, `note_b`, `resource_id`, `data`), handed to
`assert_migrated` to check the round-trip.

**Used by** — `seed` (returns), `assert_migrated` (consumes), all three tests.

---

## fn seed

**Identification** — `async fn seed(src: &dyn StorageBackend) -> Seeded`. Marker
`// md:fn seed`.

**Code** — complete and verbatim:

```rust
// md:fn seed
async fn seed(src: &dyn StorageBackend) -> Seeded {
    let notebook = Notebook::new("Work");
    let notebook_id = notebook.id;
    src.create_notebook(notebook).await.unwrap();

    let tag = Tag::new("urgent");
    let tag_id = tag.id;
    src.create_tag(tag).await.unwrap();

    let note_b = Note::new("Target", "the destination note");
    let note_b_id = note_b.id;
    src.create_note(note_b).await.unwrap();

    let mut note_a = Note::new(
        "Source",
        "intro [Anchor](### \"Alias\") and a [link](#target)",
    );
    let note_a_id = note_a.id;
    note_a.notebook_id = notebook_id;
    note_a.alias = Some("alpha".to_string());
    note_a.bookmarks = vec![Bookmark {
        number: 1,
        text: "Anchor".to_string(),
        alias: "Alias".to_string(),
    }];
    note_a.links = vec![NoteLink {
        source: LinkSource::Content,
        raw: "#target".to_string(),
        target_note_id: Some(note_b_id),
    }];
    src.create_note(note_a.clone()).await.unwrap();

    src.add_note_tag(NoteTag {
        note_id: note_a_id,
        tag_id,
    })
    .await
    .unwrap();

    let data = b"\x00\x01\x02binary-payload\xff".to_vec();
    let resource = Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        "img",
        "image/png",
        "img.png",
        data.len() as u64,
    );
    let resource_id = resource.id;
    src.create_resource(resource, data.clone()).await.unwrap();

    Seeded {
        notebook_id,
        tag_id,
        note_a: note_a_id,
        note_b: note_b_id,
        resource_id,
        data,
    }
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn assert_migrated
async fn assert_migrated(dst: &dyn StorageBackend, s: &Seeded) {
    let nb = dst.read_notebook(s.notebook_id).await.unwrap();
    assert_eq!(nb.title, "Work");

    let tag = dst.read_tag(s.tag_id).await.unwrap();
    assert_eq!(tag.title, "urgent");

    let a = dst.read_note(s.note_a).await.unwrap();
    assert_eq!(a.title, "Source");
    assert_eq!(a.notebook_id, s.notebook_id);
    assert_eq!(a.alias.as_deref(), Some("alpha"));
    assert_eq!(a.bookmarks.len(), 1);
    assert_eq!(a.bookmarks[0].text, "Anchor");
    assert_eq!(a.bookmarks[0].alias, "Alias");
    assert_eq!(a.links.len(), 1);
    assert_eq!(a.links[0].raw, "#target");
    assert_eq!(a.links[0].target_note_id, Some(s.note_b));

    let (tags, _) = dst.list_note_tags(s.note_a, 0, None).await.unwrap();
    assert!(tags.iter().any(|t| t.id == s.tag_id));

    let (meta, bytes) = dst.read_resource(s.resource_id).await.unwrap();
    assert_eq!(meta.file_name, "img.png");
    assert_eq!(bytes, s.data);

    let (back, _) = dst.note_backlinks(s.note_b, 0, None).await.unwrap();
    assert!(back.iter().any(|n| n.id == s.note_a));
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn db
async fn db(dir: &std::path::Path) -> DbBackend {
    DbBackend::new(dir.join("keeplin.db"), "", "")
        .await
        .unwrap()
}
```

**What it does** — A fresh offline `DbBackend` at `{dir}/keeplin.db` (empty
server URL and token → no WebSocket, no handshake).

**Used by** — all three tests.

---

## fn fs_to_db_round_trip

**Identification** — `#[tokio::test]`. Marker `// md:fn fs_to_db_round_trip`.

**Code** — complete and verbatim:

```rust
// md:fn fs_to_db_round_trip
#[tokio::test]
async fn fs_to_db_round_trip() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let src = FsBackend::new(src_dir.path()).await.unwrap();
    let dst = db(dst_dir.path()).await;

    let seeded = seed(&src).await;
    let report = migrate(&src, &dst).await.unwrap();

    assert_eq!(report.notebooks, 1);
    assert_eq!(report.tags, 1);
    assert_eq!(report.notes, 2);
    assert_eq!(report.note_tags, 1);
    assert_eq!(report.resources, 1);
    assert_migrated(&dst, &seeded).await;
}
```

**What it does** — Seed an `FsBackend`, migrate into a `DbBackend`: the
`MigrationReport` counts exactly 1 notebook, 1 tag, 2 notes, 1 note-tag,
1 resource, and `assert_migrated` passes.

---

## fn db_to_fs_round_trip

**Identification** — `#[tokio::test]`. Marker `// md:fn db_to_fs_round_trip`.

**Code** — complete and verbatim:

```rust
// md:fn db_to_fs_round_trip
#[tokio::test]
async fn db_to_fs_round_trip() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let src = db(src_dir.path()).await;
    let dst = FsBackend::new(dst_dir.path()).await.unwrap();

    let seeded = seed(&src).await;
    let report = migrate(&src, &dst).await.unwrap();

    assert_eq!(report.notes, 2);
    assert_eq!(report.resources, 1);
    assert_migrated(&dst, &seeded).await;
}
```

**What it does** — The reverse direction (`DbBackend` → `FsBackend`); same
fidelity (spot-checks notes/resources counts, then the full assertion).

---

## fn encrypted_fs_to_encrypted_db

**Identification** — `#[tokio::test]`. Marker
`// md:fn encrypted_fs_to_encrypted_db`.

**Code** — complete and verbatim:

```rust
// md:fn encrypted_fs_to_encrypted_db
#[tokio::test]
async fn encrypted_fs_to_encrypted_db() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let src = EncryptedBackend::new(
        FsBackend::new(src_dir.path()).await.unwrap(),
        "source-pass",
        b"source-salt",
    )
    .await
    .unwrap();
    let dst = EncryptedBackend::new(db(dst_dir.path()).await, "dest-pass", b"dest-salt")
        .await
        .unwrap();

    let seeded = seed(&src).await;
    migrate(&src, &dst).await.unwrap();

    assert_migrated(&dst, &seeded).await;
}
```

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
| 1 | `Overview` | `// md:Overview` |
| 2 | `Seeded` | `// md:Seeded` |
| 3 | `fn seed` | `// md:fn seed` |
| 4 | `fn assert_migrated` | `// md:fn assert_migrated` |
| 5 | `fn db` | `// md:fn db` |
| 6 | `fn fs_to_db_round_trip` | `// md:fn fs_to_db_round_trip` |
| 7 | `fn db_to_fs_round_trip` | `// md:fn db_to_fs_round_trip` |
| 8 | `fn encrypted_fs_to_encrypted_db` | `// md:fn encrypted_fs_to_encrypted_db` |