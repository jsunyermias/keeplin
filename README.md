# Keeplin

A small, self-hostable **notes backend** written in Rust. Keeplin stores notes,
notebooks, tags, and binary attachments, and exposes them over a **gRPC API**. It can run
fully offline against the local filesystem (replicated between devices with
[Syncthing](https://syncthing.net/)) or in server mode against a local
LibSQL/SQLite database that synchronises with a central server over WebSocket. Sensitive
content can be **encrypted at rest** with AES‑256‑GCM.

> **Status:** pre‑release (`0.1.0`). The core is well-tested and solid for **single‑user /
> self‑hosted offline** use. It is **not yet production‑ready as a multi‑user, server‑backed
> service** — see [Project status](#project-status). Formats may still change between
> versions without a migration path.

---

## Features

- **Entities:** notes (with markdown bodies, to‑do flag, due/completed dates), notebooks,
  tags (many‑to‑many with notes), and binary resources (attachments).
- **Two storage backends**, behind one `StorageBackend` trait:
  - **Offline (`FsBackend`)** — JSON/MessagePack files on disk, replicated by Syncthing.
  - **Server (`DbBackend`)** — local LibSQL (embedded SQLite) + WebSocket sync.
- **Conflict resolution:**
  - Filesystem notes use **per‑note version vectors** with deterministic convergence
    (genuine concurrent edits merge; no silent divergence).
  - Everything else uses **last‑write‑wins** by `updated_at`, with **tombstones** so a
    stale edit can never resurrect a delete.
- **At‑rest encryption:** AES‑256‑GCM with an Argon2id‑derived key (opt‑in).
- **gRPC API** with HTTP Basic Auth (constant‑time check) and optional TLS.
- **Cursor pagination** on every list endpoint.
- **Soft delete** for notes/notebooks/tags; hard delete for resources.

---

## Architecture

A Cargo workspace with two crates:

| Crate | What it is |
|-------|------------|
| [`keeplin-core`](keeplin-core) | The library: domain models, the `StorageBackend` trait + two implementations (`FsBackend`, `DbBackend`), the `EncryptedBackend` decorator, and the `SyncEngine`. |
| [`keeplin-daemon`](keeplin-daemon) | The binary: a [tonic](https://github.com/hyperium/tonic) gRPC server (`KeeplinService`) that wires a backend to the network, with auth and TLS. |

Every `.rs` source file has a companion `.md` describing it in depth (e.g.
[`keeplin-core/src/storage/fs.md`](keeplin-core/src/storage/fs.md),
[`keeplin-core/src/storage/note_log.md`](keeplin-core/src/storage/note_log.md)).

### Storage models

**Offline mode** stores each note as a directory:

```
notes/{id}/
  note.md                  # the markdown body (ciphertext when encryption is on)
  meta.msgpack             # metadata + merged version vector
  log.{device_id}.msgpack  # append-only, one per device (single-writer → Syncthing-safe)
```

Because each per‑device log has a single writer, Syncthing never produces conflict copies;
a note's state is the **merge** of all its logs (see
[`note_log`](keeplin-core/src/storage/note_log.rs)). Notebooks, tags, and resources are
single MessagePack sidecars plus a per‑device NDJSON change log.

**Server mode** keeps everything in a local SQLite database and ships each mutation as a
`Change` over a WebSocket to a central relay, which forwards it to the other devices.
Conflict resolution here is last‑write‑wins for all entities (no version vectors).

> The two backends are **not interchangeable in one sync topology**, and they differ in
> conflict‑resolution strength — see ["Conflict resolution differs by backend"](SECURITY.md).

---

## Quick start

### Prerequisites

- A recent stable **Rust** toolchain (`rustup`).
- **`protoc`** (Protocol Buffers compiler) — the daemon's `build.rs` compiles the gRPC
  definitions. On Debian/Ubuntu: `sudo apt-get install -y protobuf-compiler`.

### Build

```bash
cargo build --release          # produces target/release/keeplin-daemon
```

`scripts/build.sh` cross‑compiles release binaries for several targets (requires the
matching toolchains).

### Configure

Create a `keeplin.toml` (offline mode, no encryption):

```toml
mode      = "offline"
data_dir  = "./keeplin-data"
grpc_addr = "127.0.0.1:50051"
```

Or server mode with auth and encryption (prefer environment variables for secrets):

```toml
mode        = "server"
data_dir    = "./keeplin-data"
server_url  = "wss://sync.example.com/ws"   # use wss:// (TLS) in production
auth_token  = ""                            # set via env if needed
grpc_addr   = "127.0.0.1:50051"
# tls_cert_path = "/etc/keeplin/cert.pem"
# tls_key_path  = "/etc/keeplin/key.pem"
```

```bash
export KEEPLIN_ENCRYPTION_PASSWORD="…"   # enables AES-256-GCM at-rest encryption
export KEEPLIN_KEY_SALT="…"              # required (same on every device) for encrypted multi-device sync
export KEEPLIN_AUTH_USERNAME="alice"
export KEEPLIN_AUTH_PASSWORD="…"
```

### Run

```bash
./target/release/keeplin-daemon --config keeplin.toml
```

The daemon serves `KeeplinService` on `grpc_addr` and shuts down cleanly on `Ctrl‑C`.

---

## Configuration reference

All fields live in `keeplin.toml`; the four secrets can be overridden by the environment
variables shown.

| Field | Default | Description |
|-------|---------|-------------|
| `mode` | `offline` | `offline` (filesystem) or `server` (database + WebSocket). |
| `data_dir` | `./keeplin-data` | Root directory for files, or location of the `.db`. |
| `server_url` | `""` | WebSocket sync URL (server mode). Use `wss://` in production. |
| `auth_token` | `""` | Bearer token sent on the first WebSocket frame (server mode). |
| `grpc_addr` | `127.0.0.1:50051` | gRPC listen address. |
| `http_addr` | `none` | Optional HTTP listen address for the REST/JSON API + WebSocket feed (e.g. `127.0.0.1:50052`). Plain HTTP — front with a TLS proxy. Same Basic‑Auth credentials apply. |
| `tls_cert_path` / `tls_key_path` | `none` | PEM cert/key; set both to enable TLS. |
| `max_message_size` | 32 MiB | Max gRPC message size (in/out). |
| `journal_retention_days` | `30` | Days of change‑journal history to keep; pruned after each sync (`0` disables; no‑op for the filesystem backend). |
| `encryption_password` | `none` | Enables at‑rest encryption. Env: `KEEPLIN_ENCRYPTION_PASSWORD`. |
| `key_salt` | `none` (→ device ID) | Argon2id salt (≥ 8 bytes); set the **same** value on all synced devices for portable encryption. Env: `KEEPLIN_KEY_SALT`. |
| `auth_username` / `auth_password` | `none` | gRPC Basic Auth; when both are set, every call must authenticate. Env: `KEEPLIN_AUTH_USERNAME` / `KEEPLIN_AUTH_PASSWORD`. |

The daemon logs a loud warning if it binds a non‑loopback address without auth, or if
encryption is on without `key_salt`.

---

## gRPC API

The service is defined in
[`keeplin-daemon/proto/keeplin.proto`](keeplin-daemon/proto/keeplin.proto). `KeeplinService`
provides CRUD + paginated list RPCs for **notes, notebooks, tags, and resources**, the
note↔tag association RPCs, and a server‑streaming **`Sync`** RPC that reports progress
through one sync cycle. Authentication is HTTP Basic Auth via the `authorization` metadata
header: `Basic base64(user:password)`.

---

## REST API

When `http_addr` is set, the daemon also serves a REST/JSON API on that address, sharing the
same storage backend and the same Basic‑Auth credentials as gRPC. Requests and responses are
JSON over the domain models; authenticate with the standard `Authorization: Basic
base64(user:password)` header (only required when `auth_username`/`auth_password` are set).

| Method & path | Purpose |
|---------------|---------|
| `GET /api/health` | Liveness probe (`200 ok`). |
| `GET /api/notes?page_size=&page_token=` | List notes (cursor pagination → `{ items, next_page_token }`). |
| `POST /api/notes` | Create a note. |
| `GET/PUT/DELETE /api/notes/:id` | Read / update / soft‑delete a note. |
| `GET /api/notes/:id/tags` | List a note's tags. |
| `PUT/DELETE /api/notes/:note_id/tags/:tag_id` | Add / remove a note↔tag association. |
| `GET/POST /api/notebooks`, `GET/PUT/DELETE /api/notebooks/:id` | Notebook CRUD. |
| `GET/POST /api/tags`, `GET/PUT/DELETE /api/tags/:id` | Tag CRUD. |
| `GET/POST /api/resources`, `GET/PUT/DELETE /api/resources/:id` | Resource metadata CRUD. |
| `GET /api/resources/:id/data` | Download the raw resource bytes. |

Resource upload is a raw request body: `POST /api/resources?title=&file_name=` with the
file bytes as the body and the `Content-Type` header as the MIME type. Reads of a
soft‑deleted note, notebook, or tag return `404` (the gRPC `Get` RPCs still return the
tombstone for sync). Errors map to `404` (not found), `422` (corrupted data), `400`
(invalid UUID/body), and `500` otherwise.

The HTTP listener is **plain HTTP** — terminate TLS at a reverse proxy in production, exactly
as for the WebSocket sync token.

---

## Development

```bash
cargo test --workspace                              # full test suite
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

The suite includes unit tests for the version‑vector merge, integration tests for both
backends and the encryption layer, two‑device convergence tests, and an **end‑to‑end
WebSocket sync test** (`keeplin-core/tests/ws_sync.rs`) that stands up an in‑process relay.
CI (`.github/workflows/ci.yml`) runs check, test, clippy, and `cargo audit`.

---

## Security

See [`SECURITY.md`](SECURITY.md) for the encryption scheme, threat model, the per‑backend
conflict‑resolution difference, and known limitations (plaintext WebSocket token without
TLS, unsupported mixed‑backend sync, last‑write‑wins trade‑offs).

---

## Project status

**Ready** for single‑user, self‑hosted **offline** use (`FsBackend` + Syncthing): the
filesystem note model is well‑tested and converges deterministically.

**Not yet production‑ready** as a multi‑user, server‑backed service. Outstanding work,
roughly in priority order:

1. **No production sync server** ships in this repo — server mode needs a real relay
   (the WebSocket path is now covered end‑to‑end by a test‑only relay).
2. `DbBackend` resolves note conflicts by last‑write‑wins (no version‑vector merge).
3. Operability: metrics, health checks, and a schema/format **migration path**.
4. Performance at scale: filesystem reads materialize from logs (no compaction yet).
5. Hardening: `wss://`/TLS by default, chunked upload for large attachments.

---

## License

Licensed under the [MIT License](LICENSE).
