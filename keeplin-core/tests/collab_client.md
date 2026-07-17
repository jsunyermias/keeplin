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

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `client()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `wait_body()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `token_for()` — defined here (EXTRACTED; file-local)
- `token_device_id_decodes()` — defined here (EXTRACTED; file-local)
- `diff_roundtrip_materializes_new_body()` — defined here (EXTRACTED; file-local)
- `ops_replay_identically_on_another_mirror()` — defined here (EXTRACTED; file-local)
- `mock_server()` — defined here (EXTRACTED; file-local)
- `created_note_body_survives_the_join_welcome()` — defined here (EXTRACTED; file-local)
- `edits_travel_between_two_daemons()` — defined here (EXTRACTED; file-local)
- `resource_blob_uploads_out_of_band_and_downloads_on_read()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/collab/mod.rs` — client of the keeplin-srv collaborative channel (EXTRACTED: references×2; e.g. `CollabBackend`)
- `keeplin-core/src/collab/state.rs` — client line state and body↔lines translation (EXTRACTED: imports_from×1; e.g. `NoteLines`)
- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: references×2; e.g. `DbBackend`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- The state-machine layer is tested pure (no I/O); the e2e layer runs against an in-process mock keeplin-srv — the production server end is covered in keeplin-srv's own e2e suite.
- Deterministic replay (same ops → identical bodies on every mirror) is the core assertion and must stay covered.
- The mock has no `/version` route: `start` exercising the warn-and-continue handshake path is intentional.

## Related files

- `../src/collab/mod.rs` — the decorator under test (push/apply/suppression logic).
- `../src/collab/state.rs` — the `NoteLines` diff/apply state machine.
- `../src/compat.rs` — the `/version` handshake `start` now runs (dedicated tests in `tests/version_handshake.rs`).
- keeplin-srv `tests/collab.rs` — the server side of the same wire protocol.
