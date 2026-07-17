# `tests/version_handshake.rs` — startup protocol handshake tests

## What is tested

The `GET /version` protocol handshake (`src/compat.rs`) as wired into the two connect points:
`DbBackend::new` (relay) and `CollabBackend::start` (collaborative session). Fake keeplin-srv
`/version` endpoints (in-process axum servers on ephemeral ports) drive the three contractual
client behaviours: compatible → connect, incompatible → loud actionable failure with no sync,
missing → warn and continue (backward compatible with pre-`/version` servers).

## Test cases

| Test function | Fake server | Expected client behaviour |
|---------------|-------------|---------------------------|
| `compatible_version_connects_and_primes_capabilities` | `/version` with `protocol_version = PROTOCOL_VERSION`, capabilities `["history"]`, hit counter | construction succeeds; `/version` fetched exactly **once** — a later history read uses the capability cache primed at startup, no refetch |
| `incompatible_version_fails_construction_loudly` | `protocol_version = PROTOCOL_VERSION + 7` | `DbBackend::new` fails; message contains "incompatible", both protocol numbers, and "upgrade" (actionable: which side to bump) |
| `missing_version_warns_and_continues` | no `/version` route (404) | construction succeeds; local CRUD fully usable (offline-capable client unchanged) |
| `collab_start_applies_the_same_rule` | incompatible → `CollabBackend::start` returns `Err` (connection task never spawned); missing → `Ok` | the same three-way rule holds at the collab session start |

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `spawn_version_server(protocol_version, hits)` | axum server: optional `/version` (counts hits) + a canned empty `/api/notes/:id/history`, so a primed capability cache can be exercised |
| `fake_token()` | JWT-shaped token with a `device_id` claim (`CollabBackend::new` parses it unverified; only the server verifies signatures) |
| `db_path()` | fresh LibSQL path in a leaked tempdir |

## Related files

- `../src/compat.rs` — `PROTOCOL_VERSION`, `compatible_with`, `negotiate`, `incompatible_message`.
- `../src/storage/db.rs` — the relay-side enforcement in `DbBackend::new`.
- `../src/collab/mod.rs` — the collab-side enforcement in `CollabBackend::start`.
- keeplin-srv `tests/integration.rs` — the real-server end of the same wire contract.
