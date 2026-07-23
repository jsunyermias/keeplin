# `tests/encryption.rs` — EncryptedBackend integration tests

Self-contained companion for `keeplin-core/tests/encryption.rs`. It documents
**every code block of the source file, in source order** — a reader with only this
file must be able to understand it without opening anything else, so project-wide
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
    models::{Note, NoteTag, Notebook, Resource, Tag, SYSTEM_RESOURCE_NOTE_ID},
    storage::{
        fs::FsBackend, NoteRepository, NotebookRepository, ResourceRepository, TagRepository,
    },
};
use tempfile::tempdir;
```

**What it does** — Integration tests for `EncryptedBackend` — the AES-256-GCM
encryption decorator. Every test builds an `EncryptedBackend<FsBackend>` via
`enc_backend` and exercises the full `StorageBackend` API through the encryption
layer. Three properties are verified: **round-trip correctness** (encrypted data
decrypts to the original plaintext), **confidentiality** (raw files on disk must
not contain plaintext strings or bytes — checked by bypassing the API and
reading the filesystem directly, without which a bug that skips encryption would
still pass every round-trip test), and **authentication** (a wrong password
causes an error rather than returning corrupt data).

**Repeated context** — coverage gaps by design: `EncryptedBackend<DbBackend>` is
not exercised (the encryption logic operates entirely on domain types before
they reach the inner backend, so it is identical for both), and the sync methods
are passed through unchanged, so they are tested elsewhere.

---

## TEST_SALT

**Identification** — `const TEST_SALT: &[u8] = b"keeplin-test-salt"`. Marker
`// md:TEST_SALT`.

**Code** — complete and verbatim:

```rust
// md:TEST_SALT
const TEST_SALT: &[u8] = b"keeplin-test-salt";
```

**What it does** — Fixed Argon2id salt shared by all helper-built backends, so
the derived AES key depends only on the passphrase — exactly what the
round-trip and wrong-password tests need.

**Used by** — `enc_backend`, `wrong_password_fails_to_decrypt`.

---

## fn enc_backend

**Identification** — `async fn enc_backend(dir: &std::path::Path) ->
EncryptedBackend<FsBackend>`. Marker `// md:fn enc_backend`.

**Code** — complete and verbatim:

```rust
// md:fn enc_backend
async fn enc_backend(dir: &std::path::Path) -> EncryptedBackend<FsBackend> {
    let fs = FsBackend::new(dir).await.unwrap();
    EncryptedBackend::new(fs, "test-password", TEST_SALT)
        .await
        .unwrap()
}
```

**What it does** — An `EncryptedBackend<FsBackend>` rooted at `dir` with the
fixed passphrase `"test-password"` and `TEST_SALT`, so the Argon2id-derived key
is deterministic across tests. Tests that verify a **wrong** password build
their own instances with different passphrases (same salt) instead.

**Used by** — every test except `wrong_password_fails_to_decrypt`.

---

## fn note_round_trips

**Identification** — `#[tokio::test]`. Marker `// md:fn note_round_trips`.

**Code** — complete and verbatim:

```rust
// md:fn note_round_trips
#[tokio::test]
async fn note_round_trips() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let note = Note::new("Secret title", "Secret body");
    let id = note.id;
    backend.create_note(note).await.unwrap();

    let read = backend.read_note(id).await.unwrap();
    assert_eq!(read.title, "Secret title");
    assert_eq!(read.body, "Secret body");
}
```

**What it does** — Create a note through the encrypted backend, read it back:
`title` and `body` match the original plaintext.

---

## fn storage_contains_ciphertext_not_plaintext

**Identification** — `#[tokio::test]`. Marker
`// md:fn storage_contains_ciphertext_not_plaintext`.

**Code** — complete and verbatim:

```rust
// md:fn storage_contains_ciphertext_not_plaintext
#[tokio::test]
async fn storage_contains_ciphertext_not_plaintext() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let note = Note::new("plaintext-title", "plaintext-body");
    let id = note.id;
    backend.create_note(note).await.unwrap();

    let ndir = dir.path().join("notes").join(id.to_string());
    let md = std::fs::read_to_string(ndir.join("note.md")).unwrap();
    assert!(
        !md.contains("plaintext-body"),
        "note.md should not contain plaintext body"
    );
    let meta = std::fs::read(ndir.join("meta.ndjson")).unwrap();
    let title_needle = b"plaintext-title";
    assert!(
        !meta.windows(title_needle.len()).any(|w| w == title_needle),
        "meta.ndjson should not contain plaintext title"
    );
}
```

**What it does** — Creates a note, then reads its on-disk files directly,
bypassing the encryption layer: the body file (`notes/{id}/note.md`) must not
contain the plaintext body string, and the metadata (`meta.ndjson`) must not
contain the plaintext title bytes anywhere (windowed byte search).

---

## fn wrong_password_fails_to_decrypt

**Identification** — `#[tokio::test]`. Marker
`// md:fn wrong_password_fails_to_decrypt`.

**Code** — complete and verbatim:

```rust
// md:fn wrong_password_fails_to_decrypt
#[tokio::test]
async fn wrong_password_fails_to_decrypt() {
    let dir = tempdir().unwrap();

    let fs1 = FsBackend::new(dir.path()).await.unwrap();
    let enc1 = EncryptedBackend::new(fs1, "correct", TEST_SALT)
        .await
        .unwrap();
    let note = Note::new("Hello", "World");
    let id = note.id;
    enc1.create_note(note).await.unwrap();

    let fs2 = FsBackend::new(dir.path()).await.unwrap();
    let enc2 = EncryptedBackend::new(fs2, "wrong", TEST_SALT)
        .await
        .unwrap();
    assert!(
        enc2.read_note(id).await.is_err(),
        "wrong password must fail to decrypt"
    );
}
```

**What it does** — Persists a note under the password `"correct"`, then opens
the same directory with `"wrong"` (same salt): the AES-GCM authentication tag
fails because the derived key differs, so `read_note` errors
(`StorageError::CorruptedData`) instead of returning silently corrupt data. Two
`FsBackend` instances on one directory are safe here — never used concurrently.

---

## fn update_note_encrypts_new_content

**Identification** — `#[tokio::test]`. Marker
`// md:fn update_note_encrypts_new_content`.

**Code** — complete and verbatim:

```rust
// md:fn update_note_encrypts_new_content
#[tokio::test]
async fn update_note_encrypts_new_content() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let mut note = Note::new("Old title", "Old body");
    let id = note.id;
    backend.create_note(note.clone()).await.unwrap();

    note.title = "New title".to_string();
    note.body = "New body".to_string();
    backend.update_note(note).await.unwrap();

    let read = backend.read_note(id).await.unwrap();
    assert_eq!(read.title, "New title");
    assert_eq!(read.body, "New body");
}
```

**What it does** — Create, then update title and body: the read returns the new
plaintext values (fresh ciphertext stored).

---

## fn list_notes_decrypts_all

**Identification** — `#[tokio::test]`. Marker `// md:fn list_notes_decrypts_all`.

**Code** — complete and verbatim:

```rust
// md:fn list_notes_decrypts_all
#[tokio::test]
async fn list_notes_decrypts_all() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    for i in 0..3 {
        backend
            .create_note(Note::new(format!("Note {i}"), "body"))
            .await
            .unwrap();
    }

    let (notes, _) = backend.list_notes(0, None).await.unwrap();
    assert_eq!(notes.len(), 3);
    for note in &notes {
        assert!(
            note.title.starts_with("Note "),
            "list_notes must return decrypted titles, got: {}",
            note.title
        );
    }
}
```

**What it does** — Three notes; `list_notes` returns all three with decrypted
titles (never raw ciphertext).

---

## fn notebook_round_trips

**Identification** — `#[tokio::test]`. Marker `// md:fn notebook_round_trips`.

**Code** — complete and verbatim:

```rust
// md:fn notebook_round_trips
#[tokio::test]
async fn notebook_round_trips() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let nb = Notebook::new("Private Notebook");
    let id = nb.id;
    backend.create_notebook(nb).await.unwrap();

    let read = backend.read_notebook(id).await.unwrap();
    assert_eq!(read.title, "Private Notebook");
}
```

**What it does** — Notebook title round-trips through encryption.

---

## fn tag_round_trips

**Identification** — `#[tokio::test]`. Marker `// md:fn tag_round_trips`.

**Code** — complete and verbatim:

```rust
// md:fn tag_round_trips
#[tokio::test]
async fn tag_round_trips() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let tag = Tag::new("confidential");
    let id = tag.id;
    backend.create_tag(tag).await.unwrap();

    let read = backend.read_tag(id).await.unwrap();
    assert_eq!(read.title, "confidential");
}
```

**What it does** — Tag title round-trips through encryption.

---

## fn note_tag_relation_preserved

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_tag_relation_preserved`.

**Code** — complete and verbatim:

```rust
// md:fn note_tag_relation_preserved
#[tokio::test]
async fn note_tag_relation_preserved() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let note = Note::new("N", "");
    let tag = Tag::new("T");
    let note_id = note.id;
    let tag_id = tag.id;
    backend.create_note(note).await.unwrap();
    backend.create_tag(tag).await.unwrap();
    backend
        .add_note_tag(NoteTag { note_id, tag_id })
        .await
        .unwrap();

    let (tags, _) = backend.list_note_tags(note_id, 0, None).await.unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].title, "T");
}
```

**What it does** — Note + tag created through the encrypted backend, linked with
`add_note_tag`: `list_note_tags` returns the one tag with its decrypted title.

---

## fn resource_round_trips

**Identification** — `#[tokio::test]`. Marker `// md:fn resource_round_trips`.

**Code** — complete and verbatim:

```rust
// md:fn resource_round_trips
#[tokio::test]
async fn resource_round_trips() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let data = b"secret binary content".to_vec();
    let res = Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        "attachment",
        "application/octet-stream",
        "file.bin",
        data.len() as u64,
    );
    let id = res.id;
    backend.create_resource(res, data.clone()).await.unwrap();

    let (meta, bytes) = backend.read_resource(id).await.unwrap();
    assert_eq!(meta.title, "attachment");
    assert_eq!(bytes, data);
}
```

**What it does** — Resource with binary data round-trips: metadata fields match
and the bytes are identical to the originals.

---

## fn resource_data_stored_encrypted

**Identification** — `#[tokio::test]`. Marker
`// md:fn resource_data_stored_encrypted`.

**Code** — complete and verbatim:

```rust
// md:fn resource_data_stored_encrypted
#[tokio::test]
async fn resource_data_stored_encrypted() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let data = b"supersecret".to_vec();
    let res = Resource::new(SYSTEM_RESOURCE_NOTE_ID, "r", "text/plain", "r.txt", data.len() as u64);
    let id = res.id;
    backend.create_resource(res, data).await.unwrap();

    let data_path = dir
        .path()
        .join("resources")
        .join(id.to_string())
        .join("data");
    let raw = std::fs::read(&data_path).unwrap();
    assert_ne!(
        raw, b"supersecret",
        "resource data must not be stored in plaintext"
    );
}
```

**What it does** — Reads the raw `resources/{id}/data` file directly: it
contains `nonce || ciphertext` (raw bytes, no Base64 wrapper) and must not equal
the plaintext payload.

---

## fn list_notes_paginates_and_decrypts_each_page

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_notes_paginates_and_decrypts_each_page`.

**Code** — complete and verbatim:

```rust
// md:fn list_notes_paginates_and_decrypts_each_page
#[tokio::test]
async fn list_notes_paginates_and_decrypts_each_page() {
    let dir = tempdir().unwrap();
    let backend = enc_backend(dir.path()).await;

    let total = 20usize;
    for i in 0..total {
        backend
            .create_note(Note::new(format!("Secret {i:02}"), "body"))
            .await
            .unwrap();
    }

    let page_size = 6u32;
    let mut seen = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let (page, next) = backend.list_notes(page_size, token).await.unwrap();
        assert!(page.len() <= page_size as usize);
        for note in &page {
            assert!(
                note.title.starts_with("Secret "),
                "title must be decrypted, got: {}",
                note.title
            );
        }
        seen.extend(page.iter().map(|n| n.id));
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }

    assert_eq!(seen.len(), total);
    let unique: std::collections::HashSet<_> = seen.iter().copied().collect();
    assert_eq!(unique.len(), total, "no note may appear on two pages");
}
```

**What it does** — 20 notes walked with page size 6: every page comes back
decrypted, no page exceeds the size, all 20 ids are seen, and no note appears on
two pages (cursor pagination is stable through the decrypting decorator).

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `enc_backend()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `note_round_trips()` — defined here (EXTRACTED; file-local)
- `storage_contains_ciphertext_not_plaintext()` — defined here (EXTRACTED; file-local)
- `wrong_password_fails_to_decrypt()` — defined here (EXTRACTED; file-local)
- `update_note_encrypts_new_content()` — defined here (EXTRACTED; file-local)
- `list_notes_decrypts_all()` — defined here (EXTRACTED; file-local)
- `notebook_round_trips()` — defined here (EXTRACTED; file-local)
- `tag_round_trips()` — defined here (EXTRACTED; file-local)
- `note_tag_relation_preserved()` — defined here (EXTRACTED; file-local)
- `resource_round_trips()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/encryption.rs` — transparent at-rest encryption (EXTRACTED: references×1; e.g. `EncryptedBackend`)
- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: references×1; e.g. `FsBackend`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Plaintext must never be readable in the underlying store; wrong passwords must fail loudly, not return garbage.
- Round-trips return the original plaintext byte-for-byte.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `TEST_SALT` | `// md:TEST_SALT` |
| 3 | `fn enc_backend` | `// md:fn enc_backend` |
| 4 | `fn note_round_trips` | `// md:fn note_round_trips` |
| 5 | `fn storage_contains_ciphertext_not_plaintext` | `// md:fn storage_contains_ciphertext_not_plaintext` |
| 6 | `fn wrong_password_fails_to_decrypt` | `// md:fn wrong_password_fails_to_decrypt` |
| 7 | `fn update_note_encrypts_new_content` | `// md:fn update_note_encrypts_new_content` |
| 8 | `fn list_notes_decrypts_all` | `// md:fn list_notes_decrypts_all` |
| 9 | `fn notebook_round_trips` | `// md:fn notebook_round_trips` |
| 10 | `fn tag_round_trips` | `// md:fn tag_round_trips` |
| 11 | `fn note_tag_relation_preserved` | `// md:fn note_tag_relation_preserved` |
| 12 | `fn resource_round_trips` | `// md:fn resource_round_trips` |
| 13 | `fn resource_data_stored_encrypted` | `// md:fn resource_data_stored_encrypted` |
| 14 | `fn list_notes_paginates_and_decrypts_each_page` | `// md:fn list_notes_paginates_and_decrypts_each_page` |