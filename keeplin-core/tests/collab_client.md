# `tests/collab_client.rs` — collaborative client tests (state machine + mock server e2e)

## What is tested

Two layers of the collaborative client (`src/collab/`):

1. **Pure state machine** (`collab::state::NoteLines`): diffing a note body into line ops and
   replaying ops deterministically — no I/O.
2. **End-to-end against a mock keeplin-srv**: an in-process axum stand-in serving the REST
   note listing, the blob endpoints, and a `/api/ws` Join/Welcome/Op relay, driven by two
   `CollabBackend<DbBackend>` stacks speaking the genuine protocol. (The *production* server
   end of the same protocol is exercised in keeplin-srv's `tests/collab_client_e2e.rs`.)

## Test cases

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `token_device_id_decodes` | forged unsigned JWT | `device_id_from_token` extracts the claim; garbage → `None` |
| `diff_roundtrip_materializes_new_body` | diff body v1→v2 into ops, apply to mirror | mirror materialises v2 |
| `ops_replay_identically_on_another_mirror` | replay one diff's ops elsewhere | both mirrors converge byte-identically |
| `created_note_body_survives_the_join_welcome` | create note locally, then a (late, empty) `Welcome` arrives | pending-push reconcile: the local body is **not** clobbered |
| `edits_travel_between_two_daemons` | two clients joined to one note | an edit on A materialises on B via the op relay |
| `resource_blob_uploads_out_of_band_and_downloads_on_read` | `create_resource` through the collab stack | binary is `PUT` to `/api/resources/:id/data` (never inline in the relayed `Change`); `read_resource` fetches it back |
| `cursor_updates_flow_into_presence` | `send_cursor` then `presence()` | the server's presence broadcast is readable through `CollabHandle` |

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `token_for(device)` | unsigned JWT with a `device_id` claim (client decodes, never verifies) |
| `mock_server(note_id)` | axum mock of keeplin-srv: `GET /api/notes` (one note), blob PUT/GET, `/api/ws` Join→Welcome + op fan-out |
| `client(addr, device)` | offline `DbBackend` wrapped in `CollabBackend`, `start`ed with itself as stack top (`start` is `Result` since the `/version` handshake; the mock has no `/version`, which is the warn-and-continue path) |
| `wait_body` | poll a client's local note body until convergence (~5 s bound) |

## Related files

- `../src/collab/mod.rs` — the decorator under test (push/apply/suppression logic).
- `../src/collab/state.rs` — the `NoteLines` diff/apply state machine.
- `../src/compat.rs` — the `/version` handshake `start` now runs (dedicated tests in `tests/version_handshake.rs`).
- keeplin-srv `tests/collab.rs` — the server side of the same wire protocol.
