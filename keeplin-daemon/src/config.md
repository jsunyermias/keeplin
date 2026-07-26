# `config.rs` — daemon runtime configuration

Self-contained companion for `keeplin-daemon/src/config.rs`. It documents **every code block of
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

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
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

**Code** — complete and verbatim:

```rust
// md:Mode
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Offline,
    Server,
}
```

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

**Code** — complete and verbatim:

```rust
// md:Config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub mode: Mode,

    pub data_dir: PathBuf,

    #[serde(default)]
    pub server_url: String,

    #[serde(default)]
    pub auth_token: String,

    #[serde(default)]
    pub collab_api_url: Option<String>,

    #[serde(default = "default_grpc_addr")]
    pub grpc_addr: String,

    #[serde(default)]
    pub http_addr: Option<String>,

    #[serde(default)]
    pub tls_cert_path: Option<String>,

    #[serde(default)]
    pub tls_key_path: Option<String>,

    #[serde(default = "default_max_message_size")]
    pub max_message_size: usize,

    #[serde(default = "default_max_upload_bytes")]
    pub max_upload_bytes: usize,

    #[serde(default = "default_journal_retention_days")]
    pub journal_retention_days: u64,

    #[serde(default)]
    pub resource_purge_days: u64,

    #[serde(default)]
    pub sync_interval_secs: u64,

    #[serde(default)]
    pub encryption_password: Option<String>,

    #[serde(default)]
    pub key_salt: Option<String>,

    #[serde(default)]
    pub auth_username: Option<String>,

    #[serde(default)]
    pub auth_password: Option<String>,

    #[serde(default)]
    pub insecure: bool,
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn default_grpc_addr
fn default_grpc_addr() -> String {
    "127.0.0.1:50051".to_string()
}
```

**What it does** — `127.0.0.1:50051` — loopback so a config-less first start
cannot expose an unauthenticated API.

---

## fn default_max_message_size

**Identification** — marker `// md:fn default_max_message_size`.

**Code** — complete and verbatim:

```rust
// md:fn default_max_message_size
fn default_max_message_size() -> usize {
    32 * 1024 * 1024
}
```

**What it does** — 32 MiB, applied to both decode and encode.

---

## fn default_max_upload_bytes

**Identification** — marker `// md:fn default_max_upload_bytes`.

**Code** — complete and verbatim:

```rust
// md:fn default_max_upload_bytes
fn default_max_upload_bytes() -> usize {
    1024 * 1024 * 1024
}
```

**What it does** — 1 GiB — generous for large attachments while bounding the
memory one streamed upload can consume (assembled in memory).

---

## fn default_journal_retention_days

**Identification** — marker `// md:fn default_journal_retention_days`.

**Code** — complete and verbatim:

```rust
// md:fn default_journal_retention_days
fn default_journal_retention_days() -> u64 {
    30
}
```

**What it does** — 30 days — comfortably exceeds normal peer offline time so
pruning does not strand an unsynced device.

---

## impl Config (loading)

**Identification** — the first `impl Config`; marker
`// md:impl Config (loading)`. One method.

**Code** — container: members documented as sub-blocks below: fn from_file.

### fn from_file

**Identification** — `pub fn from_file(path) -> anyhow::Result<Self>`; marker
`// md:impl Config (loading) > fn from_file`.

**Code** — complete and verbatim:

```rust
    // md:impl Config (loading) > fn from_file
    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&raw)?;
        Ok(cfg)
    }
```

**What it does** — Reads and TOML-parses the file; missing optional fields
fall back to serde defaults, so a minimal file with only `data_dir` starts the
daemon offline. Errors on unreadable file or malformed TOML.

**Used by** — `main.rs` startup and the `migrate` subcommand.

---

## impl Default for Config

**Identification** — marker `// md:impl Default for Config`.

**Code** — complete and verbatim:

```rust
// md:impl Default for Config
impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Offline,
            data_dir: PathBuf::from("./keeplin-data"),
            server_url: String::new(),
            auth_token: String::new(),
            collab_api_url: None,
            grpc_addr: default_grpc_addr(),
            http_addr: None,
            tls_cert_path: None,
            tls_key_path: None,
            max_message_size: default_max_message_size(),
            max_upload_bytes: default_max_upload_bytes(),
            journal_retention_days: default_journal_retention_days(),
            resource_purge_days: 0,
            sync_interval_secs: 0,
            encryption_password: None,
            key_salt: None,
            auth_username: None,
            auth_password: None,
            insecure: false,
        }
    }
}
```

**What it does** — The all-defaults config (offline, `./keeplin-data`,
loopback gRPC, no HTTP/TLS/auth/encryption, retention 30, purge/sync-interval
off, `insecure = false`).

**Used by** — tests; documentation of the defaults.

---

## impl Config (security)

**Identification** — the second `impl Config`; marker
`// md:impl Config (security)`. Three methods.

**Code** — container: members documented as sub-blocks below: fn security_issues, fn auth_enabled, fn validate_auth.

### fn security_issues

**Identification** — `pub fn security_issues(&self) -> Vec<String>`; marker
`// md:impl Config (security) > fn security_issues`.

**Code** — complete and verbatim:

```rust
    // md:impl Config (security) > fn security_issues
    pub fn security_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        let auth = self.auth_enabled();

        if !auth {
            if let Ok(addr) = self.grpc_addr.parse::<SocketAddr>() {
                if !addr.ip().is_loopback() {
                    issues.push(format!(
                        "grpc_addr ({addr}) is reachable from the network but no auth is \
                         configured — set auth_username + auth_password (or KEEPLIN_AUTH_*)"
                    ));
                }
            }
            if let Some(http) = &self.http_addr {
                if let Ok(addr) = http.parse::<SocketAddr>() {
                    if !addr.ip().is_loopback() {
                        issues.push(format!(
                            "http_addr ({addr}) is reachable from the network but no auth is \
                             configured — set auth_username + auth_password (or KEEPLIN_AUTH_*)"
                        ));
                    }
                }
            }
        }

        if matches!(self.mode, Mode::Server) {
            if let Some(host) = plaintext_ws_remote_host(&self.server_url) {
                issues.push(format!(
                    "server_url uses plaintext ws:// to a non-loopback host ({host}), leaking \
                     the auth_token in transit — use wss:// (TLS)"
                ));
            }
        }

        issues
    }
```

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

**Code** — complete and verbatim:

```rust
    // md:impl Config (security) > fn auth_enabled
    pub fn auth_enabled(&self) -> bool {
        matches!(
            (self.auth_username.as_deref(), self.auth_password.as_deref()),
            (Some(u), Some(p)) if !u.is_empty() && !p.is_empty()
        )
    }
```

**What it does** — Auth is **active** only when both credentials are set and
non-empty — a half-configured or empty pair is *not* active (and is rejected
by `validate_auth`), so this never reports a store as protected when requests
would pass unauthenticated.

**Used by** — `security_issues`, `main.rs` (whether to install the
interceptors).

### fn validate_auth

**Identification** — `pub fn validate_auth(&self) -> Result<(), String>`;
marker `// md:impl Config (security) > fn validate_auth`.

**Code** — complete and verbatim:

```rust
    // md:impl Config (security) > fn validate_auth
    pub fn validate_auth(&self) -> Result<(), String> {
        match (self.auth_username.as_deref(), self.auth_password.as_deref()) {
            (None, None) => Ok(()),
            (Some(_), None) => Err(
                "auth_username is set but auth_password is not — set both (or neither); \
                 prefer KEEPLIN_AUTH_USERNAME + KEEPLIN_AUTH_PASSWORD"
                    .into(),
            ),
            (None, Some(_)) => Err(
                "auth_password is set but auth_username is not — set both (or neither); \
                 prefer KEEPLIN_AUTH_USERNAME + KEEPLIN_AUTH_PASSWORD"
                    .into(),
            ),
            (Some(u), Some(p)) => {
                if u.is_empty() || p.is_empty() {
                    Err(
                        "auth_username and auth_password must both be non-empty when \
                         authentication is configured"
                            .into(),
                    )
                } else {
                    Ok(())
                }
            }
        }
    }
```

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

**Code** — complete and verbatim:

```rust
// md:fn plaintext_ws_remote_host
fn plaintext_ws_remote_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("ws://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = match authority.strip_prefix('[') {
        Some(after) => after.split(']').next().unwrap_or(after),
        None => authority.rsplit_once(':').map_or(authority, |(h, _)| h),
    };
    let is_loopback = matches!(host, "localhost" | "127.0.0.1" | "::1")
        || host.starts_with("127.")
        || host.eq_ignore_ascii_case("ip6-localhost");
    if is_loopback {
        None
    } else {
        Some(host)
    }
}
```

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

**Code** — container: members documented as sub-blocks below: fn base, fn with_auth, fn loopback_defaults_are_safe, fn network_grpc_without_auth_is_flagged, fn network_http_without_auth_is_flagged, fn validate_auth_rejects_partial_and_empty_credentials, fn partial_auth_still_flags_network_exposure, fn plaintext_ws_to_remote_is_flagged_in_server_mode, fn plaintext_ws_remote_host_parsing, fn multiple_issues_accumulate.

**What it does** — Pins the security-check and auth-validation matrices.

The explicit `imports` leaf below preserves the test-module dependency preamble
verbatim.

### imports

**Identification** — test-module dependencies; marker `// md:mod tests > imports`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > imports
    use super::*;
```

**What it does** — Brings the parent module API and test-only dependencies into
scope.

**Dependencies** —

- `super::*` — the items under test from the parent module; expects: the parent keeps them at module scope; a rename or a move into a submodule breaks these tests at compile time, which is the intended early signal.

**Used by** — every block of `mod tests` in this file: `fn base`, `fn with_auth`, `fn loopback_defaults_are_safe`, `fn network_grpc_without_auth_is_flagged`, `fn network_http_without_auth_is_flagged`, `fn validate_auth_rejects_partial_and_empty_credentials`, `fn partial_auth_still_flags_network_exposure`, `fn plaintext_ws_to_remote_is_flagged_in_server_mode`, `fn plaintext_ws_remote_host_parsing`, `fn multiple_issues_accumulate`. Nothing outside the module can use it: the preamble is private to `mod tests`.

**Repeated context** — This preamble is a leaf block, not scaffolding: only the `mod` declaration, its attributes and its braces are exempt from coverage, so these `use` lines carry their own marker and are verified verbatim against the source (template v2.5.0, RULE 6). Changing an import here without updating this fence fails `scripts/check-docs.sh`.

### fn base

**Identification** — helper; marker `// md:mod tests > fn base`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn base
    fn base() -> Config {
        Config::default()
    }
```

**What it does** — `Config::default()` (a safe loopback config).

### fn with_auth

**Identification** — helper; marker `// md:mod tests > fn with_auth`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn with_auth
    fn with_auth(mut c: Config) -> Config {
        c.auth_username = Some("alice".into());
        c.auth_password = Some("s3cr3t".into());
        c
    }
```

**What it does** — Adds a full credential pair.

### fn loopback_defaults_are_safe

**Identification** — marker `// md:mod tests > fn loopback_defaults_are_safe`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn loopback_defaults_are_safe
    #[test]
    fn loopback_defaults_are_safe() {
        assert!(base().security_issues().is_empty());
    }
```

**What it does** — The default config has no issues.

### fn network_grpc_without_auth_is_flagged

**Identification** — marker
`// md:mod tests > fn network_grpc_without_auth_is_flagged`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn network_grpc_without_auth_is_flagged
    #[test]
    fn network_grpc_without_auth_is_flagged() {
        let mut c = base();
        c.grpc_addr = "0.0.0.0:50051".into();
        let issues = c.security_issues();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("grpc_addr"));

        assert!(with_auth(c).security_issues().is_empty());
    }
```

**What it does** — `0.0.0.0` gRPC without auth → one issue; adding auth
clears it.

### fn network_http_without_auth_is_flagged

**Identification** — marker
`// md:mod tests > fn network_http_without_auth_is_flagged`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn network_http_without_auth_is_flagged
    #[test]
    fn network_http_without_auth_is_flagged() {
        let mut c = base();
        c.http_addr = Some("0.0.0.0:50052".into());
        let issues = c.security_issues();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("http_addr"));
        assert!(with_auth(c).security_issues().is_empty());
    }
```

**What it does** — Same for `http_addr`.

### fn validate_auth_rejects_partial_and_empty_credentials

**Identification** — marker
`// md:mod tests > fn validate_auth_rejects_partial_and_empty_credentials`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn validate_auth_rejects_partial_and_empty_credentials
    #[test]
    fn validate_auth_rejects_partial_and_empty_credentials() {
        let mut c = base();
        assert!(c.validate_auth().is_ok());
        assert!(!c.auth_enabled());

        c.auth_username = Some("alice".into());
        assert!(c.validate_auth().is_err());
        assert!(!c.auth_enabled());

        c.auth_username = None;
        c.auth_password = Some("s3cr3t".into());
        assert!(c.validate_auth().is_err());
        assert!(!c.auth_enabled());

        c.auth_username = Some(String::new());
        c.auth_password = Some(String::new());
        assert!(c.validate_auth().is_err());
        assert!(!c.auth_enabled());

        c.auth_username = Some("alice".into());
        c.auth_password = Some(String::new());
        assert!(c.validate_auth().is_err());
        assert!(!c.auth_enabled());

        c.auth_password = Some("s3cr3t".into());
        assert!(c.validate_auth().is_ok());
        assert!(c.auth_enabled());
    }
```

**What it does** — The full matrix: both unset ok/off; one set rejected;
both-empty rejected; one-empty rejected; both non-empty ok/enabled.

### fn partial_auth_still_flags_network_exposure

**Identification** — marker
`// md:mod tests > fn partial_auth_still_flags_network_exposure`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn partial_auth_still_flags_network_exposure
    #[test]
    fn partial_auth_still_flags_network_exposure() {
        let mut c = base();
        c.grpc_addr = "0.0.0.0:50051".into();
        c.auth_password = Some("s3cr3t".into());
        let issues = c.security_issues();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("grpc_addr"));
    }
```

**What it does** — A half-configured credential is not "auth enabled" for the
exposure check — the network listener is still flagged (defence in depth).

### fn plaintext_ws_to_remote_is_flagged_in_server_mode

**Identification** — marker
`// md:mod tests > fn plaintext_ws_to_remote_is_flagged_in_server_mode`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn plaintext_ws_to_remote_is_flagged_in_server_mode
    #[test]
    fn plaintext_ws_to_remote_is_flagged_in_server_mode() {
        let mut c = with_auth(base());
        c.mode = Mode::Server;
        c.server_url = "ws://sync.example.com:9000/ws".into();
        let issues = c.security_issues();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("server_url"));

        c.server_url = "wss://sync.example.com:9000/ws".into();
        assert!(c.security_issues().is_empty());

        c.server_url = "ws://127.0.0.1:9000/ws".into();
        assert!(c.security_issues().is_empty());

        c.mode = Mode::Offline;
        c.server_url = "ws://sync.example.com:9000/ws".into();
        assert!(c.security_issues().is_empty());
    }
```

**What it does** — Remote `ws://` in server mode flagged; `wss://` safe;
loopback `ws://` safe; the same URL in offline mode ignored.

### fn plaintext_ws_remote_host_parsing

**Identification** — marker
`// md:mod tests > fn plaintext_ws_remote_host_parsing`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn plaintext_ws_remote_host_parsing
    #[test]
    fn plaintext_ws_remote_host_parsing() {
        assert_eq!(
            plaintext_ws_remote_host("ws://example.com:9000/ws"),
            Some("example.com")
        );
        assert_eq!(
            plaintext_ws_remote_host("ws://example.com"),
            Some("example.com")
        );
        assert_eq!(
            plaintext_ws_remote_host("ws://[2001:db8::1]:80/x"),
            Some("2001:db8::1")
        );
        assert_eq!(plaintext_ws_remote_host("wss://example.com/ws"), None);
        assert_eq!(plaintext_ws_remote_host(""), None);
        assert_eq!(plaintext_ws_remote_host("ws://localhost:9000"), None);
        assert_eq!(plaintext_ws_remote_host("ws://127.0.0.1:9000"), None);
        assert_eq!(plaintext_ws_remote_host("ws://[::1]:9000"), None);
    }
```

**What it does** — Host extraction incl. IPv6 literals; every safe case →
`None`.

### fn multiple_issues_accumulate

**Identification** — marker
`// md:mod tests > fn multiple_issues_accumulate`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn multiple_issues_accumulate
    #[test]
    fn multiple_issues_accumulate() {
        let mut c = base();
        c.grpc_addr = "0.0.0.0:50051".into();
        c.http_addr = Some("0.0.0.0:50052".into());
        c.mode = Mode::Server;
        c.server_url = "ws://sync.example.com/ws".into();
        assert_eq!(c.security_issues().len(), 3, "{:?}", c.security_issues());
    }
```

**What it does** — Network gRPC + HTTP + remote `ws://`, no auth → three
issues.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

<!-- Data source: CI artifact or local graphify-out/graph.json from this exact commit.
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. Never
     present inference as fact. -->
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
| 1 | `Overview` | `// md:Overview` |
| 2 | `Mode` | `// md:Mode` |
| 3 | `Config` | `// md:Config` |
| 4 | `fn default_grpc_addr` | `// md:fn default_grpc_addr` |
| 5 | `fn default_max_message_size` | `// md:fn default_max_message_size` |
| 6 | `fn default_max_upload_bytes` | `// md:fn default_max_upload_bytes` |
| 7 | `fn default_journal_retention_days` | `// md:fn default_journal_retention_days` |
| 8 | `impl Config (loading)` (container) | `// md:impl Config (loading)` |
| 9 | `fn from_file` | `// md:impl Config (loading) > fn from_file` |
| 10 | `impl Default for Config` | `// md:impl Default for Config` |
| 11 | `impl Config (security)` (container) | `// md:impl Config (security)` |
| 12 | `fn security_issues` | `// md:impl Config (security) > fn security_issues` |
| 13 | `fn auth_enabled` | `// md:impl Config (security) > fn auth_enabled` |
| 14 | `fn validate_auth` | `// md:impl Config (security) > fn validate_auth` |
| 15 | `fn plaintext_ws_remote_host` | `// md:fn plaintext_ws_remote_host` |
| 16 | `mod tests` (container) | `// md:mod tests` |
| 17 | `imports` | `// md:mod tests > imports` |
| 18 | `fn base` | `// md:mod tests > fn base` |
| 19 | `fn with_auth` | `// md:mod tests > fn with_auth` |
| 20 | `fn loopback_defaults_are_safe` | `// md:mod tests > fn loopback_defaults_are_safe` |
| 21 | `fn network_grpc_without_auth_is_flagged` | `// md:mod tests > fn network_grpc_without_auth_is_flagged` |
| 22 | `fn network_http_without_auth_is_flagged` | `// md:mod tests > fn network_http_without_auth_is_flagged` |
| 23 | `fn validate_auth_rejects_partial_and_empty_credentials` | `// md:mod tests > fn validate_auth_rejects_partial_and_empty_credentials` |
| 24 | `fn partial_auth_still_flags_network_exposure` | `// md:mod tests > fn partial_auth_still_flags_network_exposure` |
| 25 | `fn plaintext_ws_to_remote_is_flagged_in_server_mode` | `// md:mod tests > fn plaintext_ws_to_remote_is_flagged_in_server_mode` |
| 26 | `fn plaintext_ws_remote_host_parsing` | `// md:mod tests > fn plaintext_ws_remote_host_parsing` |
| 27 | `fn multiple_issues_accumulate` | `// md:mod tests > fn multiple_issues_accumulate` |
