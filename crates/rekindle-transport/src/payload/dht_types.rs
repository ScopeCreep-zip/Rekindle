//! DHT record value types — structured data stored in DHT subkeys.
//!
//! Every DHT subkey value is frame-encoded (version + type + length + payload)
//! before writing, ensuring all stored data is self-describing and versioned.

use serde::{Deserialize, Serialize};

// ── Profile record (DFLT, 8 subkeys) ────────────────────────────────

pub const PROFILE_SUBKEY_DISPLAY_NAME: u32 = 0;
pub const PROFILE_SUBKEY_STATUS_MESSAGE: u32 = 1;
pub const PROFILE_SUBKEY_STATUS: u32 = 2;
pub const PROFILE_SUBKEY_AVATAR: u32 = 3;
pub const PROFILE_SUBKEY_GAME_INFO: u32 = 4;
pub const PROFILE_SUBKEY_PREKEY_BUNDLE: u32 = 5;
pub const PROFILE_SUBKEY_ROUTE_BLOB: u32 = 6;
pub const PROFILE_SUBKEY_METADATA: u32 = 7;
pub const PROFILE_SUBKEY_COUNT: u32 = 8;

/// Status byte encoding for profile subkey 2.
pub const STATUS_ONLINE: u8 = 0;
pub const STATUS_AWAY: u8 = 1;
pub const STATUS_BUSY: u8 = 2;
pub const STATUS_OFFLINE: u8 = 3;
pub const STATUS_INVISIBLE: u8 = 4;

// ── Friend list record (DFLT, 1 subkey) ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendEntry {
    pub public_key: String,
    pub nickname: Option<String>,
    pub group: Option<String>,
    pub added_at: u64,
    pub profile_dht_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FriendList {
    pub friends: Vec<FriendEntry>,
}

// ── Mailbox record (DFLT, 1 subkey) ──────────────────────────────────

pub const MAILBOX_SUBKEY_ROUTE_BLOB: u32 = 0;
pub const MAILBOX_SUBKEY_COUNT: u16 = 1;

// ── Community governance types ───────────────────────────────────────

/// Manifest subkey layout (DFLT, 16 subkeys).
pub const MANIFEST_METADATA: u32 = 0;
pub const MANIFEST_CHANNELS: u32 = 1;
pub const MANIFEST_CATEGORIES: u32 = 2;
pub const MANIFEST_ROLES: u32 = 3;
pub const MANIFEST_BANS: u32 = 4;
pub const MANIFEST_COORDINATOR: u32 = 5;
pub const MANIFEST_POLICIES: u32 = 6;
pub const MANIFEST_INVITES: u32 = 7;
pub const MANIFEST_AUTOMOD: u32 = 9;
pub const MANIFEST_ONBOARDING: u32 = 10;
pub const MANIFEST_WELCOME: u32 = 11;
pub const MANIFEST_REGISTRY_SPINE: u32 = 12;
pub const MANIFEST_AUDIT_LOG_KEY: u32 = 14;
pub const MANIFEST_SUBKEY_COUNT: u32 = 16;

/// Registry subkey layout (SMPL, multi-writer).
pub const REGISTRY_MEMBER_INDEX: u32 = 0;
pub const REGISTRY_MEK_VAULT: u32 = 1;
pub const REGISTRY_OWNER_SUBKEY_COUNT: u16 = 2;
pub const REGISTRY_MEMBER_SUBKEY_COUNT: u16 = 1;

/// Maximum member slots per registry segment.
pub const SLOTS_PER_SEGMENT: u32 = 255;

/// Channel SMPL record constants.
pub const CHANNEL_OWNER_SUBKEY_COUNT: u16 = 0;
pub const CHANNEL_MEMBER_SUBKEY_COUNT: u16 = 1;

// ── Community metadata ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityMetadata {
    pub name: String,
    pub description: Option<String>,
    pub icon_hash: Option<String>,
    pub banner_hash: Option<String>,
    pub created_at: u64,
    pub owner_pseudonym: String,
    #[serde(default)]
    pub last_refreshed: u64,
}

// ── Channel types ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelKind {
    Text, Voice, Announcement, Forum, Stage, Directory, Media, Events, Dm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelEntry {
    pub id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub sort_order: u16,
    pub category_id: Option<String>,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub slowmode_seconds: u32,
    #[serde(default)]
    pub nsfw: bool,
    pub message_record_key: Option<String>,
    #[serde(default)]
    pub mek_generation: u64,
    pub log_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryEntry {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

// ── Role types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleEntry {
    pub id: u32,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    pub position: i32,
    #[serde(default)]
    pub hoist: bool,
    #[serde(default)]
    pub mentionable: bool,
    #[serde(default)]
    pub self_assignable: bool,
}

// ── Member types ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberSummary {
    pub pseudonym_key: String,
    pub display_name: String,
    pub role_ids: Vec<u32>,
    pub joined_at: u64,
    pub subkey_index: u32,
    #[serde(default)]
    pub onboarding_complete: bool,
    pub timeout_until: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberPresence {
    pub pseudonym_key: String,
    pub status: String,
    pub status_message: Option<String>,
    pub game_info: Option<String>,
    pub route_blob: Option<Vec<u8>>,
    pub last_heartbeat: u64,
    #[serde(default)]
    pub is_archiver: bool,
}

// ── Ban types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BanEntry {
    pub pseudonym_key: String,
    pub reason: Option<String>,
    pub banned_by: String,
    pub banned_at: u64,
}

// ── Invite types ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteEntry {
    pub code_hash: String,
    pub created_by: String,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub max_uses: u32,
    #[serde(default)]
    pub use_count: u32,
    pub encrypted_secrets: Option<String>,
}

// ── MEK vault ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MekVaultEntry {
    pub channel_id: String,
    pub generation: u64,
    pub rotator_pseudonym: String,
    pub copies: Vec<EncryptedMekCopy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedMekCopy {
    pub target_pseudonym: String,
    pub encrypted_mek: Vec<u8>,
}

// ── Channel message record ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessage {
    pub sequence: u64,
    pub sender_pseudonym: String,
    pub ciphertext: Vec<u8>,
    pub mek_generation: u64,
    pub timestamp: u64,
    pub reply_to: Option<u64>,
    #[serde(default)]
    pub lamport_ts: u64,
    pub message_id: Option<String>,
}

// ── Invite blob (for friend invites) ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteBlob {
    pub public_key: String,
    pub display_name: String,
    pub mailbox_dht_key: String,
    pub profile_dht_key: String,
    pub route_blob: Vec<u8>,
    pub prekey_bundle: Vec<u8>,
    pub invite_id: Option<String>,
    pub signature: Vec<u8>,
}
