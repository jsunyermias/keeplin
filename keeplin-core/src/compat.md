# `compat.rs` — keeplin-srv protocol/capability handshake (`GET /version`)

Self-contained companion for `keeplin-core/src/compat.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be able
to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the import. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use serde::Deserialize;
```

**What it does** — The client side of the keeplin-srv protocol/capability handshake
(`GET /version`; issues keeplin-srv#39 / keeplin#114), and the **single place in this
repository** that defines which server protocol this client speaks. keeplin and
keeplin-srv evolve in separate repositories (keeplin-srv pins a keeplin-core git
`rev`), so a wire-protocol drift would otherwise fail silently or with confusing
mid-sync errors; keeplin-srv mirrors the same rule around its own `PROTOCOL_VERSION`
constant in its `src/http.rs`. The three-way contract, applied identically at both
connect points (`DbBackend::new` for the relay, `CollabBackend::start` for the
collaborative channel):

- `/version` answers a **compatible** `protocol_version` → log negotiated version +
  capabilities and proceed (DbBackend primes its capability cache from the reply).
- `/version` answers an **incompatible** `protocol_version` → fail loudly at startup
  (`StorageError::InvalidState` with `incompatible_message`); sync is not attempted.
- `/version` is **missing/unreachable/unparseable** (older keeplin-srv, or a bare
  fake relay in tests) → warn and continue; behaviour unchanged from before the
  handshake existed (backward compatible).

**Dependencies** — `serde` (deserialising `ServerInfo`); `reqwest` appears in
`negotiate`'s signature.

**Used by** — `storage/db.rs` (`DbBackend::new`), `collab/mod.rs`
(`CollabBackend::start`), `keeplin-core/tests/version_handshake.rs`.

**Repeated context** — Version-bump procedure (documented in both repos' READMEs):
to adopt a newer keeplin-core, bump the pinned `rev` in keeplin-srv's `Cargo.toml`
and run its test suite — it exercises this real client against the real server.

---

## PROTOCOL_VERSION

**Identification** — `pub const PROTOCOL_VERSION: u32 = 1;` marker
`// md:PROTOCOL_VERSION`.

**Code** — complete and verbatim:

```rust
// md:PROTOCOL_VERSION
pub const PROTOCOL_VERSION: u32 = 1;
```

**What it does** — The sync/collab wire-protocol version this client speaks.
Mirrors keeplin-srv's `PROTOCOL_VERSION` (its `src/http.rs`); bump **both sides
together** on any breaking change to the relay or collab message shapes.

**Dependencies** — none.

**Used by** — `compatible_with`, `incompatible_message`,
`tests/version_handshake.rs` (drives its fake servers).

**Repeated context** — Project premise: clean breaks, no migrations — a breaking
wire change is expressed only as a version bump, never as dual-format support.

---

## fn compatible_with

**Identification** — `pub fn compatible_with(server_protocol: u32) -> bool`; marker
`// md:fn compatible_with`.

**Code** — complete and verbatim:

```rust
// md:fn compatible_with
pub fn compatible_with(server_protocol: u32) -> bool {
    server_protocol == PROTOCOL_VERSION
}
```

**What it does** — The compatibility rule, in one place: **exact protocol match**
(`server_protocol == PROTOCOL_VERSION`). Capabilities cover additive evolution (a
client probes them instead of guessing), so a `protocol_version` bump is reserved
for breaking changes — hence equality, not a range.

**Dependencies** — `PROTOCOL_VERSION`.

**Used by** — `negotiate` (classification), unit test `exact_match_is_compatible`.

**Repeated context** — keeplin-srv applies the identical equality check on its side
of the handshake; both must change in lockstep.

---

## ServerInfo

**Identification** — struct deriving `Debug, Clone, Deserialize`; marker
`// md:ServerInfo`.

**Code** — complete and verbatim:

```rust
// md:ServerInfo
#[derive(Debug, Clone, Deserialize)]
pub struct ServerInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub protocol_version: u32,
    #[serde(default)]
    pub capabilities: Vec<String>,
}
```

**What it does** — What `GET /version` advertises: `name` and `version`
(`#[serde(default)]` — informational, may be absent/empty), `protocol_version`
(required `u32` — a body missing it fails deserialisation and classifies as
`Unavailable`), `capabilities` (defaulted `Vec<String>` of additive feature flags a
client probes, e.g. collab support). Unknown fields are ignored (serde default), so
an older client keeps working against a newer server's additions.

**Dependencies** — `serde`.

**Used by** — `negotiate` (deserialisation target), `Handshake`'s payload,
`incompatible_message`, callers logging capabilities.

**Repeated context** — additive evolution goes in `capabilities`; breaking
evolution goes in `protocol_version` — never infer features from `version` strings.

---

## Handshake

**Identification** — enum deriving `Debug, Clone`; marker `// md:Handshake`.

**Code** — complete and verbatim:

```rust
// md:Handshake
#[derive(Debug, Clone)]
pub enum Handshake {
    Compatible(ServerInfo),
    Incompatible(ServerInfo),
    Unavailable,
}
```

**What it does** — Outcome of the startup handshake:
`Compatible(ServerInfo)` (server speaks our protocol; capabilities known),
`Incompatible(ServerInfo)` (server answered with a protocol we do not speak — the
caller must fail startup), `Unavailable` (no usable `/version`: older server,
unreachable, or not an HTTP endpoint at all — the caller warns and continues).

**Dependencies** — `ServerInfo`.

**Used by** — returned by `negotiate`; matched in `storage/db.rs` and
`collab/mod.rs`.

**Repeated context** — only `Incompatible` may block startup; ambiguity always
degrades to the pre-handshake behaviour.

---

## fn negotiate

**Identification** —
`pub async fn negotiate(http: &reqwest::Client, http_base: &str) -> Handshake`;
marker `// md:fn negotiate`.

**Code** — complete and verbatim:

```rust
// md:fn negotiate
pub async fn negotiate(http: &reqwest::Client, http_base: &str) -> Handshake {
    let url = format!("{}/version", http_base.trim_end_matches('/'));
    let response = match http.get(&url).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return Handshake::Unavailable,
    };
    match response.json::<ServerInfo>().await {
        Ok(info) if compatible_with(info.protocol_version) => Handshake::Compatible(info),
        Ok(info) => Handshake::Incompatible(info),
        Err(_) => Handshake::Unavailable,
    }
}
```

**What it does** — Fetches `GET {http_base}/version` (trailing `/` on the base is
trimmed before joining) and classifies the answer. **Never errors**: non-2xx status
or a network failure → `Unavailable`; a 2xx body that parses as `ServerInfo` →
`Compatible`/`Incompatible` per `compatible_with`; unparseable JSON →
`Unavailable`. Anything short of a well-formed reply must leave the pre-handshake
behaviour intact against old servers and test relays.

**Dependencies** — `reqwest` (caller supplies the client — connection pooling and
TLS config stay the caller's concern), `ServerInfo`, `compatible_with`.

**Used by** — `DbBackend::new` (relay connect), `CollabBackend::start` (collab
session start).

**Repeated context** — async convention of the crate: all I/O is `async` on tokio;
pure logic (like `compatible_with`) stays sync and unit-testable.

---

## fn incompatible_message

**Identification** — `pub fn incompatible_message(info: &ServerInfo) -> String`;
marker `// md:fn incompatible_message`.

**Code** — complete and verbatim:

```rust
// md:fn incompatible_message
pub fn incompatible_message(info: &ServerInfo) -> String {
    let direction = if info.protocol_version > PROTOCOL_VERSION {
        "The server is newer: upgrade this keeplin client/daemon to a release \
         that speaks the server's protocol."
    } else {
        "The client is newer: upgrade keeplin-srv (its Cargo.toml pins a \
         keeplin-core rev; bump it to a matching release and run its test \
         suite), or downgrade this client."
    };
    format!(
        "incompatible sync server: {} {} speaks protocol {} but this client speaks \
         protocol {}. {} Sync is disabled until the versions match.",
        if info.name.is_empty() {
            "server"
        } else {
            &info.name
        },
        if info.version.is_empty() {
            "(unknown version)"
        } else {
            &info.version
        },
        info.protocol_version,
        PROTOCOL_VERSION,
        direction
    )
}
```

**What it does** — Builds the actionable startup error for an incompatible server:
names the server (falling back to `"server"` / `"(unknown version)"` for empty
fields), states both protocol versions, and says **which side to upgrade** — server
newer (`info.protocol_version > PROTOCOL_VERSION`) → upgrade this keeplin
client/daemon; client newer → upgrade keeplin-srv (bump its pinned keeplin-core
`rev`, run its test suite) or downgrade this client. Ends with "Sync is disabled
until the versions match."

**Dependencies** — `ServerInfo`, `PROTOCOL_VERSION`.

**Used by** — the failure paths in `storage/db.rs` and `collab/mod.rs`; unit test
`incompatible_message_names_the_side_to_upgrade`.

**Repeated context** — error-message convention: operator-facing errors must be
actionable (say what to do), and must never contain sensitive data.

---

## mod tests

**Identification** — `#[cfg(test)]` unit-test module; marker `// md:mod tests`.
Two tests.

**Code** — container: members documented as sub-blocks below: fn exact_match_is_compatible, fn incompatible_message_names_the_side_to_upgrade.

**What it does** — Unit tests for the pure pieces (the equality rule and the
message direction); the network path is covered end-to-end by
`tests/version_handshake.rs` with fake HTTP servers.

**Dependencies** — `super::*`.

**Used by** — CI (`cargo test --workspace`).

**Repeated context** — project test convention: pure logic in in-file
`#[cfg(test)]` tests; anything needing sockets in `keeplin-core/tests/`.

### fn exact_match_is_compatible

**Identification** — unit test; marker
`// md:mod tests > fn exact_match_is_compatible`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn exact_match_is_compatible
    #[test]
    fn exact_match_is_compatible() {
        assert!(compatible_with(PROTOCOL_VERSION));
        assert!(!compatible_with(PROTOCOL_VERSION + 1));
        assert!(!compatible_with(0));
    }
```

**What it does** — Asserts `compatible_with(PROTOCOL_VERSION)` is true and both a
higher (`+1`) and lower (`0`) version are rejected.

**Dependencies** — `compatible_with`, `PROTOCOL_VERSION`.

**Used by** — CI only.

**Repeated context** — none.

### fn incompatible_message_names_the_side_to_upgrade

**Identification** — unit test; marker
`// md:mod tests > fn incompatible_message_names_the_side_to_upgrade`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn incompatible_message_names_the_side_to_upgrade
    #[test]
    fn incompatible_message_names_the_side_to_upgrade() {
        let newer_server = ServerInfo {
            name: "keeplin-srv".into(),
            version: "9.9.9".into(),
            protocol_version: PROTOCOL_VERSION + 1,
            capabilities: vec![],
        };
        let msg = incompatible_message(&newer_server);
        assert!(msg.contains("upgrade this keeplin client"), "{msg}");

        let older_server = ServerInfo {
            name: "keeplin-srv".into(),
            version: "0.0.1".into(),
            protocol_version: 0,
            capabilities: vec![],
        };
        let msg = incompatible_message(&older_server);
        assert!(msg.contains("upgrade keeplin-srv"), "{msg}");
    }
```

**What it does** — Builds a newer-server `ServerInfo` (`PROTOCOL_VERSION + 1`) and
asserts the message says to upgrade this keeplin client; builds an older-server one
(`0`) and asserts it says to upgrade keeplin-srv.

**Dependencies** — `incompatible_message`, `ServerInfo`.

**Used by** — CI only.

**Repeated context** — none.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `compatible_with()` — defined here (EXTRACTED; file-local)
- `ServerInfo` — defined here (EXTRACTED; file-local)
- `Handshake` — defined here (EXTRACTED; file-local)
- `negotiate()` — defined here (EXTRACTED; file-local)
- `incompatible_message()` — defined here (EXTRACTED; file-local)
- `exact_match_is_compatible()` — defined here (EXTRACTED; file-local)
- `incompatible_message_names_the_side_to_upgrade()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph — the callers use fully-qualified `crate::compat::…` paths the AST pass does not link) (EXTRACTED)
- `keeplin-core/src/storage/db.rs` — `DbBackend::new` calls `negotiate`/`incompatible_message` for the relay-side startup handshake (INFERRED)
- `keeplin-core/src/collab/mod.rs` — `CollabBackend::start` calls the same pair for the collab session handshake (INFERRED)
- `keeplin-core/tests/version_handshake.rs` — imports `PROTOCOL_VERSION` to drive the three fake-server behaviours (INFERRED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `PROTOCOL_VERSION` | `// md:PROTOCOL_VERSION` |
| 3 | `fn compatible_with` | `// md:fn compatible_with` |
| 4 | `struct ServerInfo` | `// md:ServerInfo` |
| 5 | `enum Handshake` | `// md:Handshake` |
| 6 | `fn negotiate` | `// md:fn negotiate` |
| 7 | `fn incompatible_message` | `// md:fn incompatible_message` |
| 8 | `mod tests` | `// md:mod tests` |
| 9 | `fn exact_match_is_compatible` | `// md:mod tests > fn exact_match_is_compatible` |
| 10 | `fn incompatible_message_names_the_side_to_upgrade` | `// md:mod tests > fn incompatible_message_names_the_side_to_upgrade` |
