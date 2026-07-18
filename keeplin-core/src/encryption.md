# `encryption.rs` — transparent at-rest encryption decorator

Self-contained companion for `keeplin-core/src/encryption.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

```rust
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
    storage::{
        EntityVersion, HistoryRepository, NoteRepository, NotebookRepository, ResourceRepository,
        StorageBackend, SyncBackend, TagRepository,
    },
};
```

**What it does** — `EncryptedBackend<B>` wraps any `B: StorageBackend` and
transparently encrypts sensitive fields before they reach the inner backend,
decrypting them on the way out — callers use the same `StorageBackend` trait as a
plain backend. Scheme: **AES-256-GCM** (authenticated — tampering is detected);
key derived via **Argon2id** (64 MiB memory, 3 iterations, parallelism 1); salt
supplied by the caller — not secret, but it must be **stable** and **identical on
every device that reads the same data** (the daemon passes config `key_salt`, or
the device id when unset: per-device salt keeps data local, a shared salt makes
it portable across synced devices); a fresh 12-byte random nonce **per
encryption call**; wire format `base64(nonce ‖ ciphertext)` for strings, raw
`nonce ‖ ciphertext` bytes for binaries.

**Dependencies** — `aes_gcm`, `argon2`, `base64`, `async_trait`, `chrono`,
`uuid`; the crate's error/model/storage-trait types.

**Used by** — `keeplin-daemon/src/main.rs` (constructs it when
`encryption_password` is configured); `keeplin-core/tests/encryption.rs`;
`migrate.rs` relies on its decrypt-on-read/encrypt-on-write for cross-key
migration.

**Repeated context** — Invariants (also in `SECURITY.md`): callers never see
ciphertext and the inner backend never sees plaintext for the protected fields;
encrypted fields produce different ciphertext per write (fresh nonce), so
equality of stored values never leaks equality of plaintext — which is exactly
why there is no database `UNIQUE` index for aliases (equal plaintexts become
different ciphertexts; `LinkingBackend` enforces uniqueness by scanning). Field
policy: human-readable content is encrypted; UUIDs, timestamps, sizes, flags,
sort keys, and vv metadata stay plaintext because queries and resolution need
them.

---

## NONCE_LEN

**Identification** — `const NONCE_LEN: usize = 12;` marker `// md:NONCE_LEN`.

**What it does** — The AES-GCM nonce length. AES-GCM is specified with a 96-bit
(12-byte) nonce; this must not change without changing the cipher.

**Dependencies** — none.

**Used by** — `decrypt_str`, `decrypt_bytes` (nonce extraction and the
too-short check).

**Repeated context** — none.

---

## EncryptedBackend

**Identification** — `pub struct EncryptedBackend<B: StorageBackend>`; marker
`// md:EncryptedBackend`.

**What it does** — The decorator: `inner: B` (all reads/writes ultimately go
through it) and `cipher: Aes256Gcm` (initialised once with the Argon2id-derived
key). Encrypted fields: `Note.title`/`body`/`alias`/each bookmark's
`text`+`alias`/each link's `raw`; `Notebook.title`/`alias`; `Tag.title`;
`Resource.title`/`mime_type`/`file_name`; and resource binary payloads.

**Dependencies** — `Aes256Gcm`, `StorageBackend`.

**Used by** — daemon startup; tests.

**Repeated context** — decorator stacking is the project's composition pattern
(`LinkingBackend<EncryptedBackend<FsBackend>>`, …); each layer implements the
sub-traits and delegates inward.

---

## impl EncryptedBackend

**Identification** — inherent impl `impl<B: StorageBackend> EncryptedBackend<B>`;
marker `// md:impl EncryptedBackend`. Constructor, four crypto primitives, and
ten per-entity field mappers.

### fn new

**Identification** —
`pub async fn new(inner: B, password: &str, salt: &[u8]) -> Result<Self, StorageError>`;
marker `// md:impl EncryptedBackend > fn new`.

**What it does** — Derives the AES-256 key from `password` + `salt` via
`derive_key` and builds the cipher. `salt` must be stable across restarts and
identical on every device that must decrypt the same data; Argon2id requires at
least 8 bytes. Errors as `StorageError::InvalidState` if parameter construction
or derivation fails.

**Dependencies** — `derive_key`, `Aes256Gcm`.

**Used by** — `keeplin-daemon/src/main.rs::build_storage`; tests.

**Repeated context** — none.

### fn encrypt_str

**Identification** — `fn encrypt_str(&self, plaintext: &str) -> Result<String, StorageError>`;
marker `// md:impl EncryptedBackend > fn encrypt_str`.

**What it does** — Fresh 12-byte random nonce (semantic security: the same
plaintext encrypts differently every time), AES-GCM encrypt, prepend the nonce
so decryption needs no separate storage, Base64-encode the combined buffer so
the result is plain ASCII storable in a JSON field. Encrypt failure →
`InvalidState`.

**Dependencies** — `aes_gcm`, `base64`, `OsRng`.

**Used by** — every `enc_*` mapper.

**Repeated context** — none.

### fn decrypt_str

**Identification** — `fn decrypt_str(&self, encoded: &str) -> Result<String, StorageError>`;
marker `// md:impl EncryptedBackend > fn decrypt_str`.

**What it does** — Base64-decode, check length ≥ `NONCE_LEN`, split
nonce ‖ ciphertext, AES-GCM decrypt, UTF-8 decode. **Every** failure maps to
`StorageError::CorruptedData` (malformed base64, short buffer, failed
authentication tag — wrong key or tampering — or invalid UTF-8) so callers and
the daemon's error mapping handle them uniformly.

**Dependencies** — `aes_gcm`, `base64`, `NONCE_LEN`.

**Used by** — every `dec_*` mapper.

**Repeated context** — a wrong password surfaces as `CorruptedData`, not a
dedicated "wrong password" error — the cipher cannot distinguish the two.

### fn encrypt_bytes

**Identification** — `fn encrypt_bytes(&self, data: &[u8]) -> Result<Vec<u8>, StorageError>`;
marker `// md:impl EncryptedBackend > fn encrypt_bytes`.

**What it does** — Same as `encrypt_str` but returns raw `nonce ‖ ciphertext`
bytes without Base64 — the caller stores them directly in a binary column/file.

**Dependencies** — `aes_gcm`, `OsRng`.

**Used by** — `create_resource`.

**Repeated context** — none.

### fn decrypt_bytes

**Identification** — `fn decrypt_bytes(&self, data: &[u8]) -> Result<Vec<u8>, StorageError>`;
marker `// md:impl EncryptedBackend > fn decrypt_bytes`.

**What it does** — Length check, nonce extraction, AES-GCM decrypt; failures →
`CorruptedData`.

**Dependencies** — `aes_gcm`, `NONCE_LEN`.

**Used by** — `read_resource`.

**Repeated context** — none.

### fn enc_note

**Identification** — `fn enc_note(&self, mut n: Note) -> Result<Note, StorageError>`;
marker `// md:impl EncryptedBackend > fn enc_note`.

**What it does** — Encrypts `title`, `body`, the optional `alias`, every
bookmark's `text` and `alias`, and every link's `raw`. Alias/bookmarks/links are
derived from (or describe) the sensitive body, so they are encrypted too; UUIDs
(`target_note_id`, `notebook_id`) stay plaintext per the field-level policy.

**Dependencies** — `encrypt_str`.

**Used by** — `create_note`, `update_note`.

**Repeated context** — none.

### fn dec_note

**Identification** — `fn dec_note(&self, mut n: Note) -> Result<Note, StorageError>`;
marker `// md:impl EncryptedBackend > fn dec_note`.

**What it does** — Exact inverse of `enc_note` over the same fields.

**Dependencies** — `decrypt_str`.

**Used by** — every note read/list path and `note_history`.

**Repeated context** — none.

### fn enc_notebook

**Identification** — marker `// md:impl EncryptedBackend > fn enc_notebook`.

**What it does** — Encrypts `Notebook.title` and the optional `alias`.

**Dependencies** — `encrypt_str`.

**Used by** — `create_notebook`, `update_notebook`.

**Repeated context** — none.

### fn dec_notebook

**Identification** — marker `// md:impl EncryptedBackend > fn dec_notebook`.

**What it does** — Inverse of `enc_notebook`.

**Dependencies** — `decrypt_str`.

**Used by** — notebook read/list paths and `notebook_history`.

**Repeated context** — none.

### fn enc_tag

**Identification** — marker `// md:impl EncryptedBackend > fn enc_tag`.

**What it does** — Encrypts `Tag.title`.

**Dependencies** — `encrypt_str`.

**Used by** — `create_tag`, `update_tag`.

**Repeated context** — none.

### fn dec_tag

**Identification** — marker `// md:impl EncryptedBackend > fn dec_tag`.

**What it does** — Inverse of `enc_tag`.

**Dependencies** — `decrypt_str`.

**Used by** — tag read/list paths.

**Repeated context** — none.

### fn enc_resource

**Identification** — marker `// md:impl EncryptedBackend > fn enc_resource`.

**What it does** — Encrypts `Resource.title`, `mime_type`, `file_name`; `size`
stays plaintext (needed to validate uploads without decrypting the payload).

**Dependencies** — `encrypt_str`.

**Used by** — `create_resource`.

**Repeated context** — none.

### fn dec_resource

**Identification** — marker `// md:impl EncryptedBackend > fn dec_resource`.

**What it does** — Inverse of `enc_resource`.

**Dependencies** — `decrypt_str`.

**Used by** — resource read/list paths.

**Repeated context** — none.

---

## fn derive_key

**Identification** —
`fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32], StorageError>`;
marker `// md:fn derive_key`.

**What it does** — Argon2id (`Algorithm::Argon2id`, `Version::V0x13`) with
memory 64 MiB (`65536` KiB), 3 iterations, parallelism 1, 32-byte output —
roughly 300 ms on a modern laptop, balancing security against startup latency.
Failures (bad params, kdf error) → `InvalidState`. The salt must be a stable,
persisted byte sequence; it need not be secret.

**Dependencies** — `argon2`.

**Used by** — `EncryptedBackend::new` (its only caller).

**Repeated context** — the same parameter profile is documented in
`SECURITY.md`. keeplin-srv's server-side at-rest scheme (`enc:v1:`) is
independent — this one protects *client* stores.

---

## impl NoteRepository for EncryptedBackend

**Identification** —
`#[async_trait] impl<B: StorageBackend> NoteRepository for EncryptedBackend<B>`;
marker `// md:impl NoteRepository for EncryptedBackend` (one marker for the whole
impl block; its methods are documented in the table below).

**What it does** — The note surface, encrypt-on-write / decrypt-on-read:

| Method | Behaviour |
|--------|-----------|
| `create_note` / `update_note` | `enc_note` → inner → `dec_note` of the stored copy (caller gets plaintext back) |
| `read_note` | inner → `dec_note` |
| `delete_note` | pure delegation (id-only) |
| `list_notes` / `list_starred_notes` | inner page → decrypt every note; the first failure aborts the whole page |
| `list_notes_in_notebook` | delegation + page decryption — `notebook_id`/`sort_key` are plaintext, so the inner backend orders natively |
| `note_backlinks` | **explicit delegation** (not the trait default) so an inner indexed backend is reached — `target_note_id` is plaintext, so the index works under encryption — then decrypt the page |
| `notebook_sort_profile` | pure delegation — plaintext metadata, nothing to decrypt |

**Dependencies** — `enc_/dec_note`, the inner backend.

**Used by** — all note traffic when encryption is configured.

**Repeated context** — decorators must override defaulted trait methods with
delegation (the `note_backlinks` rule from `storage/backend.md`), or inner
indexes are silently bypassed.

---

## impl NotebookRepository for EncryptedBackend

**Identification** — marker `// md:impl NotebookRepository for EncryptedBackend`
(one marker for the whole impl block).

**What it does** — `create_notebook`/`update_notebook` encrypt → store → decrypt
the stored copy; `read_notebook`/`list_notebooks` decrypt on the way out;
`delete_notebook` delegates (id-only).

**Dependencies** — `enc_/dec_notebook`.

**Used by** — notebook traffic.

**Repeated context** — none.

---

## impl TagRepository for EncryptedBackend

**Identification** — marker `// md:impl TagRepository for EncryptedBackend`
(one marker for the whole impl block).

**What it does** — `create_tag`/`update_tag` encrypt → store → decrypt;
`read_tag`/`list_tags`/`list_note_tags` decrypt pages; `add_note_tag`/
`remove_note_tag` delegate unchanged (pure UUID pairs — nothing sensitive).

**Dependencies** — `enc_/dec_tag`.

**Used by** — tag traffic.

**Repeated context** — none.

---

## impl ResourceRepository for EncryptedBackend

**Identification** — marker `// md:impl ResourceRepository for EncryptedBackend`;
per-method markers `> fn <name>`.

**What it does** — `create_resource` encrypts the metadata (`enc_resource`) and
the payload (`encrypt_bytes`) before storing; `read_resource` decrypts both;
`list_resources` decrypts the metadata page; `delete_resource` delegates;
`purge_deleted_resources` delegates — pure reclamation of (encrypted) dead
bytes, nothing to decrypt.

**Dependencies** — `enc_/dec_resource`, `encrypt_/decrypt_bytes`.

**Used by** — resource traffic.

**Repeated context** — none.

---

## impl SyncBackend for EncryptedBackend

**Identification** — marker `// md:impl SyncBackend for EncryptedBackend`
(one marker for the whole impl block).

**What it does** — **All eight methods pass through unchanged**
(`get_changes_since`, `apply_change`, `get_last_sync_time`, `update_sync_time`,
`send_changes`, `receive_changes`, `get_device_id`, `prune_change_journal`):
the data on the sync channel is already in the encrypted form the inner backend
stored on disk, so no extra crypto step is needed — the relay and peers only
ever see ciphertext.

**Dependencies** — the inner backend only.

**Used by** — `sync/engine.rs` when the stack includes encryption.

**Repeated context** — this pass-through is why `migrate.rs` cannot use raw
changes across an encryption boundary (an `apply_change` into an
`EncryptedBackend` would store plaintext): migration goes through the typed
`create_*` methods instead.

---

## impl HistoryRepository for EncryptedBackend

**Identification** — marker `// md:impl HistoryRepository for EncryptedBackend`
(one marker for the whole impl block).

**What it does** — `note_history`/`notebook_history` fetch the inner versions
(the journal stores ciphertext snapshots) and decrypt each version's entity on
the way up, exactly as `read_note` does for the current state; tombstone
versions (`entity: None`) pass through untouched.

**Dependencies** — `dec_note`/`dec_notebook`, `EntityVersion`.

**Used by** — `crate::history` and the daemon's history endpoints.

**Repeated context** — history reads return plaintext even on encrypted stores —
unlike `get_changes_since`, which deliberately passes ciphertext through for the
relay.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `EncryptedBackend<B>` — defined here (EXTRACTED; 6 cross-file edge(s))
- `.note_history()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `.notebook_history()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `.enc_note()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.dec_note()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.enc_notebook()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.dec_notebook()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.enc_tag()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.dec_tag()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.enc_resource()` — defined here (EXTRACTED; 2 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs` — error types (EXTRACTED: references×51; e.g. `StorageError`)
- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: references×34; e.g. `Note`, `Notebook`, `Tag`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: implements×6, references×3; e.g. `NotebookRepository`, `NoteRepository`, `ResourceRepository`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/tests/encryption.rs` — EncryptedBackend integration tests (EXTRACTED: references×1; e.g. `enc_backend()`)
- `keeplin-daemon/src/main.rs` — constructs `EncryptedBackend` when `encryption_password` is configured (INFERRED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `const NONCE_LEN` | `// md:NONCE_LEN` |
| 3 | `struct EncryptedBackend` | `// md:EncryptedBackend` |
| 4 | `impl EncryptedBackend` | `// md:impl EncryptedBackend` |
| 5–17 | `fn new`, `encrypt_str`, `decrypt_str`, `encrypt_bytes`, `decrypt_bytes`, `enc_note`, `dec_note`, `enc_notebook`, `dec_notebook`, `enc_tag`, `dec_tag`, `enc_resource`, `dec_resource` | `// md:impl EncryptedBackend > fn <name>` |
| 18 | `fn derive_key` | `// md:fn derive_key` |
| 19 | `impl NoteRepository for EncryptedBackend` (9 methods) | `// md:impl NoteRepository for EncryptedBackend` |
| 20 | `impl NotebookRepository for EncryptedBackend` (5 methods) | `// md:impl NotebookRepository for EncryptedBackend` |
| 21 | `impl TagRepository for EncryptedBackend` (8 methods) | `// md:impl TagRepository for EncryptedBackend` |
| 22 | `impl SyncBackend for EncryptedBackend` (8 methods) | `// md:impl SyncBackend for EncryptedBackend` |
| 23 | `impl HistoryRepository for EncryptedBackend` (2 methods) | `// md:impl HistoryRepository for EncryptedBackend` |
