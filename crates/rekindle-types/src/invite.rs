//! Invite secrets for self-sovereign community join.
//!
//! In v2.0, invites are distributed out-of-band (deep links, QR codes, peer share).
//! The invite blob contains everything a joiner needs: governance key, registry key,
//! slot seed, channel keys, and current MEK. Encrypted with HKDF(invite_code).

use serde::{Deserialize, Serialize};

/// Decrypted invite secrets — everything needed to join a community.
///
/// The invite code (shared out-of-band) is the HKDF key that decrypts this blob.
/// The governance_key in the deep link URL identifies the community.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteSecrets {
    /// DHT key of the SMPL governance record (community identifier).
    pub governance_key: String,
    /// DHT key of the SMPL member registry record.
    pub registry_key: String,
    /// Private route blob to the inviting member for bootstrap bundle app_call.
    pub inviter_route_blob: Vec<u8>,
    /// 32-byte slot seed for deriving SMPL member slot keypairs (hex-encoded).
    pub slot_seed: String,
    /// Current MEK wire bytes (generation LE + key, base64-encoded).
    pub mek_wire_bytes: String,
    /// Channel record keys for direct channel access.
    pub channel_keys: Vec<ChannelKeyInfo>,
    /// Community display name (for UI before full state is loaded).
    pub community_name: String,
}

/// A channel's DHT record key bundled in invite secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelKeyInfo {
    /// Channel identifier (hex-encoded 16-byte UUID).
    pub channel_id: String,
    /// DHT key of the channel's SMPL record.
    pub record_key: String,
    /// Channel display name.
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_secrets_serde_roundtrip() {
        let secrets = InviteSecrets {
            governance_key: "VLD0:gov123".into(),
            registry_key: "VLD0:reg456".into(),
            inviter_route_blob: vec![1, 2, 3, 4],
            slot_seed: "ab".repeat(32),
            mek_wire_bytes: "base64mekdata".into(),
            channel_keys: vec![ChannelKeyInfo {
                channel_id: "ch1".into(),
                record_key: "VLD0:ch789".into(),
                name: "general".into(),
            }],
            community_name: "Test Community".into(),
        };
        let json = serde_json::to_string(&secrets).unwrap();
        let back: InviteSecrets = serde_json::from_str(&json).unwrap();
        assert_eq!(back.governance_key, "VLD0:gov123");
        assert_eq!(back.inviter_route_blob, vec![1, 2, 3, 4]);
        assert_eq!(back.channel_keys.len(), 1);
    }
}

// ── Invite deep link ─────────────────────────────────────────────────

/// The three components an invite link carries.
///
/// Lives in Tier 1 because every frontend must be able to accept the
/// same link: the parser previously existed only in `src-tauri`'s
/// `deep_links.rs`, so a CLI or third-party TUI had no way to act on a
/// `rekindle://invite/...` URL even though the daemon's `CommunityJoin`
/// IPC takes one.
///
/// Format: `rekindle://invite/{governance_key}/{secrets_record_key}/{invite_code}`
/// (`rekindle://community/...` is accepted as an alias.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteLink {
    pub governance_key: String,
    pub secrets_record_key: String,
    pub invite_code: String,
}

impl InviteLink {
    /// Parse an invite URL, or `None` if it is not one.
    ///
    /// Rejects a link with any empty component: a blank governance key
    /// or code would otherwise reach the join flow and fail much later
    /// with a confusing error.
    #[must_use]
    pub fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        let rest = url
            .strip_prefix("rekindle://invite/")
            .or_else(|| url.strip_prefix("rekindle://community/"))?
            .trim_end_matches('/');

        let mut parts = rest.splitn(3, '/');
        let governance_key = parts.next()?;
        let secrets_record_key = parts.next()?;
        let invite_code = parts.next()?;
        if governance_key.is_empty() || secrets_record_key.is_empty() || invite_code.is_empty() {
            return None;
        }
        Some(Self {
            governance_key: governance_key.to_string(),
            secrets_record_key: secrets_record_key.to_string(),
            invite_code: invite_code.to_string(),
        })
    }
}

#[cfg(test)]
mod invite_link_tests {
    use super::InviteLink;

    #[test]
    fn parses_a_well_formed_link() {
        let link = InviteLink::parse("rekindle://invite/VLD0:gov/VLD0:sec/code123").unwrap();
        assert_eq!(link.governance_key, "VLD0:gov");
        assert_eq!(link.secrets_record_key, "VLD0:sec");
        assert_eq!(link.invite_code, "code123");
    }

    #[test]
    fn accepts_the_community_alias_and_trailing_slash() {
        let link = InviteLink::parse("rekindle://community/g/s/c/").unwrap();
        assert_eq!(link.invite_code, "c");
    }

    #[test]
    fn rejects_non_invite_urls_and_empty_components() {
        assert!(InviteLink::parse("https://example.com").is_none());
        assert!(InviteLink::parse("rekindle://invite/g/s").is_none());
        assert!(InviteLink::parse("rekindle://invite//s/c").is_none());
        assert!(InviteLink::parse("rekindle://invite/g//c").is_none());
        assert!(InviteLink::parse("rekindle://invite/g/s/").is_none());
    }

    #[test]
    fn an_invite_code_may_contain_slashes() {
        // splitn(3) keeps the remainder intact — a base64url code can
        // contain characters we must not truncate.
        let link = InviteLink::parse("rekindle://invite/g/s/a/b/c").unwrap();
        assert_eq!(link.invite_code, "a/b/c");
    }
}
