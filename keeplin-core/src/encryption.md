# `encryption.rs` — transparent at-rest encryption decorator

Self-contained companion for `keeplin-core/src/encryption.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand it without opening anything else, so project-wide conventions are
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

**Code** — complete and verbatim:

```rust
// md:NONCE_LEN
const NONCE_LEN: usize = 12;
```

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

**Code** — complete and verbatim:

```rust
// md:EncryptedBackend
pub struct EncryptedBackend<B: StorageBackend> {
    inner: B,
    cipher: Aes256Gcm,
}
```

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

**Code** — container: members documented as sub-blocks below: fn new, fn encrypt_str, fn decrypt_str, fn encrypt_bytes, fn decrypt_bytes, fn enc_note, fn dec_note, fn enc_notebook, fn dec_notebook, fn enc_tag, fn dec_tag, fn enc_resource, fn dec_resource.

### fn new

**Identification** —
`pub async fn new(inner: B, password: &str, salt: &[u8]) -> Result<Self, StorageError>`;
marker `// md:impl EncryptedBackend > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl EncryptedBackend > fn new
    pub async fn new(inner: B, password: &str, salt: &[u8]) -> Result<Self, StorageError> {
        let key = derive_key(password, salt)?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        Ok(Self { inner, cipher })
    }
```

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

**Code** — complete and verbatim:

```rust
    // md:impl EncryptedBackend > fn encrypt_str
    fn encrypt_str(&self, plaintext: &str) -> Result<String, StorageError> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = self
            .cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|e| StorageError::InvalidState(format!("encrypt: {e}")))?;
        let mut combined = nonce.to_vec();
        combined.extend_from_slice(&ct);
        Ok(STANDARD.encode(&combined))
    }
```

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

**Code** — complete and verbatim:

```rust
    // md:impl EncryptedBackend > fn decrypt_str
    fn decrypt_str(&self, encoded: &str) -> Result<String, StorageError> {
        let combined = STANDARD
            .decode(encoded)
            .map_err(|e| StorageError::CorruptedData(format!("base64: {e}")))?;
        if combined.len() < NONCE_LEN {
            return Err(StorageError::CorruptedData("ciphertext too short".into()));
        }
        let nonce = Nonce::from_slice(&combined[..NONCE_LEN]);
        let plain = self
            .cipher
            .decrypt(nonce, &combined[NONCE_LEN..])
            .map_err(|e| StorageError::CorruptedData(format!("decrypt: {e}")))?;
        String::from_utf8(plain).map_err(|e| StorageError::CorruptedData(format!("utf8: {e}")))
    }
```

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

**Code** — complete and verbatim:

```rust
    // md:impl EncryptedBackend > fn encrypt_bytes
    fn encrypt_bytes(&self, data: &[u8]) -> Result<Vec<u8>, StorageError> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = self
            .cipher
            .encrypt(&nonce, data)
            .map_err(|e| StorageError::InvalidState(format!("encrypt: {e}")))?;
        let mut combined = nonce.to_vec();
        combined.extend_from_slice(&ct);
        Ok(combined)
    }
```

**What it does** — Same as `encrypt_str` but returns raw `nonce ‖ ciphertext`
bytes without Base64 — the caller stores them directly in a binary column/file.

**Dependencies** — `aes_gcm`, `OsRng`.

**Used by** — `create_resource`.

**Repeated context** — none.

### fn decrypt_bytes

**Identification** — `fn decrypt_bytes(&self, data: &[u8]) -> Result<Vec<u8>, StorageError>`;
marker `// md:impl EncryptedBackend > fn decrypt_bytes`.

**Code** — complete and verbatim:

```rust
    // md:impl EncryptedBackend > fn decrypt_bytes
    fn decrypt_bytes(&self, data: &[u8]) -> Result<Vec<u8>, StorageError> {
        if data.len() < NONCE_LEN {
            return Err(StorageError::CorruptedData("ciphertext too short".into()));
        }
        let nonce = Nonce::from_slice(&data[..NONCE_LEN]);
        self.cipher
            .decrypt(nonce, &data[NONCE_LEN..])
            .map_err(|e| StorageError::CorruptedData(format!("decrypt: {e}")))
    }
```

**What it does** — Length check, nonce extraction, AES-GCM decrypt; failures →
`CorruptedData`.

**Dependencies** — `aes_gcm`, `NONCE_LEN`.

**Used by** — `read_resource`.

**Repeated context** — none.

### fn enc_note

**Identification** — `fn enc_note(&self, mut n: Note) -> Result<Note, StorageError>`;
marker `// md:impl EncryptedBackend > fn enc_note`.

**Code** — complete and verbatim:

```rust
    // md:impl EncryptedBackend > fn enc_note
    fn enc_note(&self, mut n: Note) -> Result<Note, StorageError> {
        n.title = self.encrypt_str(&n.title)?;
        n.body = self.encrypt_str(&n.body)?;
        n.alias = n.alias.map(|a| self.encrypt_str(&a)).transpose()?;
        for b in &mut n.bookmarks {
            b.text = self.encrypt_str(&b.text)?;
            b.alias = self.encrypt_str(&b.alias)?;
        }
        for l in &mut n.links {
            l.raw = self.encrypt_str(&l.raw)?;
        }
        Ok(n)
    }
```

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

**Code** — complete and verbatim:

```rust
    // md:impl EncryptedBackend > fn dec_note
    fn dec_note(&self, mut n: Note) -> Result<Note, StorageError> {
        n.title = self.decrypt_str(&n.title)?;
        n.body = self.decrypt_str(&n.body)?;
        n.alias = n.alias.map(|a| self.decrypt_str(&a)).transpose()?;
        for b in &mut n.bookmarks {
            b.text = self.decrypt_str(&b.text)?;
            b.alias = self.decrypt_str(&b.alias)?;
        }
        for l in &mut n.links {
            l.raw = self.decrypt_str(&l.raw)?;
        }
        Ok(n)
    }
```

**What it does** — Exact inverse of `enc_note` over the same fields.

**Dependencies** — `decrypt_str`.

**Used by** — every note read/list path and `note_history`.

**Repeated context** — none.

### fn enc_notebook

**Identification** — marker `// md:impl EncryptedBackend > fn enc_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl EncryptedBackend > fn enc_notebook
    fn enc_notebook(&self, mut nb: Notebook) -> Result<Notebook, StorageError> {
        nb.title = self.encrypt_str(&nb.title)?;
        nb.alias = nb.alias.map(|a| self.encrypt_str(&a)).transpose()?;
        Ok(nb)
    }
```

**What it does** — Encrypts `Notebook.title` and the optional `alias`.

**Dependencies** — `encrypt_str`.

**Used by** — `create_notebook`, `update_notebook`.

**Repeated context** — none.

### fn dec_notebook

**Identification** — marker `// md:impl EncryptedBackend > fn dec_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl EncryptedBackend > fn dec_notebook
    fn dec_notebook(&self, mut nb: Notebook) -> Result<Notebook, StorageError> {
        nb.title = self.decrypt_str(&nb.title)?;
        nb.alias = nb.alias.map(|a| self.decrypt_str(&a)).transpose()?;
        Ok(nb)
    }
```

**What it does** — Inverse of `enc_notebook`.

**Dependencies** — `decrypt_str`.

**Used by** — notebook read/list paths and `notebook_history`.

**Repeated context** — none.

### fn enc_tag

**Identification** — marker `// md:impl EncryptedBackend > fn enc_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl EncryptedBackend > fn enc_tag
    fn enc_tag(&self, mut t: Tag) -> Result<Tag, StorageError> {
        t.title = self.encrypt_str(&t.title)?;
        Ok(t)
    }
```

**What it does** — Encrypts `Tag.title`.

**Dependencies** — `encrypt_str`.

**Used by** — `create_tag`, `update_tag`.

**Repeated context** — none.

### fn dec_tag

**Identification** — marker `// md:impl EncryptedBackend > fn dec_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl EncryptedBackend > fn dec_tag
    fn dec_tag(&self, mut t: Tag) -> Result<Tag, StorageError> {
        t.title = self.decrypt_str(&t.title)?;
        Ok(t)
    }
```

**What it does** — Inverse of `enc_tag`.

**Dependencies** — `decrypt_str`.

**Used by** — tag read/list paths.

**Repeated context** — none.

### fn enc_resource

**Identification** — marker `// md:impl EncryptedBackend > fn enc_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl EncryptedBackend > fn enc_resource
    fn enc_resource(&self, mut r: Resource) -> Result<Resource, StorageError> {
        r.title = self.encrypt_str(&r.title)?;
        r.mime_type = self.encrypt_str(&r.mime_type)?;
        r.file_name = self.encrypt_str(&r.file_name)?;
        Ok(r)
    }
```

**What it does** — Encrypts `Resource.title`, `mime_type`, `file_name`; `size`
stays plaintext (needed to validate uploads without decrypting the payload).

**Dependencies** — `encrypt_str`.

**Used by** — `create_resource`.

**Repeated context** — none.

### fn dec_resource

**Identification** — marker `// md:impl EncryptedBackend > fn dec_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl EncryptedBackend > fn dec_resource
    fn dec_resource(&self, mut r: Resource) -> Result<Resource, StorageError> {
        r.title = self.decrypt_str(&r.title)?;
        r.mime_type = self.decrypt_str(&r.mime_type)?;
        r.file_name = self.decrypt_str(&r.file_name)?;
        Ok(r)
    }
```

**What it does** — Inverse of `enc_resource`.

**Dependencies** — `decrypt_str`.

**Used by** — resource read/list paths.

**Repeated context** — none.

---

## fn derive_key

**Identification** —
`fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32], StorageError>`;
marker `// md:fn derive_key`.

**Code** — complete and verbatim:

```rust
// md:fn derive_key
fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32], StorageError> {
    let params = Params::new(65536, 3, 1, Some(32))
        .map_err(|e| StorageError::InvalidState(format!("argon2 params: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| StorageError::InvalidState(format!("kdf: {e}")))?;
    Ok(key)
}
```

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

**Code** — complete and verbatim:

```rust
// md:impl NoteRepository for EncryptedBackend
#[async_trait]
impl<B: StorageBackend> NoteRepository for EncryptedBackend<B> {
    async fn create_note(&self, note: Note) -> Result<Note, StorageError> {
        let stored = self.inner.create_note(self.enc_note(note)?).await?;
        self.dec_note(stored)
    }

    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        self.dec_note(self.inner.read_note(id).await?)
    }

    async fn update_note(&self, note: Note) -> Result<Note, StorageError> {
        let stored = self.inner.update_note(self.enc_note(note)?).await?;
        self.dec_note(stored)
    }

    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_note(id).await
    }

    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let (notes, next) = self.inner.list_notes(page_size, page_token).await?;
        let decrypted: Result<Vec<Note>, StorageError> =
            notes.into_iter().map(|n| self.dec_note(n)).collect();
        Ok((decrypted?, next))
    }

    async fn note_backlinks(
        &self,
        target_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let (notes, next) = self
            .inner
            .note_backlinks(target_id, page_size, page_token)
            .await?;
        let decrypted: Result<Vec<Note>, StorageError> =
            notes.into_iter().map(|n| self.dec_note(n)).collect();
        Ok((decrypted?, next))
    }

    async fn list_notes_in_notebook(
        &self,
        notebook_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let (notes, next) = self
            .inner
            .list_notes_in_notebook(notebook_id, page_size, page_token)
            .await?;
        let decrypted: Result<Vec<Note>, StorageError> =
            notes.into_iter().map(|n| self.dec_note(n)).collect();
        Ok((decrypted?, next))
    }

    async fn list_starred_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let (notes, next) = self.inner.list_starred_notes(page_size, page_token).await?;
        let decrypted: Result<Vec<Note>, StorageError> =
            notes.into_iter().map(|n| self.dec_note(n)).collect();
        Ok((decrypted?, next))
    }

    async fn notebook_sort_profile(
        &self,
        notebook_id: Uuid,
    ) -> Result<crate::storage::NotebookSortProfile, StorageError> {
        self.inner.notebook_sort_profile(notebook_id).await
    }
}
```

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

**Code** — complete and verbatim:

```rust
// md:impl NotebookRepository for EncryptedBackend
#[async_trait]
impl<B: StorageBackend> NotebookRepository for EncryptedBackend<B> {
    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        let stored = self
            .inner
            .create_notebook(self.enc_notebook(notebook)?)
            .await?;
        self.dec_notebook(stored)
    }

    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        self.dec_notebook(self.inner.read_notebook(id).await?)
    }

    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        let stored = self
            .inner
            .update_notebook(self.enc_notebook(notebook)?)
            .await?;
        self.dec_notebook(stored)
    }

    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_notebook(id).await
    }

    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        let (notebooks, next) = self.inner.list_notebooks(page_size, page_token).await?;
        let decrypted: Result<Vec<Notebook>, StorageError> = notebooks
            .into_iter()
            .map(|nb| self.dec_notebook(nb))
            .collect();
        Ok((decrypted?, next))
    }
}
```

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

**Code** — complete and verbatim:

```rust
// md:impl TagRepository for EncryptedBackend
#[async_trait]
impl<B: StorageBackend> TagRepository for EncryptedBackend<B> {
    async fn create_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        let stored = self.inner.create_tag(self.enc_tag(tag)?).await?;
        self.dec_tag(stored)
    }

    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        self.dec_tag(self.inner.read_tag(id).await?)
    }

    async fn update_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        let stored = self.inner.update_tag(self.enc_tag(tag)?).await?;
        self.dec_tag(stored)
    }

    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_tag(id).await
    }

    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let (tags, next) = self.inner.list_tags(page_size, page_token).await?;
        let decrypted: Result<Vec<Tag>, StorageError> =
            tags.into_iter().map(|t| self.dec_tag(t)).collect();
        Ok((decrypted?, next))
    }

    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        self.inner.add_note_tag(note_tag).await
    }

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        self.inner.remove_note_tag(note_id, tag_id).await
    }

    async fn list_note_tags(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let (tags, next) = self
            .inner
            .list_note_tags(note_id, page_size, page_token)
            .await?;
        let decrypted: Result<Vec<Tag>, StorageError> =
            tags.into_iter().map(|t| self.dec_tag(t)).collect();
        Ok((decrypted?, next))
    }
}
```

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

**Code** — complete and verbatim:

```rust
// md:impl ResourceRepository for EncryptedBackend
#[async_trait]
impl<B: StorageBackend> ResourceRepository for EncryptedBackend<B> {
    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let enc_data = self.encrypt_bytes(&data)?;
        let stored = self
            .inner
            .create_resource(self.enc_resource(resource)?, enc_data)
            .await?;
        self.dec_resource(stored)
    }

    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let (res, enc_data) = self.inner.read_resource(id).await?;
        let data = self.decrypt_bytes(&enc_data)?;
        Ok((self.dec_resource(res)?, data))
    }

    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_resource(id).await
    }

    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let (resources, next) = self.inner.list_resources(page_size, page_token).await?;
        let decrypted: Result<Vec<Resource>, StorageError> = resources
            .into_iter()
            .map(|r| self.dec_resource(r))
            .collect();
        Ok((decrypted?, next))
    }

    async fn purge_deleted_resources(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, StorageError> {
        self.inner.purge_deleted_resources(older_than).await
    }
}
```

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

**Code** — complete and verbatim:

```rust
// md:impl SyncBackend for EncryptedBackend
#[async_trait]
impl<B: StorageBackend> SyncBackend for EncryptedBackend<B> {
    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError> {
        self.inner.get_changes_since(since).await
    }

    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        self.inner.apply_change(change).await
    }

    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
        self.inner.get_last_sync_time().await
    }

    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        self.inner.update_sync_time(ts).await
    }

    async fn send_changes(&self, changes: Vec<Change>) -> Result<(), StorageError> {
        self.inner.send_changes(changes).await
    }

    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        self.inner.receive_changes().await
    }

    async fn get_device_id(&self) -> Result<String, StorageError> {
        self.inner.get_device_id().await
    }

    async fn prune_change_journal(&self, older_than: DateTime<Utc>) -> Result<u64, StorageError> {
        self.inner.prune_change_journal(older_than).await
    }
}
```

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

**Code** — complete and verbatim:

```rust
// md:impl HistoryRepository for EncryptedBackend
#[async_trait]
impl<B: StorageBackend> HistoryRepository for EncryptedBackend<B> {
    async fn note_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Note>>, StorageError> {
        self.inner
            .note_history(id, limit)
            .await?
            .into_iter()
            .map(|v| {
                Ok(EntityVersion {
                    timestamp: v.timestamp,
                    device_id: v.device_id,
                    entity: v.entity.map(|n| self.dec_note(n)).transpose()?,
                })
            })
            .collect()
    }

    async fn notebook_history(
        &self,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<EntityVersion<Notebook>>, StorageError> {
        self.inner
            .notebook_history(id, limit)
            .await?
            .into_iter()
            .map(|v| {
                Ok(EntityVersion {
                    timestamp: v.timestamp,
                    device_id: v.device_id,
                    entity: v.entity.map(|n| self.dec_notebook(n)).transpose()?,
                })
            })
            .collect()
    }
}
```

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

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

<!-- Data source: CI artifact or local graphify-out/graph.json from this exact commit.
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. Never
     present inference as fact. -->
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
| 1 | `Overview` | `// md:Overview` |
| 2 | `NONCE_LEN` | `// md:NONCE_LEN` |
| 3 | `EncryptedBackend` | `// md:EncryptedBackend` |
| 4 | `impl EncryptedBackend` (container) | `// md:impl EncryptedBackend` |
| 5 | `fn new` | `// md:impl EncryptedBackend > fn new` |
| 6 | `fn encrypt_str` | `// md:impl EncryptedBackend > fn encrypt_str` |
| 7 | `fn decrypt_str` | `// md:impl EncryptedBackend > fn decrypt_str` |
| 8 | `fn encrypt_bytes` | `// md:impl EncryptedBackend > fn encrypt_bytes` |
| 9 | `fn decrypt_bytes` | `// md:impl EncryptedBackend > fn decrypt_bytes` |
| 10 | `fn enc_note` | `// md:impl EncryptedBackend > fn enc_note` |
| 11 | `fn dec_note` | `// md:impl EncryptedBackend > fn dec_note` |
| 12 | `fn enc_notebook` | `// md:impl EncryptedBackend > fn enc_notebook` |
| 13 | `fn dec_notebook` | `// md:impl EncryptedBackend > fn dec_notebook` |
| 14 | `fn enc_tag` | `// md:impl EncryptedBackend > fn enc_tag` |
| 15 | `fn dec_tag` | `// md:impl EncryptedBackend > fn dec_tag` |
| 16 | `fn enc_resource` | `// md:impl EncryptedBackend > fn enc_resource` |
| 17 | `fn dec_resource` | `// md:impl EncryptedBackend > fn dec_resource` |
| 18 | `fn derive_key` | `// md:fn derive_key` |
| 19 | `impl NoteRepository for EncryptedBackend` | `// md:impl NoteRepository for EncryptedBackend` |
| 20 | `impl NotebookRepository for EncryptedBackend` | `// md:impl NotebookRepository for EncryptedBackend` |
| 21 | `impl TagRepository for EncryptedBackend` | `// md:impl TagRepository for EncryptedBackend` |
| 22 | `impl ResourceRepository for EncryptedBackend` | `// md:impl ResourceRepository for EncryptedBackend` |
| 23 | `impl SyncBackend for EncryptedBackend` | `// md:impl SyncBackend for EncryptedBackend` |
| 24 | `impl HistoryRepository for EncryptedBackend` | `// md:impl HistoryRepository for EncryptedBackend` |