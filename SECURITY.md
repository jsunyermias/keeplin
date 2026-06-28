# Security

## Encryption

When `encryption_password` is set (or `KEEPLIN_ENCRYPTION_PASSWORD` env var), Keeplin
derives a 32-byte AES-256-GCM key using Argon2id (65536 KiB, 3 iterations, 1 thread).
The Argon2id salt comes from the `key_salt` config field (or `KEEPLIN_KEY_SALT` env var)
when set; otherwise it falls back to this device's ID. The salt is not secret, but it
must be **stable** and **identical on every device that needs to read the same data**
(see the multi-device note below).

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
- The optional REST/WebSocket listener (`http_addr`) is **plain HTTP with no native TLS**.
  Its Basic-Auth credentials (the same `auth_username`/`auth_password`) and payloads travel
  unencrypted on the wire, so terminate TLS at a reverse proxy in production — exactly as for
  the `ws://` sync token below.
- When `grpc_addr` or `http_addr` is not a loopback address, the daemon logs a warning if
  `auth_username`/`auth_password` are not configured.

## Design decisions

### Conflict resolution differs by backend

The two storage backends resolve concurrent edits with **different strength**, and this
is a deliberate, load-bearing distinction:

| | `FsBackend` (offline / Syncthing) | `DbBackend` (server mode) |
|---|---|---|
| Notes | **Per-note version vectors** — genuine concurrent edits are detected and resolved deterministically, and every device **converges** on the same winner | **Last-write-wins by `updated_at`** — no version vectors, no merge |
| Notebooks / tags | Last-write-wins by `updated_at` | Last-write-wins by `updated_at` |
| Resources | Last-write-wins (hard delete) | Last-write-wins (hard delete) |

Practical consequence: if two devices edit the **same note while both are offline** and
then sync, `FsBackend` reconciles them (the causal edit wins, or a true conflict is broken
deterministically so nothing silently diverges), whereas `DbBackend` keeps only the edit
whose `updated_at` is later — the other edit is overwritten **without warning**.

Guidance: choose **offline mode** (`FsBackend` + Syncthing) when strong note-merge
guarantees matter. **Server mode** (`DbBackend`) trades that merge fidelity for a central
WebSocket relay and is best when edits rarely overlap or a single device is authoritative.
Porting version vectors to `DbBackend` is a possible future change but is not implemented
today.

### Multi-device encryption constraint

All devices that sync with each other **must share the same `encryption_password`
and the same `key_salt`**. The key is derived as `Argon2id(password, key_salt)`, so
both inputs must match for two devices to derive the same key. Because encryption
happens before data is written or synced, a mismatch in either value means a peer
receives ciphertext it cannot decrypt.

If `key_salt` is left unset, the salt defaults to the device ID — which is unique per
installation — so encrypted data is **not** portable to other devices. The daemon logs
a loud warning at startup when `encryption_password` is set without `key_salt`. For
encrypted multi-device sync, set the same `key_salt` (at least 8 bytes) on every device.
Keeplin does not otherwise detect or prevent mismatched-key sync configurations.

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

- **Filesystem notes resolve conflicts with per-note version vectors.** In `FsBackend`,
  each note keeps one append-only log per device (`notes/{id}/log.{device}.msgpack`,
  single-writer so Syncthing never conflicts on it). The note's state is the merge of all
  its logs: a causal edit applies cleanly, a genuine concurrent edit is resolved
  deterministically by last-write-wins (timestamp, then device id) so every device
  converges on the same winner. Note bodies live in `note.md` and metadata in
  `meta.msgpack` (MessagePack); both are local projections regenerated from the logs.
- **All other conflict resolution is last-write-wins by `updated_at`.** When two devices
  modify the same non-note entity (or sync notes through `DbBackend`) concurrently, the
  version with the later `updated_at` timestamp wins:
  `apply_change` compares timestamps and **ignores** an incoming change that is older
  than the local copy, so a stale remote edit can never clobber a newer local one. No
  three-way merge is attempted, and the losing version is discarded without warning.
  Equal timestamps keep the existing local record.
- **Deletes are tombstones that participate in last-write-wins.** A delete bumps
  `updated_at` to the deletion time and the `Change::*Delete` records carry that
  timestamp, so a delete competes against edits by time: a stale edit cannot resurrect a
  newer delete, and a stale delete cannot override a newer edit. (Resources are an
  exception — they are hard-deleted, so there is nothing to resurrect.)

## Reporting vulnerabilities

Please open a confidential issue or contact the maintainers directly.
