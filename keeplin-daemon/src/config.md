# `src/config.rs` — daemon configuration

## Purpose

This module defines the `Config` struct that controls every aspect of the
`keeplin-daemon` runtime: storage mode, data directory, gRPC address, TLS certificates,
authentication credentials, and encryption settings. Configuration is loaded from a TOML
file (default: `keeplin.toml`) and may be partially overridden by environment variables.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `Config` | struct | All runtime configuration knobs for `keeplin-daemon` |
| `Mode` | enum | Selects between `offline` (filesystem) and `server` (database + WebSocket) |

## `Config` fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `mode` | `Mode` | `Offline` | Storage backend to use |
| `data_dir` | `PathBuf` | `./keeplin-data` | Root directory for file storage (offline) or location of the `.db` file (server) |
| `server_url` | `String` | `""` | WebSocket URL of the sync server (server mode only) |
| `auth_token` | `String` | `""` | Bearer token sent on first WebSocket connection (server mode only) |
| `grpc_addr` | `String` | `127.0.0.1:50051` | Address and port on which the gRPC server listens |
| `tls_cert_path` | `Option<String>` | `None` | Filesystem path to the PEM-encoded TLS certificate |
| `tls_key_path` | `Option<String>` | `None` | Filesystem path to the PEM-encoded TLS private key |
| `max_message_size` | `usize` | 33,554,432 (32 MiB) | Maximum size of a single gRPC message (both inbound and outbound) |
| `encryption_password` | `Option<String>` | `None` | Passphrase for AES-256-GCM at-rest encryption; prefer env var |
| `auth_username` | `Option<String>` | `None` | Username for HTTP Basic Auth on every gRPC call; prefer env var |
| `auth_password` | `Option<String>` | `None` | Password for HTTP Basic Auth on every gRPC call; prefer env var |

## `Mode` variants

| Variant | TOML value | Description |
|---------|-----------|-------------|
| `Offline` | `"offline"` | Uses `FsBackend`; no network connection required |
| `Server` | `"server"` | Uses `DbBackend`; requires `server_url` to be set |

## Public API

### `Config::from_file(path: impl AsRef<Path>) -> anyhow::Result<Self>`
**What it does:** Reads the file at `path`, parses it as TOML, and deserialises it into
a `Config`. Missing optional fields receive their defaults via `serde(default)` attributes.  
**Parameters:** `path` — path to the TOML configuration file.  
**Returns:** A fully-populated `Config`.  
**Errors:** `anyhow::Error` if the file cannot be read or the TOML is malformed.

## Environment variable overrides

The following environment variables override the corresponding config file fields. They
are applied in `main.rs` after loading the file so that secrets never need to be stored
on disk.

| Environment variable | Field overridden |
|----------------------|-----------------|
| `KEEPLIN_ENCRYPTION_PASSWORD` | `encryption_password` |
| `KEEPLIN_AUTH_PASSWORD` | `auth_password` |
| `KEEPLIN_AUTH_USERNAME` | `auth_username` |

## TLS behaviour

TLS is enabled when **both** `tls_cert_path` and `tls_key_path` are non-empty. If either
is absent, the gRPC server starts without TLS (plaintext). For production deployments
exposed to a network, TLS should always be enabled.

## Design notes

- `Default::default()` on `Config` produces a usable offline configuration pointing to
  `./keeplin-data` and listening on `127.0.0.1:50051`. This is the configuration used
  when no config file is present.
- `max_message_size` defaults to 32 MiB because many PDF and image files that users
  attach as resources fall within this limit, avoiding the need for manual tuning.

## Related files

- `keeplin-daemon/src/main.rs` — reads `Config`, applies env var overrides, and uses it
  to construct the backend and start the server
- `SECURITY.md` — guidance on credential management
