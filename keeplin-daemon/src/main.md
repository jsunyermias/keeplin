# `src/main.rs` — daemon entry point

## Purpose

This is the binary entry point for `keeplin-daemon`. It parses the configuration file and
environment variable overrides, constructs the appropriate storage backend (with optional
encryption), wraps the backend in the gRPC service layer, optionally enables TLS, installs
an HTTP Basic Auth interceptor on every RPC, and starts the gRPC server.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `Args` | struct | Command-line arguments parsed by `clap` |

## Program flow

```
main()
 1. Parse Args (--config flag; default: keeplin.toml)
 2. Load Config from file (or default if file absent)
 3. Apply env var overrides (KEEPLIN_ENCRYPTION_PASSWORD, KEEPLIN_AUTH_PASSWORD, KEEPLIN_AUTH_USERNAME)
 4. Warn if gRPC is exposed to the network without authentication
 5. Construct backend according to (mode, encryption_password):
    ┌─────────────────────┬───────────────────────────────────┐
    │ (Offline, None)     │ FsBackend                         │
    │ (Offline, Some(pw)) │ EncryptedBackend<FsBackend>       │
    │ (Server, None)      │ DbBackend                         │
    │ (Server, Some(pw))  │ EncryptedBackend<DbBackend>       │
    └─────────────────────┴───────────────────────────────────┘
 6. Call run_server(cfg, addr, backend)
```

### `run_server`

1. Creates a `KeeplinServiceServer` wrapping `KeeplinServer<B>`.
2. Sets `max_decoding_message_size` and `max_encoding_message_size` from config.
3. Wraps the service with `InterceptedService` using `validate_basic_auth` as the
   interceptor. The interceptor runs on every RPC call before the handler.
4. Optionally loads TLS certificate and key from the configured paths.
5. Listens for `Ctrl-C` as the shutdown signal (graceful drain).
6. Starts `builder.add_service(svc).serve_with_shutdown(addr, shutdown_signal)`.

## Public functions

### `fn validate_basic_auth(req: tonic::Request<()>, expected_user: Option<&str>, expected_pass: Option<&str>) -> Result<tonic::Request<()>, tonic::Status>`
**What it does:** Validates the `Authorization: Basic <base64(user:pass)>` header on
every incoming gRPC call.  
**Parameters:**
- `req` — the incoming gRPC request (metadata only; `()` body)
- `expected_user` — the configured username, or `None` to skip auth entirely
- `expected_pass` — the configured password, or `None` to skip auth entirely  
**Returns:** The unmodified `req` if authentication succeeds.  
**Errors:** `tonic::Status::Unauthenticated` if the header is missing, malformed, or
the credentials do not match.

**Security:** Credential comparison uses `subtle::ConstantTimeEq` to prevent
timing-based side-channel attacks. Both the username and password are compared in
constant time regardless of where they differ.

If both `expected_user` and `expected_pass` are `None`, the function returns `Ok(req)`
immediately without inspecting the header (authentication is disabled).

## Command-line interface

```
keeplin-daemon [OPTIONS]

Options:
  -c, --config <PATH>  Path to the TOML configuration file [default: keeplin.toml]
  -h, --help           Print help
```

## Environment variables

| Variable | Effect |
|----------|--------|
| `KEEPLIN_ENCRYPTION_PASSWORD` | Sets `cfg.encryption_password`; overrides the config file |
| `KEEPLIN_AUTH_PASSWORD` | Sets `cfg.auth_password`; overrides the config file |
| `KEEPLIN_AUTH_USERNAME` | Sets `cfg.auth_username`; overrides the config file |

Environment variables take precedence over the config file so that secrets do not need
to be stored in plaintext on disk.

## Security warnings

At startup, if the gRPC address is not a loopback address (`127.*` or `::1`) and
authentication is not configured, a `WARN`-level tracing message is emitted. This is a
deliberate reminder that the server is exposed to the network without protection.

## Design notes

- The `run_server` function is generic over `B: StorageBackend`, which means the compiler
  produces one monomorphised copy per backend type. This avoids dynamic dispatch in the
  hot path.
- `#[allow(clippy::result_large_err)]` is on both `run_server` and `validate_basic_auth`
  because they return `Result<_, tonic::Status>` and `tonic::Status` is 176 bytes.
  Wrapping it in a `Box` would add unnecessary heap allocation to every RPC call.
- Shutdown uses `tokio::signal::ctrl_c()` which resolves on SIGINT (`Ctrl-C`). Tonic's
  `serve_with_shutdown` drains existing connections gracefully after the signal arrives.

## Related files

- `keeplin-daemon/src/config.rs` — `Config` and `Mode` types
- `keeplin-daemon/src/server.rs` — `KeeplinServer` implementation
- `keeplin-core/src/storage/fs.rs` — used in offline mode
- `keeplin-core/src/storage/db.rs` — used in server mode
- `keeplin-core/src/encryption.rs` — wraps the backend when a password is configured
- `SECURITY.md` — full security model and credential guidance
