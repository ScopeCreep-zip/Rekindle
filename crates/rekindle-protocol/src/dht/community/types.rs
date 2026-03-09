//! V2 community data types for the multi-record DHT architecture.
//!
//! These types are used in the manifest (DFLT record), member registry
//! (SMPL record), and per-channel message records (SMPL records).

use serde::{Deserialize, Serialize};

// ── Manifest subkey layout (DFLT, 16 subkeys, single owner = coordinator) ──

/// Subkey 0: Community metadata (name, description, icon, policies).
pub const MANIFEST_METADATA: u32 = 0;
/// Subkey 1: Channel directory (list of all channels).
pub const MANIFEST_CHANNELS: u32 = 1;
/// Subkey 2: Category directory (channel groupings).
pub const MANIFEST_CATEGORIES: u32 = 2;
/// Subkey 3: Role definitions.
pub const MANIFEST_ROLES: u32 = 3;
/// Subkey 4: Ban list.
pub const MANIFEST_BANS: u32 = 4;
/// Subkey 5: Coordinator info (route blob, epoch, capabilities).
pub const MANIFEST_COORDINATOR: u32 = 5;
/// Subkey 6: Community policies (join rules, content moderation).
pub const MANIFEST_POLICIES: u32 = 6;
/// Subkey 7: Invite list.
pub const MANIFEST_INVITES: u32 = 7;
/// Subkey 8: Reserved.
/// Subkey 9: AutoMod configuration.
pub const MANIFEST_AUTOMOD: u32 = 9;
/// Subkey 10: Onboarding configuration.
pub const MANIFEST_ONBOARDING: u32 = 10;
/// Subkey 11: Welcome screen.
pub const MANIFEST_WELCOME: u32 = 11;
/// Subkey 12: Registry spine (for >256 member chaining).
pub const MANIFEST_REGISTRY_SPINE: u32 = 12;
/// Subkey 13: Reserved.
/// Subkey 14: Audit log DHT record key (pointer).
pub const MANIFEST_AUDIT_LOG_KEY: u32 = 14;
/// Total manifest subkey count.
pub const MANIFEST_SUBKEY_COUNT: u32 = 16;

// ── Member registry subkey layout (SMPL, multi-writer) ──

/// Owner subkey 0: Member index (list of all member pseudonym keys + subkey assignments).
pub const REGISTRY_MEMBER_INDEX: u32 = 0;
/// Owner subkey 1: MEK vault (encrypted MEK copies for key distribution).
pub const REGISTRY_MEK_VAULT: u32 = 1;
/// Owner subkey count for the coordinator (controls member index + MEK vault).
pub const REGISTRY_OWNER_SUBKEY_COUNT: u16 = 2;
/// Each member gets 1 subkey for their presence data.
pub const REGISTRY_MEMBER_SUBKEY_COUNT: u16 = 1;

// ── Channel types ──

/// All supported channel kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelKind {
    Text,
    Voice,
    Announcement,
    Forum,
    Stage,
    Directory,
    Media,
    Events,
    Dm,
}

impl ChannelKind {
    /// Convert from the u8 wire representation.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Text),
            1 => Some(Self::Voice),
            2 => Some(Self::Announcement),
            3 => Some(Self::Forum),
            4 => Some(Self::Stage),
            5 => Some(Self::Directory),
            6 => Some(Self::Media),
            7 => Some(Self::Events),
            8 => Some(Self::Dm),
            _ => None,
        }
    }

    /// Convert to the u8 wire representation.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Text => 0,
            Self::Voice => 1,
            Self::Announcement => 2,
            Self::Forum => 3,
            Self::Stage => 4,
            Self::Directory => 5,
            Self::Media => 6,
            Self::Events => 7,
            Self::Dm => 8,
        }
    }

    /// String representation matching the serde lowercase format.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
            Self::Announcement => "announcement",
            Self::Forum => "forum",
            Self::Stage => "stage",
            Self::Directory => "directory",
            Self::Media => "media",
            Self::Events => "events",
            Self::Dm => "dm",
        }
    }
}

impl std::fmt::Display for ChannelKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ChannelKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text" => Ok(Self::Text),
            "voice" => Ok(Self::Voice),
            "announcement" => Ok(Self::Announcement),
            "forum" => Ok(Self::Forum),
            "stage" => Ok(Self::Stage),
            "directory" => Ok(Self::Directory),
            "media" => Ok(Self::Media),
            "events" => Ok(Self::Events),
            "dm" => Ok(Self::Dm),
            other => Err(format!("unknown channel kind: {other}")),
        }
    }
}

// ── Manifest types ──

/// Community metadata stored in manifest subkey 0.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityMetadataV2 {
    pub name: String,
    pub description: Option<String>,
    pub icon_hash: Option<String>,
    pub created_at: u64,
    pub owner_pseudonym: String,
    /// Timestamp of the last DHT keepalive refresh (seconds since epoch).
    #[serde(default)]
    pub last_refreshed: u64,
}

/// A channel entry in the manifest channel directory (subkey 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelEntryV2 {
    pub id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub sort_order: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub slowmode_seconds: u32,
    #[serde(default)]
    pub nsfw: bool,
    /// DHT record key for this channel's message record (SMPL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_record_key: Option<String>,
    /// Current MEK generation for this channel.
    #[serde(default)]
    pub mek_generation: u64,
    #[serde(default)]
    pub permission_overwrites: Vec<super::PermissionOverwrite>,
    /// DHTLog spine key for persistent message history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_key: Option<String>,
}

/// A category entry in the manifest category directory (subkey 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryEntry {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

/// A role entry in the manifest role list (subkey 3).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleEntryV2 {
    pub id: u32,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    /// Role hierarchy position. Higher = more authority.
    pub position: i32,
    /// Whether to display this role separately in the member list.
    #[serde(default)]
    pub hoist: bool,
    /// Whether this role can be @mentioned by anyone.
    #[serde(default)]
    pub mentionable: bool,
}

/// A member summary in the member index (registry owner subkey 0).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberSummary {
    pub pseudonym_key: String,
    pub display_name: String,
    pub role_ids: Vec<u32>,
    pub joined_at: u64,
    /// The member's subkey index in the SMPL registry record.
    pub subkey_index: u32,
    #[serde(default)]
    pub onboarding_complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_until: Option<u64>,
}

/// A member's presence data written to their own SMPL subkey.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberPresence {
    pub pseudonym_key: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_info: Option<String>,
    /// Route blob for direct messaging within the community.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_blob: Option<Vec<u8>>,
    pub last_heartbeat: u64,
    /// Whether this member is currently acting as the coordinator.
    #[serde(default)]
    pub is_coordinator: bool,
    /// Timestamp when this member became coordinator (for priority resolution).
    #[serde(default)]
    pub coordinator_since: u64,
    /// Whether this member opts in to serve full message history.
    #[serde(default)]
    pub is_archiver: bool,
}

/// Signed presence wrapper for authenticity.
///
/// The presence is signed by the member's pseudonym key so other members
/// can verify it wasn't forged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedPresence {
    pub presence: MemberPresence,
    /// Ed25519 signature over the serialized `presence` bytes.
    pub pseudonym_signature: Vec<u8>,
}

/// Registry spine for communities with >256 members.
///
/// Stored in manifest subkey 12. Tracks multiple registry segments,
/// each holding up to 256 member slots.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrySpine {
    /// Total number of members across all segments.
    pub total_members: u32,
    /// Ordered list of registry segments.
    pub segments: Vec<RegistrySegmentInfo>,
}

/// Information about a single registry segment record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrySegmentInfo {
    /// DHT record key for this segment.
    pub record_key: String,
    /// Slot seed encrypted with the community MEK (for admin delegation).
    pub slot_seed_encrypted: Vec<u8>,
    /// Member index range covered by this segment (start_index, end_index).
    pub member_range: (u32, u32),
}

/// A ban entry in the manifest ban list (subkey 4).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BanEntry {
    pub pseudonym_key: String,
    pub reason: Option<String>,
    pub banned_by: String,
    pub banned_at: u64,
}

/// Coordinator info stored in manifest subkey 5.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoordinatorInfo {
    /// The coordinator's pseudonym public key.
    pub pseudonym_key: String,
    /// Route blob for sending RPCs to the coordinator.
    pub route_blob: Vec<u8>,
    /// Monotonically increasing epoch — incremented on coordinator restart.
    pub epoch: u64,
    /// Capabilities advertised by the coordinator.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Timestamp of the last heartbeat write (seconds since epoch).
    /// Members trigger re-election when `now - heartbeat_at > 60`.
    #[serde(default)]
    pub heartbeat_at: u64,
}

/// Community policies stored in manifest subkey 6.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityPolicy {
    /// Whether the community requires an invite to join.
    #[serde(default)]
    pub invite_only: bool,
    /// Maximum number of members (0 = unlimited).
    #[serde(default)]
    pub max_members: u32,
    /// Default role IDs assigned to new members.
    #[serde(default)]
    pub default_role_ids: Vec<u32>,
    /// Content moderation level.
    #[serde(default)]
    pub moderation_level: ModerationLevel,
}

/// Content moderation strictness level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModerationLevel {
    #[default]
    None,
    Low,
    Medium,
    High,
}

// ── MEK distribution types ──

/// MEK vault entry stored in registry owner subkey 1.
///
/// Contains encrypted copies of the current MEK for each member,
/// wrapped with X25519 ECDH + AES-256-GCM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MEKVaultEntry {
    /// Channel ID this vault entry is for (empty string = community-wide MEK).
    pub channel_id: String,
    /// MEK generation number.
    pub generation: u64,
    /// Pseudonym public key (hex) of the admin who wrapped these copies.
    /// Recipients need this to derive the shared ECDH secret for unwrapping.
    pub rotator_pseudonym: String,
    /// Per-member encrypted MEK copies.
    pub copies: Vec<EncryptedMEKCopy>,
}

/// A single encrypted MEK copy targeted at a specific member.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedMEKCopy {
    /// Target member's pseudonym public key (hex).
    pub target_pseudonym: String,
    /// Encrypted MEK: `[12-byte nonce || ciphertext+tag]` (68 bytes for a 40-byte MEK).
    #[serde(with = "base64_bytes")]
    pub encrypted_mek: Vec<u8>,
}

/// Invite entry stored in manifest subkey 7.
///
/// The `code_hash` is SHA-256(raw_code) so the raw invite code is never
/// exposed in the publicly-readable DHT manifest. The `encrypted_secrets`
/// blob can only be decrypted by someone who has the raw code.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteEntry {
    /// SHA-256 hash of the invite code (hex). Raw code never stored in DHT.
    pub code_hash: String,
    pub created_by: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub max_uses: u32,
    #[serde(default)]
    pub use_count: u32,
    /// Encrypted `InviteSecrets` blob (base64). Decrypted with
    /// `HKDF(raw_invite_code) → AES-256-GCM`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_secrets: Option<String>,
}

/// Secrets embedded in a community invite for self-service joining.
///
/// Encrypted with HKDF(invite_code) → AES-256-GCM and stored alongside
/// the invite metadata in manifest subkey 7. Contains everything a new
/// member needs to join without any online coordinator or peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteSecrets {
    /// Slot seed for deriving SMPL presence keypairs (hex-encoded, 32 bytes).
    pub slot_seed: String,
    /// MEK wire bytes: `[8-byte generation LE || 32-byte key]` (base64-encoded).
    pub mek_wire_bytes: String,
    /// DHT record key for the member registry (SMPL record).
    pub registry_key: String,
    /// Pre-assigned subkey index (single-use invites).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_subkey_index: Option<u32>,
    /// Slot range `[start, end]` inclusive (multi-use invites).
    /// Joiner claims the first empty slot in this range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_range: Option<(u32, u32)>,
}

/// Serde helper for base64-encoding Vec<u8> fields in JSON.
mod base64_bytes {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        serializer.serialize_str(&b64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_kind_roundtrip_u8() {
        for v in 0..=8u8 {
            let kind = ChannelKind::from_u8(v).unwrap();
            assert_eq!(kind.to_u8(), v);
        }
        assert!(ChannelKind::from_u8(9).is_none());
    }

    #[test]
    fn channel_kind_roundtrip_str() {
        let kinds = [
            "text",
            "voice",
            "announcement",
            "forum",
            "stage",
            "directory",
            "media",
            "events",
            "dm",
        ];
        for s in &kinds {
            let kind: ChannelKind = s.parse().unwrap();
            assert_eq!(kind.as_str(), *s);
        }
    }

    #[test]
    fn channel_kind_serde_json() {
        let kind = ChannelKind::Forum;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"forum\"");
        let back: ChannelKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kind);
    }

    #[test]
    fn mek_vault_entry_serde() {
        let entry = MEKVaultEntry {
            channel_id: "ch_01".into(),
            generation: 1,
            rotator_pseudonym: "rotator_key_hex".into(),
            copies: vec![EncryptedMEKCopy {
                target_pseudonym: "abcdef".into(),
                encrypted_mek: vec![1, 2, 3, 4, 5],
            }],
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: MEKVaultEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.channel_id, "ch_01");
        assert_eq!(back.copies[0].encrypted_mek, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn community_metadata_v2_serde() {
        let meta = CommunityMetadataV2 {
            name: "Test".into(),
            description: Some("desc".into()),
            icon_hash: None,
            created_at: 1234567890,
            owner_pseudonym: "abc".into(),
            last_refreshed: 0,
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("ownerPseudonym"));
        let back: CommunityMetadataV2 = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "Test");
    }
}
