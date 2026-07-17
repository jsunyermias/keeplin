# `auth.rs` — shared HTTP Basic authentication

## Purpose

One place for the credential check used by **both** client surfaces: the gRPC interceptor
(`main.rs`) and the REST/WebSocket middleware (`rest.rs`). Keeping it here guarantees the two
surfaces authenticate **identically**.

## Public function

### `fn verify_basic(header: Option<&str>, expected_user: &str, expected_pass: &str) -> bool`

Returns `true` only when `header` is a well-formed `Authorization: Basic <base64(user:pass)>`
value whose decoded credentials match. `header` is `None` when the header is absent.

Steps:

1. Reject up front if either expected credential is empty — empty credentials are never a
   valid configuration and would let `Basic Og==` (base64 of `:`) authenticate as anyone.
   (Startup validation also refuses this; see `config.md`.)
2. Parse the scheme **per RFC 7617 / RFC 7235**: split on whitespace, accept the scheme
   case-insensitively (`basic`, `BASIC`) with any amount of separating whitespace, and take
   the single Base64 token that follows (a stray third token is malformed). Base64-decode it;
   interpret as UTF-8.
3. Split on the **first** colon only (so passwords may contain colons, per RFC 7617).
4. Compare username and password with **constant-time** equality.

## Why constant-time

The username and password are compared with `subtle::ConstantTimeEq` and combined with a
bitwise `&` (no `&&`/`||` short-circuit), so both are always evaluated. This prevents a timing
side-channel from revealing whether the username alone was correct or how many characters
matched.

## How the surfaces use it

| Surface | Caller | On failure |
|---------|--------|------------|
| gRPC | `validate_basic_auth` interceptor in `main.rs` | `Status::unauthenticated` |
| REST / WebSocket | `auth_mw` middleware in `rest.rs` | `401` + `WWW-Authenticate: Basic` |

When `auth_username`/`auth_password` are **not** configured, both surfaces skip the check
entirely (a no-op), so auth is opt-in but uniform once enabled.

## Design notes

- The check is transport-agnostic — it takes the raw header string, so the same logic works for
  a tonic metadata value and an axum header.
- **Plain HTTP/gRPC leaks credentials on the wire.** Enable TLS (gRPC) or front the REST/WS
  listener with a TLS proxy in production (see `SECURITY.md`).

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `verify_basic()` — defined here (EXTRACTED; file-local)
- `basic()` — defined here (EXTRACTED; file-local)
- `accepts_valid_credentials()` — defined here (EXTRACTED; file-local)
- `rejects_wrong_password_user_and_missing_header()` — defined here (EXTRACTED; file-local)
- `password_with_colons_works()` — defined here (EXTRACTED; file-local)
- `rejects_empty_expected_credentials()` — defined here (EXTRACTED; file-local)
- `scheme_is_case_and_whitespace_tolerant()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- gRPC and REST/WebSocket must authenticate IDENTICALLY — both call this one Basic-auth check.
- When no credentials are configured the check is a no-op (open local daemon); when configured, every request is checked.

## Related files

- `keeplin-daemon/src/main.rs` — the gRPC interceptor that calls this.
- `keeplin-daemon/src/rest.rs` — the axum middleware that calls this.
- `SECURITY.md` — credentials & TLS guidance.
