//! Client side of the keeplin-srv protocol/capability handshake
//! (`GET /version`, issues keeplin-srv#39/keeplin#114).
//!
//! keeplin and keeplin-srv evolve in separate repositories (keeplin-srv pins a
//! keeplin-core git `rev`), so a wire-protocol drift between the two would
//! otherwise fail silently or with confusing mid-sync errors. This module is
//! the **single place in this repo** that defines which server protocol this
//! client speaks; keeplin-srv mirrors the same rule around its
//! `PROTOCOL_VERSION` constant in `src/http.rs`.
//!
//! The contract, applied identically at both connect points (`DbBackend::new`
//! for the relay, `CollabBackend::start` for the collaborative channel):
//!
//! - `/version` answers with a **compatible** `protocol_version` → log the
//!   negotiated version + capabilities and proceed.
//! - `/version` answers with an **incompatible** `protocol_version` → fail
//!   loudly at startup with an actionable message (which side to upgrade);
//!   sync is not attempted.
//! - `/version` is **missing/unreachable/unparseable** (an older keeplin-srv,
//!   or a fake relay in tests) → warn and continue; behaviour is unchanged
//!   from before the handshake existed (backward compatible).

use serde::Deserialize;

/// The sync/collab wire-protocol version this client speaks. Mirrors
/// keeplin-srv's `PROTOCOL_VERSION` (`src/http.rs`); bump both sides together
/// on a breaking change to the relay or collab message shapes.
pub const PROTOCOL_VERSION: u32 = 1;

/// The compatibility rule, in one place: exact protocol match. Capabilities
/// cover additive evolution (a client probes them instead of guessing), so a
/// `protocol_version` bump is reserved for breaking changes — hence equality,
/// not a range.
pub fn compatible_with(server_protocol: u32) -> bool {
    server_protocol == PROTOCOL_VERSION
}

/// What `GET /version` advertises. Unknown fields are ignored so an older
/// client keeps working against a newer server's additions.
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

/// Outcome of the startup handshake.
#[derive(Debug, Clone)]
pub enum Handshake {
    /// The server speaks our protocol; capabilities are known.
    Compatible(ServerInfo),
    /// The server answered `/version` with a protocol we do not speak.
    Incompatible(ServerInfo),
    /// No usable `/version` (older server, unreachable, or not an HTTP
    /// endpoint at all). The caller warns and continues.
    Unavailable,
}

/// Fetch `GET {http_base}/version` and classify the answer. Never errors:
/// anything short of a well-formed reply is `Unavailable` — the pre-handshake
/// behaviour must keep working against old servers and test relays.
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

/// The actionable startup error for an incompatible server: name both
/// versions and say which side to upgrade.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_is_compatible() {
        assert!(compatible_with(PROTOCOL_VERSION));
        assert!(!compatible_with(PROTOCOL_VERSION + 1));
        assert!(!compatible_with(0));
    }

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
}
