# `tests/ws_sync.rs` — end-to-end WebSocket sync test

## What is tested

The other suites drive `DbBackend` only in **offline** mode. This one stands up a real
(in-process) **WebSocket relay** — a minimal stand-in for the production sync server — and
pushes two `DbBackend` instances through the genuine wire protocol:

1. the `auth` handshake performed on construction,
2. `send_changes` serialising a `changes` envelope and writing it to the socket,
3. the relay forwarding the batch to the **other** device,
4. `receive_changes` draining and parsing the incoming frames.

This proves a change actually travels between two devices **over a socket**, not just through
the local database — closing the gap the offline tests leave open.

## Test cases

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `note_create_syncs_between_two_devices` | Device A creates a note, syncs; B syncs | B ends up with the note |
| `update_propagates_and_converges` | A updates the note, both sync | Both devices converge on the new body |
| `failed_send_keeps_watermark_and_changes_are_resent_after_recovery` | A's relay is down during a sync cycle, then comes back on the same address | The failed cycle errors (`StorageError::WebSocket`) and leaves the last-sync watermark unchanged; the next cycle re-sends the batch and B receives it |
| `send_without_configured_relay_is_a_noop` | `server_url = ""` (no relay configured), `send_changes` with a non-empty batch | `Ok` — a local-only backend skipping the send is not a failure |
| `malformed_frame_does_not_abort_receive` | The relay forwards a garbage text frame followed by a valid `changes` envelope | `receive_changes` skips the garbage with a warning and still delivers the valid batch |

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `spawn_relay()` | Starts the in-process WebSocket relay on an ephemeral port and returns its `SocketAddr`; forwards each device's batch to the others |
| `spawn_relay_on(listener)` | Same relay on an already-bound listener, so a test can reserve an address, fail against it, then bring the relay up there (the relay-recovery scenario) |
| `device(url)` | Builds a `DbBackend` connected to the relay `url` (performs the auth handshake) |
| `push(dev)` | Runs one `send_changes` for a device's pending local changes |
| `sync_until(dev, id, want_body)` | Polls `receive_changes` + `apply_change` until the note reaches the expected state (bounded retries) |

## Related files

- `keeplin-core/src/storage/db.rs` — the WebSocket protocol (`ensure_ws`, `send_changes`, `receive_changes`) under test
- `keeplin-core/tests/sync.rs` — the same convergence proven without a socket
- `README.md` — "end-to-end WebSocket sync test" in the Development section
