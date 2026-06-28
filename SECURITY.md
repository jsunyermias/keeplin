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
  same reason. Set `KEEPLIN_AUTH_USERNAME` similarly for the username.
- Enable TLS by setting `tls_cert_path` and `tls_key_path` in `keeplin.toml`. Without
  TLS, gRPC traffic (including auth credentials) is transmitted in plaintext.
- When `grpc_addr` is not a loopback address, the daemon logs a warning if
  `auth_username`/`auth_password` are not configured.

## Design decisions

### Multi-device encryption constraint

All devices that sync with each other **must share the same `encryption_password`**.
Encryption happens before data is written to storage and before sync — so if two devices
use different passwords, the data each device stores is ciphertext encrypted under
different keys, and sync will propagate ciphertext that the peer cannot decrypt.
Keeplin does not detect or prevent mixed-password sync configurations.

### Sync delivery guarantee

WebSocket sync (server mode) is **at-least-once**: `send_changes` retries up to 3 times
with exponential backoff (2 s, 4 s, 8 s) and each batch carries a `batch_id` UUID so the
server can deduplicate retried batches. There is no application-level ACK — permanent loss
of a batch is only possible if the server is unreachable for all retry attempts and the
client never comes back online. All `apply_change` operations are idempotent
(`INSERT OR IGNORE`, `INSERT OR REPLACE`, marker-file creation/removal), so
re-delivery is safe.

### Resource deletion

Resources use **hard delete** (data removed immediately from disk / database).
This is intentional: binary payloads can be large and there is no business need to
retain deleted attachment data. The `ResourceDelete` entry in the change journal
ensures the deletion propagates correctly to other synced devices.

## Known limitations

- **WebSocket token in plaintext.** When using server mode (`DbBackend`), the
  authentication token is sent in the first WebSocket message. If the WebSocket URL
  uses `ws://` (unencrypted), the token is transmitted in plaintext on the network.
  Always use a `wss://` URL or place a TLS-terminating reverse proxy in front of the
  WebSocket endpoint in production deployments.

- **Mixed-backend sync is not supported.** It is not possible to mix `FsBackend`
  and `DbBackend` devices in the same sync topology. Each backend uses a different
  change-propagation mechanism (file replication vs. WebSocket) and the two are
  incompatible. Attempting to mix them may produce undefined behaviour (missing or
  duplicated changes).

- **Conflict resolution is last-write-wins.** When two devices modify the same entity
  concurrently, the version with the later `updated_at` timestamp wins. No three-way
  merge is attempted. The losing version is overwritten without warning.

## Reporting vulnerabilities

Please open a confidential issue or contact the maintainers directly.
