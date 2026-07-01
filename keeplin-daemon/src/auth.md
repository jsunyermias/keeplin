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

1. Require the `"Basic "` prefix; Base64-decode the rest; interpret as UTF-8.
2. Split on the **first** colon only (so passwords may contain colons, per RFC 7617).
3. Compare username and password with **constant-time** equality.

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

## Related files

- `keeplin-daemon/src/main.rs` — the gRPC interceptor that calls this.
- `keeplin-daemon/src/rest.rs` — the axum middleware that calls this.
- `SECURITY.md` — credentials & TLS guidance.
