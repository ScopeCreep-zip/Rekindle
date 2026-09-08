//! V2 community data types for the multi-record DHT architecture.
//!
//! These types are used in the manifest (DFLT record), member registry
//! (SMPL record), and per-channel message records (SMPL records).

use serde::{Deserialize, Serialize};

// ── Manifest + registry subkey layout ──
//
// The manifest is the v1.0 coordinator-owned governance record (DFLT,
// 16 subkeys). v2.0 replaces it with an SMPL `o_cnt:0` governance
// record — see the architecture doc's v1.0 -> v2.0 table — so this
// table describes a record on its way out, not the target layout. The
// indices stay pinned meanwhile because they are wire-visible.
//
// Declared once in `rekindle_types::dht_layout` — the daemon track
// indexes the same records and kept its own copy of this table, which
// is how the profile and registry layouts drifted apart. Aliased here
// so existing call sites are unchanged.
pub use rekindle_types::dht_layout::manifest::{
    AUDIT_LOG_KEY as MANIFEST_AUDIT_LOG_KEY, AUTOMOD as MANIFEST_AUTOMOD, BANS as MANIFEST_BANS,
    CATEGORIES as MANIFEST_CATEGORIES, CHANNELS as MANIFEST_CHANNELS,
    COORDINATOR as MANIFEST_COORDINATOR, INVITES as MANIFEST_INVITES,
    METADATA as MANIFEST_METADATA, ONBOARDING as MANIFEST_ONBOARDING,
    POLICIES as MANIFEST_POLICIES, ROLES as MANIFEST_ROLES, SUBKEY_COUNT as MANIFEST_SUBKEY_COUNT,
    WELCOME as MANIFEST_WELCOME,
};

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
    /// Whether members can assign this role to themselves.
    #[serde(default)]
    pub self_assignable: bool,
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

/// A ban entry in the manifest ban list (subkey 4).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BanEntry {
    pub pseudonym_key: String,
    pub reason: Option<String>,
    pub banned_by: String,
    pub banned_at: u64,
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
    fn community_metadata_v2_serde() {
        let meta = CommunityMetadataV2 {
            name: "Test".into(),
            description: Some("desc".into()),
            icon_hash: None,
            created_at: 1_234_567_890,
            owner_pseudonym: "abc".into(),
            last_refreshed: 0,
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("ownerPseudonym"));
        let back: CommunityMetadataV2 = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "Test");
    }
}
