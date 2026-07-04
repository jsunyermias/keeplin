# `main.rs` — keeplin-relay entry point

## Purpose

The thin binary wrapper around [`keeplin_relay::serve`](lib.rs). It parses command-line flags
(and their environment overrides), warns about insecure configurations, binds the TCP
listener, and runs the relay until Ctrl-C. All behaviour lives in `lib.rs`; this file is just
process wiring.

## Startup / wiring

1. Initialise `tracing` (`keeplin_relay=info` by default).
2. Parse `Args` (clap), resolving secrets from the environment where offered.
3. Warn if `--auth-token` is empty (the relay would accept **any** client — dev only).
4. Warn if `--ephemeral` (offline devices miss frames sent while away).
5. Warn if listening on a **non-loopback** address in plaintext (front it with a TLS proxy).
6. Bind the `TcpListener` and call `serve(listener, RelayConfig { … }, shutdown_signal())`.

## Configuration / key reference

| Flag | Env | Default | Meaning |
|------|-----|---------|---------|
| `--listen` | — | `127.0.0.1:9000` | Address to accept device WebSocket connections on |
| `--auth-token` | `KEEPLIN_RELAY_TOKEN` | `""` | Shared secret every device presents; empty disables auth (dev only) |
| `--data-dir` | `KEEPLIN_RELAY_DATA_DIR` | `./keeplin-relay-data` | Durable journal + per-device cursors |
| `--retention-days` | `KEEPLIN_RELAY_RETENTION_DAYS` | `30` | Days of buffered frames to keep (`0` = forever) |
| `--ephemeral` | — | off | Disable the durable buffer entirely |

## Notes & gotchas

- **Prefer the env vars for secrets** — a command-line `--auth-token` is visible in the
  process list.
- **`--ephemeral` and `--data-dir` are mutually exclusive in effect**: `--ephemeral` wins and
  passes `data_dir: None` to `serve`.
- Shutdown drains in-flight connections when the `serve` future returns; the compaction task
  is aborted on the way out.

## Related files

- `keeplin-relay/src/lib.rs` — `serve`, the durable buffer, the wire protocol.
- `keeplin-relay/README.md` — the fuller operator guide (TLS, backups, retention sizing).
