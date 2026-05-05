//! Architecture §4.3 — flat description of a community member used by
//! gossip envelopes (`MemberJoined` broadcasts, `JoinAccepted` member
//! list, `BootstrapResponse.member_list`). Mirrors the wire shape that
//! the gossip path serialized as untyped JSON before the v2.0 envelope
//! migration; the typed form here is what `CommunityEnvelope` carries.

use serde::{Deserialize, Serialize};

/// Member identity + presence snapshot, as broadcast by gossip.
///
/// String IDs match the existing on-the-wire envelope contract used by
/// `rekindle-protocol::dht::community::envelope`. The Cap'n Proto
/// migration (plan: `.claude/plans/community-envelope-capnp-migration.md`,
/// Phase 5) replaces these with typed ID schemas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberInfo {
    /// Hex-encoded community pseudonym Ed25519 public key.
    pub pseudonym_key: String,
    pub display_name: String,
    pub role_ids: Vec<u32>,
    /// `online` / `away` / `busy` / `offline`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_until: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_blob: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pronouns: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_color: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<String>,
    /// Unix-seconds of the last presence heartbeat from this member.
    /// Used by joiners to age out stale entries from `BootstrapResponse.member_list`.
    #[serde(default)]
    pub last_seen: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_info_roundtrip() {
        let m = MemberInfo {
            pseudonym_key: "abcd".into(),
            display_name: "Alice".into(),
            role_ids: vec![0, 1],
            status: "online".into(),
            timeout_until: None,
            route_blob: Some(vec![1, 2, 3]),
            bio: Some("hi".into()),
            pronouns: None,
            theme_color: Some(0xff_ee_dd),
            badges: vec!["founder".into()],
            last_seen: 1_700_000_000,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: MemberInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn member_info_defaults_omit_optional() {
        let m = MemberInfo {
            pseudonym_key: "abcd".into(),
            display_name: "Alice".into(),
            role_ids: vec![],
            status: "online".into(),
            timeout_until: None,
            route_blob: None,
            bio: None,
            pronouns: None,
            theme_color: None,
            badges: vec![],
            last_seen: 0,
        };
        let json = serde_json::to_string(&m).unwrap();
        // Optional + empty fields are skipped on the wire.
        assert!(!json.contains("timeoutUntil"));
        assert!(!json.contains("routeBlob"));
        assert!(!json.contains("badges"));
    }
}
