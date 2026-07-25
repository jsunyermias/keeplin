# `auth.rs` — shared HTTP Basic Authentication check

Self-contained companion for `keeplin-daemon/src/auth.rs`. It documents **every code block of
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

**Code** — complete and verbatim:

```rust
// md:fn verify_basic
pub fn verify_basic(header: Option<&str>, expected_user: &str, expected_pass: &str) -> bool {
    if expected_user.is_empty() || expected_pass.is_empty() {
        return false;
    }
    let Some(header) = header else {
        return false;
    };
    let mut parts = header.split_whitespace();
    let (Some(scheme), Some(encoded), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("basic") {
        return false;
    }
    let Ok(decoded) = STANDARD.decode(encoded) else {
        return false;
    };
    let Ok(creds) = std::str::from_utf8(&decoded) else {
        return false;
    };
    let Some(colon) = creds.find(':') else {
        return false;
    };
    let (user, pass) = (&creds[..colon], &creds[colon + 1..]);
    let user_ok = user.as_bytes().ct_eq(expected_user.as_bytes());
    let pass_ok = pass.as_bytes().ct_eq(expected_pass.as_bytes());
    (user_ok & pass_ok).unwrap_u8() == 1
}
```

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

**Code** — container: members documented as sub-blocks below: fn basic, fn accepts_valid_credentials, fn rejects_wrong_password_user_and_missing_header, fn password_with_colons_works, fn rejects_empty_expected_credentials, fn scheme_is_case_and_whitespace_tolerant.

**What it does** — Pins the acceptance and every rejection path.

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

**Used by** — every block of `mod tests` in this file: `fn basic`, `fn accepts_valid_credentials`, `fn rejects_wrong_password_user_and_missing_header`, `fn password_with_colons_works`, `fn rejects_empty_expected_credentials`, `fn scheme_is_case_and_whitespace_tolerant`. Nothing outside the module can use it: the preamble is private to `mod tests`.

**Repeated context** — This preamble is a leaf block, not scaffolding: only the `mod` declaration, its attributes and its braces are exempt from coverage, so these `use` lines carry their own marker and are verified verbatim against the source (template v2.5.0, RULE 6). Changing an import here without updating this fence fails `scripts/check-docs.sh`.

### fn basic

**Identification** — helper `fn basic(user, pass) -> String`; marker
`// md:mod tests > fn basic`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn basic
    fn basic(user: &str, pass: &str) -> String {
        format!("Basic {}", STANDARD.encode(format!("{user}:{pass}")))
    }
```

**What it does** — Builds a `Basic <base64(user:pass)>` header value.

### fn accepts_valid_credentials

**Identification** — unit test; marker
`// md:mod tests > fn accepts_valid_credentials`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn accepts_valid_credentials
    #[test]
    fn accepts_valid_credentials() {
        assert!(verify_basic(
            Some(&basic("alice", "s3cr3t")),
            "alice",
            "s3cr3t"
        ));
    }
```

**What it does** — The happy path authenticates.

### fn rejects_wrong_password_user_and_missing_header

**Identification** — unit test; marker
`// md:mod tests > fn rejects_wrong_password_user_and_missing_header`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn rejects_wrong_password_user_and_missing_header
    #[test]
    fn rejects_wrong_password_user_and_missing_header() {
        assert!(!verify_basic(
            Some(&basic("alice", "nope")),
            "alice",
            "s3cr3t"
        ));
        assert!(!verify_basic(
            Some(&basic("mallory", "s3cr3t")),
            "alice",
            "s3cr3t"
        ));
        assert!(!verify_basic(None, "alice", "s3cr3t"));
        assert!(!verify_basic(Some("Bearer xyz"), "alice", "s3cr3t"));
        assert!(!verify_basic(Some("Basic !!!notbase64"), "alice", "s3cr3t"));
    }
```

**What it does** — Wrong password, wrong user, no header, a `Bearer` scheme,
and non-base64 all fail.

### fn password_with_colons_works

**Identification** — unit test; marker
`// md:mod tests > fn password_with_colons_works`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn password_with_colons_works
    #[test]
    fn password_with_colons_works() {
        let pass = "p:a:s:s";
        assert!(verify_basic(Some(&basic("alice", pass)), "alice", pass));
    }
```

**What it does** — `p:a:s:s` round-trips (first-colon split).

### fn rejects_empty_expected_credentials

**Identification** — unit test; marker
`// md:mod tests > fn rejects_empty_expected_credentials`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn rejects_empty_expected_credentials
    #[test]
    fn rejects_empty_expected_credentials() {
        let empty = format!("Basic {}", STANDARD.encode(":"));
        assert!(!verify_basic(Some(&empty), "", ""));
        assert!(!verify_basic(Some(&basic("", "")), "", ""));
        assert!(!verify_basic(Some(&basic("alice", "")), "alice", ""));
        assert!(!verify_basic(Some(&basic("", "s3cr3t")), "", "s3cr3t"));
    }
```

**What it does** — `Basic Og==` and every empty-credential combination fail
even against empty expected values (issue #73).

### fn scheme_is_case_and_whitespace_tolerant

**Identification** — unit test; marker
`// md:mod tests > fn scheme_is_case_and_whitespace_tolerant`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn scheme_is_case_and_whitespace_tolerant
    #[test]
    fn scheme_is_case_and_whitespace_tolerant() {
        let token = STANDARD.encode("alice:s3cr3t");
        assert!(verify_basic(
            Some(&format!("basic {token}")),
            "alice",
            "s3cr3t"
        ));
        assert!(verify_basic(
            Some(&format!("BASIC {token}")),
            "alice",
            "s3cr3t"
        ));
        assert!(verify_basic(
            Some(&format!("Basic   {token}")),
            "alice",
            "s3cr3t"
        ));
        assert!(verify_basic(
            Some(&format!("\tBasic {token}\t")),
            "alice",
            "s3cr3t"
        ));
        assert!(!verify_basic(
            Some(&format!("Basic {token} extra")),
            "alice",
            "s3cr3t"
        ));
    }
```

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
| 1 | `Overview` | `// md:Overview` |
| 2 | `fn verify_basic` | `// md:fn verify_basic` |
| 3 | `mod tests` (container) | `// md:mod tests` |
| 4 | `imports` | `// md:mod tests > imports` |
| 5 | `fn basic` | `// md:mod tests > fn basic` |
| 6 | `fn accepts_valid_credentials` | `// md:mod tests > fn accepts_valid_credentials` |
| 7 | `fn rejects_wrong_password_user_and_missing_header` | `// md:mod tests > fn rejects_wrong_password_user_and_missing_header` |
| 8 | `fn password_with_colons_works` | `// md:mod tests > fn password_with_colons_works` |
| 9 | `fn rejects_empty_expected_credentials` | `// md:mod tests > fn rejects_empty_expected_credentials` |
| 10 | `fn scheme_is_case_and_whitespace_tolerant` | `// md:mod tests > fn scheme_is_case_and_whitespace_tolerant` |
