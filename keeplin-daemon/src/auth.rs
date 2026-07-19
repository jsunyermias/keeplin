// md:Overview
use base64::{engine::general_purpose::STANDARD, Engine};
use subtle::ConstantTimeEq;

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

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;

    // md:mod tests > fn basic
    fn basic(user: &str, pass: &str) -> String {
        format!("Basic {}", STANDARD.encode(format!("{user}:{pass}")))
    }

    // md:mod tests > fn accepts_valid_credentials
    #[test]
    fn accepts_valid_credentials() {
        assert!(verify_basic(
            Some(&basic("alice", "s3cr3t")),
            "alice",
            "s3cr3t"
        ));
    }

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

    // md:mod tests > fn password_with_colons_works
    #[test]
    fn password_with_colons_works() {
        let pass = "p:a:s:s";
        assert!(verify_basic(Some(&basic("alice", pass)), "alice", pass));
    }

    // md:mod tests > fn rejects_empty_expected_credentials
    #[test]
    fn rejects_empty_expected_credentials() {
        let empty = format!("Basic {}", STANDARD.encode(":"));
        assert!(!verify_basic(Some(&empty), "", ""));
        assert!(!verify_basic(Some(&basic("", "")), "", ""));
        assert!(!verify_basic(Some(&basic("alice", "")), "alice", ""));
        assert!(!verify_basic(Some(&basic("", "s3cr3t")), "", "s3cr3t"));
    }

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
}
