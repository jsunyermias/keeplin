//! Shared HTTP Basic Authentication check.
//!
//! Both the gRPC interceptor (`main.rs`) and the REST/WebSocket middleware (`rest.rs`)
//! authenticate requests the same way, so the credential comparison lives here in one
//! place. The comparison is constant-time (via [`subtle::ConstantTimeEq`]) and evaluates
//! both the username and the password unconditionally — no `&&`/`||` short-circuit — so
//! response timing cannot reveal whether the username alone was correct.

use base64::{engine::general_purpose::STANDARD, Engine};
use subtle::ConstantTimeEq;

/// Verifies an HTTP Basic `Authorization` header value against the expected credentials.
///
/// `header` is the raw header value (e.g. `"Basic dXNlcjpwYXNz"`), or `None` when the
/// header is absent. Returns `true` only when the header is a well-formed Basic credential
/// whose decoded `user:pass` matches `expected_user` / `expected_pass`. The password may
/// itself contain colons; only the **first** colon separates user from password
/// (per RFC 7617).
pub fn verify_basic(header: Option<&str>, expected_user: &str, expected_pass: &str) -> bool {
    // Empty expected credentials can never be a valid, intentional configuration: they would
    // make `Basic Og==` (base64 of ":") authenticate as anyone. Startup validation rejects
    // this too (see `Config::validate_auth`), but guard here so no caller path can enable a
    // credential-less bypass (issue #73).
    if expected_user.is_empty() || expected_pass.is_empty() {
        return false;
    }
    let Some(header) = header else {
        return false;
    };
    // RFC 7617 / RFC 7235: the auth scheme is case-insensitive and any amount of whitespace
    // may separate it from the credentials. Split on whitespace so `basic dXNlcjpwYXNz` and
    // `Basic  dXNlcjpwYXNz` (extra spaces) are accepted, not just the exact `"Basic "`
    // prefix (issue #76). A base64 token contains no internal whitespace, so exactly two
    // tokens are expected — a third means the header is malformed.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn basic(user: &str, pass: &str) -> String {
        format!("Basic {}", STANDARD.encode(format!("{user}:{pass}")))
    }

    #[test]
    fn accepts_valid_credentials() {
        assert!(verify_basic(
            Some(&basic("alice", "s3cr3t")),
            "alice",
            "s3cr3t"
        ));
    }

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

    #[test]
    fn password_with_colons_works() {
        let pass = "p:a:s:s";
        assert!(verify_basic(Some(&basic("alice", pass)), "alice", pass));
    }

    #[test]
    fn rejects_empty_expected_credentials() {
        // `Basic Og==` is base64 of ":" — empty user, empty password. It must never
        // authenticate, even against empty expected values (issue #73).
        let empty = format!("Basic {}", STANDARD.encode(":"));
        assert!(!verify_basic(Some(&empty), "", ""));
        assert!(!verify_basic(Some(&basic("", "")), "", ""));
        assert!(!verify_basic(Some(&basic("alice", "")), "alice", ""));
        assert!(!verify_basic(Some(&basic("", "s3cr3t")), "", "s3cr3t"));
    }

    #[test]
    fn scheme_is_case_and_whitespace_tolerant() {
        // RFC 7617: scheme is case-insensitive and whitespace between scheme and token is
        // arbitrary (issue #76).
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
        // A stray third token is malformed and rejected.
        assert!(!verify_basic(
            Some(&format!("Basic {token} extra")),
            "alice",
            "s3cr3t"
        ));
    }
}
