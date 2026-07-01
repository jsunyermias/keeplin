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
| Note | `title`, `body`, `alias`, each bookmark's `text`/`alias`, each link's `raw` reference |
| Notebook | `title`, `alias` |
| Tag | `title` |
| Resource | `title`, `mime_type`, `file_name`, binary payload |

Bookmark and link strings are derived from (or describe) the note body, so they are
encrypted alongside it. Alias uniqueness and reference resolution still work because they
are enforced above the encryption boundary, on the decrypted values.

Each encrypted value is independently nonce-prefixed (12-byte random nonce + AES-GCM
ciphertext, base64-encoded for string fields; raw bytes for binary data).

### Stored in plaintext by design

The following fields are **not** encrypted because they are required for indexing,
querying, and sync:

- Timestamps (`created_at`, `updated_at`, `deleted_at`)
- UUIDs (`id`, `notebook_id`, `note_id`, `tag_id`, a link's resolved `target_note_id`)
- `is_todo`, `todo_due`, `todo_completed`
- A bookmark's `number` and a link's `source` (content/manual)
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

### Conflict resolution is unified on version vectors

Both backends resolve concurrent edits with **version vectors** and the same deterministic
`(timestamp, device_id)` tiebreak (`note_log::merge` for `FsBackend`'s per-device logs,
`note_log::resolve` for `DbBackend`'s current-state rows), so every device **converges** on the
same winner across **every** entity type:

| | `FsBackend` (offline / Syncthing) | `DbBackend` (server mode) |
|---|---|---|
| Notes | **Version vectors** — converge | **Version vectors** — converge |
| Notebooks / tags | **Version vectors** — converge | **Version vectors** — converge |
| Note↔tag associations | **Version vectors** — converge | **Version vectors** — converge |
| Resources | **Version vectors** — converge (soft-delete tombstone; blob retained) | **Version vectors** — converge (soft-delete tombstone) |

Both backends stamp a version vector on every note/notebook/tag/resource write, **every note↔tag
add/remove** (associations are a versioned present/tombstone state), and **every resource delete**
(a versioned soft-delete tombstone), resolving incoming changes with `note_log::resolve`. So
concurrent edits — including two that share an `updated_at`, a concurrent tag attach-vs-detach, and
a concurrent resource delete-vs-recreate — converge on the same deterministic winner instead of the
old bare-`updated_at` last-write-wins (which **diverged permanently** on a tie), the timestamp-less
association add/remove (which was order-dependent), or the order-dependent resource hard delete.

Cross-backend live sync remains unsupported — use the one-shot `migrate` command to move a store
between backends. `FsBackend` bounds its on-disk logs automatically without affecting convergence:
per-note logs collapse to their frontier, and the global journal is rewritten as a generation-epoch
snapshot (peers reading by byte offset re-read the snapshot when the epoch changes).

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

Resources are **soft-deleted**: a delete stamps `deleted_at` plus a version vector on the
resource's metadata so the tombstone competes in `note_log::resolve` exactly like a note or tag
delete. This makes concurrent delete-vs-recreate converge on every device instead of depending on
apply order. Deleted resources are excluded from `list_resources` and read as `NotFound`, and the
`ResourceDelete` change carries the tombstone's `vv`/`last_writer` so it propagates and resolves
correctly. The binary payload is **retained on disk / in the database** after a soft delete: the
tombstone must persist so the deletion converges, and log compaction rewrites change history, not
attachment blobs. Reclaiming a deleted attachment's bytes is left to out-of-band maintenance.

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

- **Alias uniqueness is best-effort across devices.** Note and notebook aliases are
  enforced unique on the device performing a write (the duplicate is rejected). Because
  sync replays edits that were made independently on other devices, two devices can each
  assign the same alias before they exchange changes; that collision is not rejected on
  apply. Reference resolution tolerates it by deterministically picking the smallest-uuid
  match and logging a warning, so behaviour stays convergent — but the duplicate persists
  until a human renames one. Such collisions are surfaced by `GET /api/aliases/conflicts`
  (and the `ListAliasConflicts` RPC) for cleanup. (No database `UNIQUE` constraint is used:
  under encryption the stored alias is per-write ciphertext, and a hard constraint would
  break sync on apply.)

- **Filesystem notes resolve conflicts with per-note version vectors.** In `FsBackend`,
  each note keeps one append-only log per device (`notes/{id}/log.{device}.msgpack`,
  single-writer so Syncthing never conflicts on it). The note's state is the merge of all
  its logs: a causal edit applies cleanly, a genuine concurrent edit is resolved
  deterministically by last-write-wins (timestamp, then device id) so every device
  converges on the same winner. Note bodies live in `note.md` and metadata in
  `meta.msgpack` (MessagePack); both are local projections regenerated from the logs.
- **`DbBackend` resolves conflicts with version vectors too.** `apply_change` runs
  `note_log::resolve` over the stored and incoming `(vv, updated_at, last_writer)` for notes,
  notebooks, and tags: a strictly-dominating write wins, and a genuine concurrent conflict is
  broken by the deterministic `(updated_at, device_id)` tiebreak — so, unlike the old bare
  timestamp comparison, two edits sharing a timestamp converge instead of diverging. `FsBackend`
  stamps and resolves notebooks/tags/resources the same way (in `apply_change`, via `resolve` over
  the sidecar's stored vector).
- **Deletes are tombstones that participate in conflict resolution.** A delete bumps
  `updated_at`/`vv` and the `Change::*Delete` records carry that version, so a delete competes
  against edits through the same `resolve`/`merge`: a stale edit cannot resurrect a newer
  delete, and a stale delete cannot override a newer edit. Resources follow the same rule via a
  **soft-delete** tombstone (`deleted_at` + `vv`), so a concurrent resource recreate-vs-delete
  converges; only the on-disk blob is retained (reclaimed by out-of-band maintenance, not sync).

## Reporting vulnerabilities

Please open a confidential issue or contact the maintainers directly.
