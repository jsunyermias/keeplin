# `lib.rs` — keeplin-relay, the server-mode sync hub

## Purpose

`keeplin-relay` is the central peer for **server mode**. Each device's `DbBackend` opens a
WebSocket to the relay, pushes its local change batches, and receives the batches other
devices pushed. The relay is a **broadcast hub with a durable buffer**: it authenticates each
connection, fans every `changes` frame out to all *other* connected devices, and journals
every frame so a device that was offline is **caught up on reconnect** with everything it
missed. It never parses or trusts the change payloads — it only moves opaque bytes; conflict
resolution and encryption stay entirely in the clients.

This file is the crate's library (`serve` + the buffer); `main.rs` is the thin binary that
parses flags and calls `serve`.

## Key types & functions

| Item | Description |
|------|-------------|
| `serve(listener, config, shutdown)` | Run the relay on a bound `TcpListener` until `shutdown` resolves |
| `RelayConfig` | `auth_token`, optional `data_dir` (buffer; `None` = ephemeral), `retention_days` |
| `Buffer` | The durable journal + per-device delivery cursors |
| `Fanout` | One broadcast-bus message: `{conn, seq, sender_device, text}` |
| `BufferedFrame` | One persisted journal entry: `{seq, sender, ts, frame}` |
| `handle_connection` | Auth handshake → catch-up replay → live bridge for one client |
| `parse_auth` / `sanitize_device_id` | Parse the first frame; validate the client-supplied device id |
| `token_ok` | Constant-time token comparison |

## Wire protocol & durable buffer

**Wire protocol** (matches `DbBackend::connect_ws` / `send_changes` / `receive_changes`):

1. The client's **first** text frame is the auth handshake
   `{"type":"auth","token":"…","device_id":"…"}`. The token is checked in constant time; a
   mismatch closes the connection. `device_id` is **optional** — a client that presents one
   gets catch-up delivery, a client without one gets plain live broadcast (the pre-buffer
   behaviour).
2. Every **subsequent** text frame is journaled and forwarded verbatim to all other
   authenticated clients.

**Durable buffer** (`data_dir` set, the binary default):

- `relay.log` — an append-only NDJSON journal, one line per broadcast frame, each carrying a
  monotonic `seq`, the sender's device id, a timestamp, and the raw frame.
- `cursors/{device_id}` — the last `seq` delivered to each known device, advanced (atomically)
  as frames are delivered live or replayed.

On connect, a device with a known cursor is **replayed** every buffered frame past its cursor
(skipping its own) before live forwarding starts; the cursor then tracks the live stream.
An hourly task compacts the journal, dropping frames older than `retention_days`; `seq` stays
monotonic across compactions and restarts, so cursors never go backwards.

### Invariants & edge cases

- **Delivery is exactly-once per device in the normal case, safely at-least-once otherwise.**
  A dropped connection or a lagged subscriber re-delivers from the cursor on reconnect; every
  change is idempotent and version-vector resolved, so over-delivery converges.
- **A frame is never buffered-and-lost.** If the journal append fails, the frame is dropped
  *loudly* (error log) and not broadcast, so the sender's at-least-once retry can succeed once
  the disk recovers — rather than being silently forwarded but absent from the buffer.
- **The client-supplied device id is untrusted.** `sanitize_device_id` accepts only 1–64
  ASCII alphanumerics/hyphens (uuids qualify), so an id can never traverse out of `cursors/`.
  An unusable id degrades to token-only auth (live broadcast), never an error.
- **Retention bounds catch-up.** A device offline longer than `retention_days` may miss
  dropped frames; size the window above the longest expected offline period.

## Design notes

- **Opaque payloads.** The relay deliberately does not deserialize `changes` — keeping it a
  dumb pipe means at-rest encryption and all conflict logic live only in the clients, and the
  relay has nothing sensitive to leak beyond transit metadata.
- **`--ephemeral`** restores the original stateless broadcast (no `data_dir`), useful for
  throwaway setups and the tests that exercise the pure-broadcast path.
- **TLS is terminated by a reverse proxy.** The relay speaks plain `ws://`; front it with
  nginx/Caddy and point devices at `wss://`, exactly as the daemon's REST/token guidance
  recommends (and the daemon *refuses* a non-loopback `ws://` `server_url`).

## Related files

- `keeplin-relay/src/main.rs` — the binary (flags, env, Ctrl-C shutdown).
- `keeplin-relay/README.md` — operator-facing run/flags/TLS guide.
- `keeplin-core/src/storage/db.rs` — the `DbBackend` client that speaks this protocol
  (`connect_ws` sends the `device_id`).
- `keeplin-core/tests/ws_sync.rs` — the in-process test relay the client is also validated
  against.
