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
    let Some(header) = header else {
        return false;
    };
    let Some(encoded) = header.strip_prefix("Basic ") else {
        return false;
    };
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
}
