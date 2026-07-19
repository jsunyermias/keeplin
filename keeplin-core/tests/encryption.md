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

```rust
use keeplin_core::{encryption::EncryptedBackend,
    models::{Note, NoteTag, Notebook, Resource, Tag},
    storage::{fs::FsBackend, NoteRepository, NotebookRepository,
              ResourceRepository, TagRepository}};
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

**What it does** — Fixed Argon2id salt shared by all helper-built backends, so
the derived AES key depends only on the passphrase — exactly what the
round-trip and wrong-password tests need.

**Used by** — `enc_backend`, `wrong_password_fails_to_decrypt`.

---

## fn enc_backend

**Identification** — `async fn enc_backend(dir: &std::path::Path) ->
EncryptedBackend<FsBackend>`. Marker `// md:fn enc_backend`.

**What it does** — An `EncryptedBackend<FsBackend>` rooted at `dir` with the
fixed passphrase `"test-password"` and `TEST_SALT`, so the Argon2id-derived key
is deterministic across tests. Tests that verify a **wrong** password build
their own instances with different passphrases (same salt) instead.

**Used by** — every test except `wrong_password_fails_to_decrypt`.

---

## fn note_round_trips

**Identification** — `#[tokio::test]`. Marker `// md:fn note_round_trips`.

**What it does** — Create a note through the encrypted backend, read it back:
`title` and `body` match the original plaintext.

---

## fn storage_contains_ciphertext_not_plaintext

**Identification** — `#[tokio::test]`. Marker
`// md:fn storage_contains_ciphertext_not_plaintext`.

**What it does** — Creates a note, then reads its on-disk files directly,
bypassing the encryption layer: the body file (`notes/{id}/note.md`) must not
contain the plaintext body string, and the metadata (`meta.msgpack`) must not
contain the plaintext title bytes anywhere (windowed byte search).

---

## fn wrong_password_fails_to_decrypt

**Identification** — `#[tokio::test]`. Marker
`// md:fn wrong_password_fails_to_decrypt`.

**What it does** — Persists a note under the password `"correct"`, then opens
the same directory with `"wrong"` (same salt): the AES-GCM authentication tag
fails because the derived key differs, so `read_note` errors
(`StorageError::CorruptedData`) instead of returning silently corrupt data. Two
`FsBackend` instances on one directory are safe here — never used concurrently.

---

## fn update_note_encrypts_new_content

**Identification** — `#[tokio::test]`. Marker
`// md:fn update_note_encrypts_new_content`.

**What it does** — Create, then update title and body: the read returns the new
plaintext values (fresh ciphertext stored).

---

## fn list_notes_decrypts_all

**Identification** — `#[tokio::test]`. Marker `// md:fn list_notes_decrypts_all`.

**What it does** — Three notes; `list_notes` returns all three with decrypted
titles (never raw ciphertext).

---

## fn notebook_round_trips

**Identification** — `#[tokio::test]`. Marker `// md:fn notebook_round_trips`.

**What it does** — Notebook title round-trips through encryption.

---

## fn tag_round_trips

**Identification** — `#[tokio::test]`. Marker `// md:fn tag_round_trips`.

**What it does** — Tag title round-trips through encryption.

---

## fn note_tag_relation_preserved

**Identification** — `#[tokio::test]`. Marker
`// md:fn note_tag_relation_preserved`.

**What it does** — Note + tag created through the encrypted backend, linked with
`add_note_tag`: `list_note_tags` returns the one tag with its decrypted title.

---

## fn resource_round_trips

**Identification** — `#[tokio::test]`. Marker `// md:fn resource_round_trips`.

**What it does** — Resource with binary data round-trips: metadata fields match
and the bytes are identical to the originals.

---

## fn resource_data_stored_encrypted

**Identification** — `#[tokio::test]`. Marker
`// md:fn resource_data_stored_encrypted`.

**What it does** — Reads the raw `resources/{id}/data` file directly: it
contains `nonce || ciphertext` (raw bytes, no Base64 wrapper) and must not equal
the plaintext payload.

---

## fn list_notes_paginates_and_decrypts_each_page

**Identification** — `#[tokio::test]`. Marker
`// md:fn list_notes_paginates_and_decrypts_each_page`.

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
| 1 | crate doc + imports | `// md:Overview` |
| 2 | `const TEST_SALT` | `// md:TEST_SALT` |
| 3 | `fn enc_backend` | `// md:fn enc_backend` |
| 4–14 | the eleven `#[tokio::test]` fns | `// md:fn <name>` |
