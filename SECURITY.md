# Security

## Encryption

When `encryption_password` is set (or `KEEPLIN_ENCRYPTION_PASSWORD` env var), Keeplin
derives a 32-byte AES-256-GCM key using Argon2id (65536 KiB, 3 iterations, 1 thread).
The device ID acts as the Argon2id salt, so the derived key is stable per installation
but unique across installations sharing the same password.

### Encrypted at rest

| Entity | Encrypted fields |
|--------|-----------------|
| Note | `title`, `body` |
| Notebook | `title` |
| Tag | `title` |
| Resource | `title`, `mime_type`, `file_name`, binary payload |

Each encrypted value is independently nonce-prefixed (12-byte random nonce + AES-GCM
ciphertext, base64-encoded for string fields; raw bytes for binary data).

### Stored in plaintext by design

The following fields are **not** encrypted because they are required for indexing,
querying, and sync:

- Timestamps (`created_at`, `updated_at`, `deleted_at`)
- UUIDs (`id`, `notebook_id`, `note_id`, `tag_id`)
- `is_todo`, `todo_due`, `todo_completed`
- `Resource.size`
- NoteTag associations (the link between a note UUID and a tag UUID)

## Threat model

Encryption protects **content at rest** against an attacker who gains physical or
file-system access to the storage directory (e.g. a stolen device or backup exposure).

It does **not** protect against:

- Analysis of temporal metadata (when notes were created/modified)
- Analysis of the association graph (which tags belong to which notes)
- An attacker who already has the derived key or the running process

## Credentials and TLS

- Set `KEEPLIN_ENCRYPTION_PASSWORD` instead of `encryption_password` in `keeplin.toml`
  to avoid committing the plaintext password to version control.
- Set `KEEPLIN_AUTH_PASSWORD` instead of `auth_password` in `keeplin.toml` for the
  same reason.
- Enable TLS by setting `tls_cert_path` and `tls_key_path` in `keeplin.toml`. Without
  TLS, gRPC traffic (including auth credentials) is transmitted in plaintext.
- When `grpc_addr` is not a loopback address, the daemon logs a warning if
  `auth_username`/`auth_password` are not configured.

## Reporting vulnerabilities

Please open a confidential issue or contact the maintainers directly.
