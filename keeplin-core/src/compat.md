# `compat.rs` — keeplin-srv protocol/capability handshake (`GET /version`)

## Purpose

The **single place in this repository** that defines which keeplin-srv wire protocol this
client speaks and how the startup handshake against `GET /version` is classified. keeplin and
keeplin-srv evolve in separate repositories (keeplin-srv pins a keeplin-core git `rev`), so a
protocol drift would otherwise fail silently or with confusing mid-sync errors. Both connect
points use this module: `DbBackend::new` (relay, `storage/db.rs`) and `CollabBackend::start`
(collaborative channel, `collab/mod.rs`). keeplin-srv mirrors the rule around its own
`PROTOCOL_VERSION` constant in its `src/http.rs`.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `PROTOCOL_VERSION` | `pub const u32` | the sync/collab protocol this client speaks (currently `1`). Bump together with keeplin-srv's constant on a breaking wire change. |
| `ServerInfo` | struct | deserialised `GET /version` body: `name`, `version`, `protocol_version` (required), `capabilities` (defaulted). Unknown fields ignored. |
| `Handshake` | enum | `Compatible(ServerInfo)` / `Incompatible(ServerInfo)` / `Unavailable` |

## Public API

| Function | Description |
|----------|-------------|
| `compatible_with(server_protocol) -> bool` | **The** compatibility rule: exact equality with `PROTOCOL_VERSION`. Capabilities cover additive evolution, so a version bump is reserved for breaking changes — hence equality, not a range. |
| `negotiate(http, http_base) -> Handshake` | `GET {base}/version`. Never errors: non-2xx, network failure, or unparseable JSON (including a missing `protocol_version`) → `Unavailable`. |
| `incompatible_message(&ServerInfo) -> String` | The actionable startup error: names both protocol versions and which side to upgrade (server newer → upgrade this client/daemon; client newer → bump keeplin-srv's pinned keeplin-core `rev` / upgrade keeplin-srv). |

## The three-way contract (identical at both connect points)

- **Compatible** → log negotiated protocol + capabilities, proceed (and `DbBackend` primes
  its capability cache from the reply).
- **Incompatible** → fail loudly at startup (`StorageError::InvalidState` with
  `incompatible_message`); **no sync is attempted**.
- **Unavailable** → warn and continue — an old keeplin-srv without `/version`, or a bare test
  relay, must keep working exactly as before the handshake existed (backward compatible).

## Design notes

- `negotiate` is infallible by design: only a *well-formed, incompatible* answer may block
  startup. Anything ambiguous degrades to the pre-handshake behaviour.
- The version-bump procedure is documented in both repos' READMEs: to adopt a newer
  keeplin-core, bump the pinned `rev` in keeplin-srv's `Cargo.toml` and run its test suite —
  it exercises this real client against the real server.

## Related files

- `storage/db.rs` — handshake enforced in `DbBackend::new` (relay connect path).
- `collab/mod.rs` — handshake enforced in `CollabBackend::start` (session start).
- `tests/version_handshake.rs` — fake `/version` servers asserting the three behaviours.
- keeplin-srv `src/http.rs` — the mirrored `PROTOCOL_VERSION` + `compatible_with` and the `/version` endpoint itself.
