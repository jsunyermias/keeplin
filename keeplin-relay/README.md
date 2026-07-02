# keeplin-relay

The **server-mode sync hub** for [Keeplin](../README.md). In server mode each device's
`DbBackend` opens a WebSocket to a relay, pushes its local changes, and receives the changes
other devices pushed. `keeplin-relay` is that relay.

It is a **broadcast hub**: it authenticates each connection, then forwards every change batch a
device sends to **all other** connected devices, never echoing it back to the sender. It is the
shippable counterpart to the in-process test relay in `keeplin-core/tests/ws_sync.rs` — same
wire protocol, plus a real auth check, configuration, and graceful shutdown.

## What it does (and does not)

- **Does:** authenticate connections against a shared token (constant-time compare), fan out
  `changes` frames between devices, run many concurrent connections, and shut down cleanly on
  Ctrl-C.
- **Does not:** persist anything. A device that is offline misses whatever was broadcast while
  it was gone and catches up the next time both peers are online together. Because every change
  is idempotent and version-vector resolved, replaying them converges — so no data is lost as
  long as devices reconnect. **Durable per-device buffering** (re-delivering to a
  long-offline device on its own schedule) is a deliberate non-goal of this relay; it would add
  a store, retention, and per-device cursors, and can be layered on later.

## Wire protocol

Matches `DbBackend`'s client exactly:

1. The client's **first** text frame is the auth handshake:
   `{"type":"auth","token":"…"}`. The relay checks the token and closes the connection on
   mismatch. An **empty** configured token disables the check (development only).
2. Every **subsequent** text frame (a `{"type":"changes",…}` batch) is forwarded verbatim to
   all other authenticated clients.

The relay never parses or trusts the `changes` payloads — it only moves bytes. Conflict
resolution and at-rest encryption stay entirely in the clients.

## Running

```bash
cargo run -p keeplin-relay -- --listen 0.0.0.0:9000
# token via env (preferred — not visible in the process list):
KEEPLIN_RELAY_TOKEN="a-long-random-secret" cargo run -p keeplin-relay -- --listen 0.0.0.0:9000
```

Point each device's `keeplin.toml` at it:

```toml
mode       = "server"
server_url = "wss://relay.example.com/"   # see TLS below
auth_token = "a-long-random-secret"        # or the KEEPLIN_AUTH_* / env path
```

| Flag | Env | Default | Meaning |
|------|-----|---------|---------|
| `--listen` | — | `127.0.0.1:9000` | Address to accept device WebSocket connections on |
| `--auth-token` | `KEEPLIN_RELAY_TOKEN` | `""` | Shared secret every device must present; empty disables auth (dev only) |

## TLS

The relay speaks plain `ws://`. **Terminate TLS at a reverse proxy** (nginx, Caddy, …) and
point devices at `wss://your-proxy` — the auth token and change payloads must not cross the
network in the clear. This mirrors the daemon's own posture (plain HTTP REST behind a proxy),
and the daemon **refuses** a non-loopback `ws://` `server_url` for exactly this reason (see the
daemon's `insecure` config and `SECURITY.md`). Native `wss://` termination in the relay itself
is a possible follow-up.

## Tests

`cargo test -p keeplin-relay` runs unit tests for the token check and auth-frame parsing, plus
end-to-end integration tests (`tests/relay.rs`) that stand the real relay up on an ephemeral
port and drive two genuine `DbBackend` instances through it: a note created on one device
arrives on the other, and a device presenting the wrong token is rejected and receives nothing.
