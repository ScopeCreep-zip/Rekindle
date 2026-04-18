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
        assert_eq!(back.channel_keys.len(), 1);
    }
}
