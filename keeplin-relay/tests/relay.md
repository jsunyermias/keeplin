# `tests/relay.rs` — keeplin-relay end-to-end tests

## What is tested

End-to-end tests that stand the **real** relay up on an ephemeral TCP port and drive traffic
through it over a genuine socket — no mocking of the transport. Two kinds of client are used:
real `DbBackend` instances (which exercise the full server-mode sync path), and a thin raw
WebSocket helper (`raw_client`) that lets the durable-buffer tests control device ids, frame
contents, and connect/disconnect timing directly.

## Test cases

### Broadcast & auth

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `note_created_on_one_device_reaches_another_through_the_relay` | Two `DbBackend`s; A creates a note and pushes; B drains sync | B converges on A's note through the relay |
| `a_device_with_the_wrong_token_cannot_sync` | Receiver presents the wrong token | Its connection is closed; it receives nothing |

### Durable buffer

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `offline_device_catches_up_from_the_durable_buffer` | A sends while B is offline; B connects later | B is replayed the missed frame; A's own frame never echoes back to it, even on reconnect |
| `delivery_cursor_prevents_redelivery` | B receives a frame, advances its cursor, reconnects | No re-delivery on the second connection |
| `buffer_survives_a_relay_restart` | A sends; relay is stopped and a fresh one started on the same `data_dir` | B still catches up from the persisted journal |
| `a_client_without_device_id_gets_live_broadcast_only` | A device-id-less (legacy) client connects | No catch-up replay, but live frames still arrive |

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `spawn_relay(token)` | Start an **ephemeral** relay (no buffer), leak the shutdown sender so it runs for the test |
| `spawn_relay_with(token, data_dir)` | Start a relay (durable when `data_dir` is `Some`); returns URL + shutdown trigger + task handle for restart tests |
| `raw_client(url, token, device)` | An authenticated raw WebSocket client, optionally presenting a device id |
| `recv_text` / `send_text` | Receive-with-timeout / send one text frame |
| `device(url, token)` | A server-mode `DbBackend` connected to the relay (temp dir leaked to outlive the db) |
| `sync_until_present` | Poll `receive_changes` + `apply_change` until a note id shows up |

## Coverage gaps

- Retention-window compaction dropping old frames is covered by the `Buffer::compact` unit
  path, not re-exercised here (it would require injecting timestamps or sleeping days).
- TLS is out of scope: the relay speaks plain `ws://` by design and is fronted by a proxy.

## Related files

- `keeplin-relay/src/lib.rs` — the relay under test (protocol + durable buffer).
- `keeplin-core/tests/ws_sync.rs` — the in-process test relay covering the client side.
