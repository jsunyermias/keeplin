# keeplin-relay

The **server-mode sync hub** for [Keeplin](../README.md). In server mode each device's
`DbBackend` opens a WebSocket to a relay, pushes its local changes, and receives the changes
other devices pushed. `keeplin-relay` is that relay.

It is a **broadcast hub with a durable buffer**: it authenticates each connection, forwards
every change batch a device sends to **all other** connected devices (never echoing it back to
the sender), and journals every frame so a device that was offline is **caught up on
reconnect** with everything it missed.

## What it does (and does not)

- **Does:** authenticate connections against a shared token (constant-time compare), fan out
  `changes` frames between devices, journal every frame durably (`relay.log` + a per-device
  delivery cursor under `cursors/`), replay missed frames to a reconnecting device, run many
  concurrent connections, and shut down cleanly on Ctrl-C. The journal is compacted hourly,
  dropping frames older than the retention window; sequence numbers stay monotonic across
  compactions and restarts, so cursors never go backwards. Because every change is idempotent
  and version-vector resolved, over-delivery is always safe.
- **Does not:** parse, validate, or store the *content* of change batches (it moves opaque
  bytes), or terminate TLS (see below). A device offline **longer than the retention window**
  may miss dropped frames — size `--retention-days` above the longest expected offline period.
  With `--ephemeral` the relay keeps no state at all (the old pure-broadcast behavior).

## Wire protocol

Matches `DbBackend`'s client exactly:

1. The client's **first** text frame is the auth handshake:
   `{"type":"auth","token":"…","device_id":"…"}`. The relay checks the token and closes the
   connection on mismatch. An **empty** configured token disables the check (development
   only). `device_id` is optional: presenting one (as `DbBackend` does) enables catch-up
   delivery — buffered frames from other devices past this device's cursor are replayed
   before live forwarding starts. A client without one gets plain live broadcast.
2. Every **subsequent** text frame (a `{"type":"changes",…}` batch) is journaled and
   forwarded verbatim to all other authenticated clients.

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
| `--data-dir` | `KEEPLIN_RELAY_DATA_DIR` | `./keeplin-relay-data` | Directory for the durable frame journal and per-device cursors |
| `--retention-days` | `KEEPLIN_RELAY_RETENTION_DAYS` | `30` | Days of buffered frames to keep (`0` = forever); size above the longest expected device offline period |
| `--ephemeral` | — | off | Disable the durable buffer entirely (offline devices miss frames) |

## TLS

The relay speaks plain `ws://`. **Terminate TLS at a reverse proxy** (nginx, Caddy, …) and
point devices at `wss://your-proxy` — the auth token and change payloads must not cross the
network in the clear. This mirrors the daemon's own posture (plain HTTP REST behind a proxy),
and the daemon **refuses** a non-loopback `ws://` `server_url` for exactly this reason (see the
daemon's `insecure` config and `SECURITY.md`). Native `wss://` termination in the relay itself
is a possible follow-up.

## Tests

`cargo test -p keeplin-relay` runs unit tests for the token check, auth-frame parsing, and
device-id sanitisation, plus end-to-end integration tests (`tests/relay.rs`): two genuine
`DbBackend` instances syncing a note through the relay, auth rejection, and the durable
buffer — an offline device catching up on reconnect, no re-delivery once the cursor advanced,
the journal surviving a relay restart, and a legacy (device-id-less) client getting live
broadcast only.
