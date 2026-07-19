# `auth.rs` — shared HTTP Basic Authentication check

Self-contained companion for `keeplin-daemon/src/auth.rs`. It documents **every code
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
use base64::{engine::general_purpose::STANDARD, Engine};
use subtle::ConstantTimeEq;
```

**What it does** — The single shared credential comparison for HTTP Basic auth:
both the gRPC interceptor (`main.rs`) and the REST/WebSocket middleware
(`rest.rs`) authenticate requests through it, so the check lives in exactly one
place. The comparison is **constant-time** (`subtle::ConstantTimeEq`) and
evaluates username and password **unconditionally** — no `&&`/`||`
short-circuit — so response timing cannot reveal whether the username alone was
correct.

**Dependencies** — `base64`, `subtle`.

**Used by** — `main.rs` (gRPC interceptor), `rest.rs::auth_mw`.

**Repeated context** — daemon auth is local Basic auth over the daemon's own
API; it is unrelated to keeplin-srv's account/JWT auth, which only the collab
client (`CollabConfig.token`) speaks.

---

## fn verify_basic

**Identification** —
`pub fn verify_basic(header: Option<&str>, expected_user: &str, expected_pass: &str) -> bool`;
marker `// md:fn verify_basic`.

**What it does** — Verifies a raw `Authorization` header value (e.g.
`"Basic dXNlcjpwYXNz"`; `None` = absent) against the expected credentials.
Rules, in order:

1. **Empty expected credentials always fail** — they can never be a valid,
   intentional configuration (they would make `Basic Og==`, the base64 of
   `":"`, authenticate as anyone). Startup validation
   (`Config::validate_auth`) rejects them too, but this guard means no caller
   path can enable a credential-less bypass (issue #73).
2. Absent header → false.
3. RFC 7617/7235 parsing: the scheme is **case-insensitive** and any amount of
   whitespace may separate it from the token, so the header is split on
   whitespace — `basic x`, `BASIC  x` accepted (issue #76); a base64 token has
   no internal whitespace, so a third token means malformed → false.
4. Non-`basic` scheme, undecodable base64, or non-UTF-8 → false.
5. Only the **first** colon separates user from password (RFC 7617 — the
   password may contain colons).
6. Constant-time compare of both parts, combined with bitwise `&`.

**Dependencies** — `base64::STANDARD`, `subtle`.

**Used by** — the gRPC interceptor and the REST middleware (both surfaces must
behave identically).

**Repeated context** — none.

---

## mod tests

**Identification** — `#[cfg(test)]` unit-test module; marker `// md:mod tests`.
One helper + five tests.

**What it does** — Pins the acceptance and every rejection path.

### fn basic

**Identification** — helper `fn basic(user, pass) -> String`; marker
`// md:mod tests > fn basic`.

**What it does** — Builds a `Basic <base64(user:pass)>` header value.

### fn accepts_valid_credentials

**Identification** — unit test; marker
`// md:mod tests > fn accepts_valid_credentials`.

**What it does** — The happy path authenticates.

### fn rejects_wrong_password_user_and_missing_header

**Identification** — unit test; marker
`// md:mod tests > fn rejects_wrong_password_user_and_missing_header`.

**What it does** — Wrong password, wrong user, no header, a `Bearer` scheme,
and non-base64 all fail.

### fn password_with_colons_works

**Identification** — unit test; marker
`// md:mod tests > fn password_with_colons_works`.

**What it does** — `p:a:s:s` round-trips (first-colon split).

### fn rejects_empty_expected_credentials

**Identification** — unit test; marker
`// md:mod tests > fn rejects_empty_expected_credentials`.

**What it does** — `Basic Og==` and every empty-credential combination fail
even against empty expected values (issue #73).

### fn scheme_is_case_and_whitespace_tolerant

**Identification** — unit test; marker
`// md:mod tests > fn scheme_is_case_and_whitespace_tolerant`.

**What it does** — `basic`/`BASIC`, multiple spaces, and surrounding tabs are
accepted (issue #76); a stray third token is rejected.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `verify_basic()` — defined here (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph — external crates only) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/main.rs` — the gRPC auth interceptor (INFERRED)
- `keeplin-daemon/src/rest.rs` — the REST/WebSocket auth middleware (INFERRED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `fn verify_basic` | `// md:fn verify_basic` |
| 3 | `mod tests` (+ helper `basic` + five tests) | `// md:mod tests` (+ `> fn …`) |
