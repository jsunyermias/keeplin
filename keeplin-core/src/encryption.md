# `encryption.rs` — transparent at-rest encryption

## Purpose

This module provides `EncryptedBackend<B>`, a decorator that wraps any `StorageBackend`
and automatically encrypts sensitive fields before writing to the inner backend and
decrypts them on the way back out. Callers interact with the encrypted backend through the
same `StorageBackend` trait as any plain backend — encryption is completely transparent.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `EncryptedBackend<B>` | struct | Transparent encryption wrapper around another `StorageBackend` |

## Encryption scheme

| Primitive | Details |
|-----------|---------|
| Cipher | AES-256-GCM (authenticated encryption with associated data) |
| Key derivation | Argon2id, parameters: memory = 64 MiB, iterations = 3, parallelism = 1, output = 32 bytes |
| Salt | Caller-supplied (`new`'s `salt` argument); the daemon passes `key_salt` from config, or the device ID when `key_salt` is unset |
| Nonce | 12-byte random nonce generated fresh for every encryption operation |
| Wire format (strings) | `base64(nonce ‖ ciphertext)` — nonce prepended to ciphertext, then Base64-encoded |
| Wire format (bytes) | `nonce ‖ ciphertext` — raw bytes; no Base64 encoding for binary data |

## Fields that are encrypted

| Type | Encrypted fields |
|------|-----------------|
| `Note` | `title`, `body` |
| `Notebook` | `title` |
| `Tag` | `title` |
| `Resource` (metadata) | `title`, `mime_type`, `file_name` |
| `Resource` (binary payload) | entire `data` bytes |

## Fields stored in plaintext (by design)

`id`, `notebook_id`, `is_todo`, `size`, `created_at`, `updated_at`, `deleted_at`, and all
NoteTag associations. These fields are required for database queries and sync logic and
contain no user-supplied content that needs protecting.

## Public API

### `EncryptedBackend::new(inner: B, password: &str, salt: &[u8]) -> Result<Self, StorageError>`
**What it does:** Derives a 256-bit AES key from `password` and `salt` (Argon2id),
constructs an `Aes256Gcm` cipher, and wraps the inner backend.  
**Parameters:**
- `inner` — the backend to wrap; must implement `StorageBackend`
- `password` — the user-supplied passphrase; never stored, only used during key derivation
- `salt` — stable Argon2id salt (≥ 8 bytes); must be identical on every device that needs
  to read the same data. Pass a shared value for portable encryption, or a per-device
  value (e.g. the device ID) to keep data local to one installation  
**Returns:** A ready-to-use encrypted backend.  
**Errors:** `StorageError::InvalidState` if Argon2id parameter construction or key
derivation fails (e.g. `salt` is shorter than 8 bytes).

### Private helpers

| Method | Description |
|--------|-------------|
| `encrypt_str(&str) -> Result<String>` | Encrypts a string field; returns `base64(nonce ‖ ciphertext)` |
| `decrypt_str(&str) -> Result<String>` | Decodes Base64, splits nonce, decrypts, validates UTF-8 |
| `encrypt_bytes(&[u8]) -> Result<Vec<u8>>` | Encrypts binary data; returns `nonce ‖ ciphertext` |
| `decrypt_bytes(&[u8]) -> Result<Vec<u8>>` | Splits nonce, decrypts, returns plaintext bytes |
| `enc_note / dec_note` | Convenience wrappers that encrypt/decrypt all sensitive `Note` fields |
| `enc_notebook / dec_notebook` | Same for `Notebook.title` |
| `enc_tag / dec_tag` | Same for `Tag.title` |
| `enc_resource / dec_resource` | Same for `Resource.title`, `mime_type`, `file_name` |

### `fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32], StorageError>` (module-private)
**What it does:** Runs Argon2id key derivation to produce a 32-byte key.  
**Parameters:**
- `password` — user passphrase as UTF-8 bytes
- `salt` — caller-supplied stable salt bytes (≥ 8 bytes); the same `(password, salt)` pair
  always derives the same key, which is what makes encrypted data portable across devices
  that share both values  
**Returns:** 32-byte raw key material suitable for `Aes256Gcm`.

## Data flow

1. Caller calls `encrypted_backend.create_note(note)`.
2. `enc_note` encrypts `note.title` and `note.body` in place.
3. The encrypted `Note` is passed to `inner.create_note(...)`.
4. The inner backend stores the ciphertext and returns the stored copy.
5. `dec_note` decrypts the returned copy before returning it to the caller.

The same round-trip applies to all create/read/update/list methods.

## Design notes

- The salt is supplied by the caller rather than fixed to the device ID. A shared,
  configured salt (the daemon's `key_salt`) makes the derived key — and therefore the
  encrypted data — portable across devices that sync with one another; falling back to
  the device ID keeps data readable only on the device that wrote it. The salt is not
  secret but must be stable across restarts.
- Sync methods (`get_changes_since`, `apply_change`, `send_changes`, `receive_changes`)
  pass through to the inner backend without any transformation. The data that travels over
  the sync channel is already in the same encrypted form that the inner backend stores on
  disk.
- A wrong password will cause Argon2id to produce a different key. All subsequent
  `decrypt_*` calls will fail with `AES-GCM` authentication errors, surfaced as
  `StorageError::InvalidState`. No silent data corruption occurs.

## Related files

- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` trait this type implements
- `keeplin-daemon/src/main.rs` — constructs `EncryptedBackend` when `encryption_password`
  is configured
- `SECURITY.md` — full threat model and encrypted-field inventory
