# `config.rs` — daemon runtime configuration

Self-contained companion for `keeplin-daemon/src/config.rs`. It documents **every code
block of the source file, in source order** — a reader with only this file must be able
to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

```rust
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
```

**What it does** — Runtime configuration for the `keeplin-daemon` binary:
`Config` (deserialised from a TOML file at startup) and `Mode` (local
filesystem storage vs server-backed LibSQL). Sensitive fields can be overridden
by environment variables (`KEEPLIN_ENCRYPTION_PASSWORD`, `KEEPLIN_KEY_SALT`,
`KEEPLIN_AUTH_USERNAME`, `KEEPLIN_AUTH_PASSWORD` — applied in `main.rs`) so
they never need to sit in the TOML file.

**Dependencies** — `serde`, `toml` (in `from_file`), `std::net`/`path`.

**Used by** — `main.rs` (loads, env-overrides, validates, and builds the stack
from it); `migrate` subcommand builds one per side.

**Repeated context** — security posture: the daemon **refuses to start** on
unambiguous exposure (`security_issues`) unless `insecure = true`; auth
half-configuration is a hard startup failure (`validate_auth`, issue #73).

---

## Mode

**Identification** — enum deriving `Debug, Clone, Default, Serialize,
Deserialize` with `#[serde(rename_all = "lowercase")]`; marker `// md:Mode`.

**What it does** — The storage mode: `Offline` (default — `FsBackend`;
Syncthing replicates, no server needed) or `Server` (`DbBackend` — local
LibSQL + WebSocket sync; `server_url`/`auth_token` required). Serialises as
`"offline"`/`"server"`.

**Dependencies** — `serde`. **Used by** — `Config.mode`,
`main.rs::build_storage`. **Repeated context** — none.

---

## Config

**Identification** — struct deriving `Debug, Clone, Serialize, Deserialize`;
marker `// md:Config`.

**What it does** — Every daemon setting:

| Field | Default | Meaning |
|-------|---------|---------|
| `mode` | `offline` | storage mode |
| `data_dir` | (required) | offline storage root / DB directory |
| `server_url` | `""` | relay WebSocket URL (server mode) |
| `auth_token` | `""` | relay/server bearer token (server mode) |
| `collab_api_url` | `None` | keeplin-srv HTTP base; when set in server mode, note bodies edit collaboratively over `/api/ws` and note changes stop flowing through the relay (notebooks/tags/resources still sync over `server_url`); same `auth_token` |
| `grpc_addr` | `127.0.0.1:50051` | gRPC listener (loopback by default — no accidental network exposure) |
| `http_addr` | `None` | optional REST/WebSocket-feed listener (plain HTTP — terminate TLS at a proxy); same Basic-Auth credentials |
| `tls_cert_path`/`tls_key_path` | `None` | daemon-terminated TLS for gRPC (both or neither) |
| `max_message_size` | 32 MiB | gRPC message cap (covers typical PDFs/images) |
| `max_upload_bytes` | 1 GiB | cap on an assembled **streamed** upload (gRPC `UploadResource` / `POST /api/resources/upload`) — streams aren't bounded by `max_message_size`; `0` = unlimited (not recommended shared) |
| `journal_retention_days` | 30 | prune `entity_changes` rows older than this after each successful sync (no-op on FS); keep larger than the longest peer offline window; `0` disables |
| `resource_purge_days` | 0 (off) | reclaim payload bytes of tombstones older than this after each sync; metadata always kept |
| `sync_interval_secs` | 0 | automatic sync cadence; `0` = frontend-driven only (issue #111); the collab channel is independent and always live |
| `encryption_password` | `None` | at-rest AES-256-GCM (Argon2id); prefer the env var |
| `key_salt` | `None` | Argon2id salt (≥ 8 bytes; not secret but must match across synced devices for portable encryption; unset → this device's id, single-device only) |
| `auth_username`/`auth_password` | `None` | Basic auth for gRPC *and* REST; active only when both set and non-empty; partial/empty pairs rejected at startup |
| `insecure` | `false` | downgrade the startup security checks from errors to warnings (only when another layer protects: isolated network, mTLS mesh, auth-enforcing proxy) |

**Dependencies** — `serde`, the default fns.

**Used by** — `main.rs` everywhere.

**Repeated context** — none.

---

## fn default_grpc_addr

**Identification** — marker `// md:fn default_grpc_addr`.

**What it does** — `127.0.0.1:50051` — loopback so a config-less first start
cannot expose an unauthenticated API.

---

## fn default_max_message_size

**Identification** — marker `// md:fn default_max_message_size`.

**What it does** — 32 MiB, applied to both decode and encode.

---

## fn default_max_upload_bytes

**Identification** — marker `// md:fn default_max_upload_bytes`.

**What it does** — 1 GiB — generous for large attachments while bounding the
memory one streamed upload can consume (assembled in memory).

---

## fn default_journal_retention_days

**Identification** — marker `// md:fn default_journal_retention_days`.

**What it does** — 30 days — comfortably exceeds normal peer offline time so
pruning does not strand an unsynced device.

---

## impl Config (loading)

**Identification** — the first `impl Config`; marker
`// md:impl Config (loading)`. One method.

### fn from_file

**Identification** — `pub fn from_file(path) -> anyhow::Result<Self>`; marker
`// md:impl Config (loading) > fn from_file`.

**What it does** — Reads and TOML-parses the file; missing optional fields
fall back to serde defaults, so a minimal file with only `data_dir` starts the
daemon offline. Errors on unreadable file or malformed TOML.

**Used by** — `main.rs` startup and the `migrate` subcommand.

---

## impl Default for Config

**Identification** — marker `// md:impl Default for Config`.

**What it does** — The all-defaults config (offline, `./keeplin-data`,
loopback gRPC, no HTTP/TLS/auth/encryption, retention 30, purge/sync-interval
off, `insecure = false`).

**Used by** — tests; documentation of the defaults.

---

## impl Config (security)

**Identification** — the second `impl Config`; marker
`// md:impl Config (security)`. Three methods.

### fn security_issues

**Identification** — `pub fn security_issues(&self) -> Vec<String>`; marker
`// md:impl Config (security) > fn security_issues`.

**What it does** — Enumerates unambiguous exposures (empty = safe to start).
Pure and side-effect-free (unit-testable); `main::serve` refuses to start on a
non-empty result unless `insecure`. Flags only what no fronting TLS proxy can
fix — so the documented reverse-proxy deployment is never blocked:

- a **network-reachable** (non-loopback) gRPC or HTTP listener with **no
  auth** (a proxy cannot invent application credentials);
- in server mode, a **plaintext `ws://` URL to a non-loopback host** — the
  `auth_token` would travel in the clear on the daemon's *outbound*
  connection, where a fronting proxy does not help.

Missing daemon-terminated TLS on the listeners is deliberately **not**
flagged.

**Dependencies** — `auth_enabled`, `plaintext_ws_remote_host`,
`SocketAddr` parsing.

**Used by** — `main.rs::serve`.

### fn auth_enabled

**Identification** — `pub fn auth_enabled(&self) -> bool`; marker
`// md:impl Config (security) > fn auth_enabled`.

**What it does** — Auth is **active** only when both credentials are set and
non-empty — a half-configured or empty pair is *not* active (and is rejected
by `validate_auth`), so this never reports a store as protected when requests
would pass unauthenticated.

**Used by** — `security_issues`, `main.rs` (whether to install the
interceptors).

### fn validate_auth

**Identification** — `pub fn validate_auth(&self) -> Result<(), String>`;
marker `// md:impl Config (security) > fn validate_auth`.

**What it does** — Rejects configurations that would silently disable auth:
exactly one credential set, or either set to an empty string (which would
accept `Basic Og==`). Both unset = intentionally off, valid. Called at
startup: half-configuring (e.g. only `KEEPLIN_AUTH_PASSWORD`) is a hard
failure, not a quietly open daemon (issue #73).

**Used by** — `main.rs` startup.

---

## fn plaintext_ws_remote_host

**Identification** — `fn plaintext_ws_remote_host(url: &str) -> Option<&str>`;
marker `// md:fn plaintext_ws_remote_host`.

**What it does** — Returns the host when `url` is **plaintext `ws://`** to a
**non-loopback** host; `None` for `wss://`, empty URLs, and loopback targets
(`localhost`, `127.*`, `::1`, `ip6-localhost`). Parses the authority
(stripping path/query and an optional port, tolerating a bracketed IPv6
literal). A host not confidently identifiable as loopback is treated as
remote — fail safe: better a spurious warning than a silent token leak.

**Used by** — `security_issues`; its own unit test.

---

## mod tests

**Identification** — `#[cfg(test)] mod tests`; marker `// md:mod tests`. Two
helpers + eight tests, all pure.

**What it does** — Pins the security-check and auth-validation matrices.

### fn base

**Identification** — helper; marker `// md:mod tests > fn base`.

**What it does** — `Config::default()` (a safe loopback config).

### fn with_auth

**Identification** — helper; marker `// md:mod tests > fn with_auth`.

**What it does** — Adds a full credential pair.

### fn loopback_defaults_are_safe

**Identification** — marker `// md:mod tests > fn loopback_defaults_are_safe`.

**What it does** — The default config has no issues.

### fn network_grpc_without_auth_is_flagged

**Identification** — marker
`// md:mod tests > fn network_grpc_without_auth_is_flagged`.

**What it does** — `0.0.0.0` gRPC without auth → one issue; adding auth
clears it.

### fn network_http_without_auth_is_flagged

**Identification** — marker
`// md:mod tests > fn network_http_without_auth_is_flagged`.

**What it does** — Same for `http_addr`.

### fn validate_auth_rejects_partial_and_empty_credentials

**Identification** — marker
`// md:mod tests > fn validate_auth_rejects_partial_and_empty_credentials`.

**What it does** — The full matrix: both unset ok/off; one set rejected;
both-empty rejected; one-empty rejected; both non-empty ok/enabled.

### fn partial_auth_still_flags_network_exposure

**Identification** — marker
`// md:mod tests > fn partial_auth_still_flags_network_exposure`.

**What it does** — A half-configured credential is not "auth enabled" for the
exposure check — the network listener is still flagged (defence in depth).

### fn plaintext_ws_to_remote_is_flagged_in_server_mode

**Identification** — marker
`// md:mod tests > fn plaintext_ws_to_remote_is_flagged_in_server_mode`.

**What it does** — Remote `ws://` in server mode flagged; `wss://` safe;
loopback `ws://` safe; the same URL in offline mode ignored.

### fn plaintext_ws_remote_host_parsing

**Identification** — marker
`// md:mod tests > fn plaintext_ws_remote_host_parsing`.

**What it does** — Host extraction incl. IPv6 literals; every safe case →
`None`.

### fn multiple_issues_accumulate

**Identification** — marker
`// md:mod tests > fn multiple_issues_accumulate`.

**What it does** — Network gRPC + HTTP + remote `ws://`, no auth → three
issues.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `Config` — defined here (EXTRACTED)
- `Mode` — defined here (EXTRACTED)
- `security_issues()`, `auth_enabled()`, `validate_auth()`, `plaintext_ws_remote_host()` — defined here (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph — external crates only) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/main.rs` — loads/validates and builds the stack (INFERRED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `enum Mode` | `// md:Mode` |
| 3 | `struct Config` | `// md:Config` |
| 4–7 | the four default fns | `// md:fn default_*` |
| 8 | first `impl Config` (`from_file`) | `// md:impl Config (loading)` (+ `> fn from_file`) |
| 9 | `impl Default for Config` | `// md:impl Default for Config` |
| 10 | second `impl Config` (`security_issues`, `auth_enabled`, `validate_auth`) | `// md:impl Config (security)` (+ `> fn …`) |
| 11 | `fn plaintext_ws_remote_host` | `// md:fn plaintext_ws_remote_host` |
| 12 | `mod tests` (+ 2 helpers + 8 tests) | `// md:mod tests` (+ `> fn …`) |
