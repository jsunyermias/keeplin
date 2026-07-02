# `Cargo.toml` — keeplin-relay

## Crate purpose

`keeplin-relay` is the binary (and thin library) crate that implements Keeplin's server-mode
**WebSocket sync hub**: it authenticates device connections and broadcasts each device's change
frames to the other connected devices. It is intentionally independent of `keeplin-core` at
runtime — it moves opaque frames and never interprets them — so it depends only on an async
runtime, a WebSocket implementation, JSON (for the auth handshake), and argument parsing.

## Runtime dependencies

| Crate | Version | Why |
|-------|---------|-----|
| `tokio` | workspace | Async runtime, TCP listener, broadcast channel, Ctrl-C shutdown |
| `tokio-tungstenite` | 0.24 | Server-side WebSocket accept + framing |
| `futures-util` | 0.3 | `SinkExt`/`StreamExt` for the split read/write halves |
| `serde` | workspace | Present for `serde_json` derive support |
| `serde_json` | workspace | Parse the `{"type":"auth","token":…}` handshake frame |
| `tracing` | workspace | Structured connection/lifecycle logging |
| `tracing-subscriber` | workspace | Log formatting + `RUST_LOG` filter |
| `anyhow` | workspace | Error propagation in `serve`/`main` |
| `clap` | 4 (`derive`, `env`) | `--listen` / `--auth-token` (with `KEEPLIN_RELAY_TOKEN` env) |
| `subtle` | 2 | Constant-time token comparison to avoid a timing side-channel |

## Dev dependencies

| Crate | Version | Why |
|-------|---------|-----|
| `keeplin-core` | (path) | The end-to-end tests drive real `DbBackend` instances through the relay |
| `tempfile` | workspace | Temp databases for the two test devices |
| `uuid` | workspace | Note ids in the tests |
| `chrono` | workspace | The `epoch()` watermark passed to `get_changes_since` |

## Layout

- `src/lib.rs` — `serve(listener, config, shutdown)` and the connection/auth/broadcast logic,
  factored as a library so `tests/relay.rs` can run the real relay in-process on an ephemeral
  port.
- `src/main.rs` — CLI parsing, logging setup, `TcpListener` bind, and Ctrl-C shutdown.
- `tests/relay.rs` — end-to-end tests over a real socket (sync round-trip + auth rejection).

## Related files

- `keeplin-relay/README.md` — deployment, wire protocol, and the no-persistence trade-off.
- `keeplin-core/src/storage/db.rs` — the `DbBackend` client whose protocol this relay serves.
- `keeplin-core/tests/ws_sync.rs` — the in-process test relay this crate productionises.
