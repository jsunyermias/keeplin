// md:Overview
use serde::Deserialize;

// md:PROTOCOL_VERSION
pub const PROTOCOL_VERSION: u32 = 1;

// md:fn compatible_with
pub fn compatible_with(server_protocol: u32) -> bool {
    server_protocol == PROTOCOL_VERSION
}

// md:ServerInfo
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

// md:Handshake
#[derive(Debug, Clone)]
pub enum Handshake {
    Compatible(ServerInfo),
    Incompatible(ServerInfo),
    Unavailable,
}

// md:fn negotiate
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

// md:fn incompatible_message
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

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;

    // md:mod tests > fn exact_match_is_compatible
    #[test]
    fn exact_match_is_compatible() {
        assert!(compatible_with(PROTOCOL_VERSION));
        assert!(!compatible_with(PROTOCOL_VERSION + 1));
        assert!(!compatible_with(0));
    }

    // md:mod tests > fn incompatible_message_names_the_side_to_upgrade
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
