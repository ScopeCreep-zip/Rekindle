//! DHT record value types — structured data stored in DHT subkeys.
//!
//! Every DHT subkey value is frame-encoded (version + type + length + payload)
//! before writing, ensuring all stored data is self-describing and versioned.

use serde::{Deserialize, Serialize};

// ── Profile record ──────────────────────────────────────────────────

// The profile layout lives in `rekindle_types::dht_layout::profile` —
// the desktop track indexes the same records, and keeping a private copy
// here is how the two ended up disagreeing about subkey 8 (relay pool
// vs friend-inbox key). These are aliases so existing call sites keep
// reading naturally.
pub use rekindle_types::dht_layout::profile::{
    AVATAR as PROFILE_SUBKEY_AVATAR, DISPLAY_NAME as PROFILE_SUBKEY_DISPLAY_NAME,
    FRIEND_INBOX_KEY as PROFILE_SUBKEY_FRIEND_INBOX_KEY,
    FRIEND_INBOX_KEYPAIR as PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
    GAME_INFO as PROFILE_SUBKEY_GAME_INFO, METADATA as PROFILE_SUBKEY_METADATA,
    PREKEY_BUNDLE as PROFILE_SUBKEY_PREKEY_BUNDLE, ROUTE_BLOB as PROFILE_SUBKEY_ROUTE_BLOB,
    STATUS as PROFILE_SUBKEY_STATUS, STATUS_AWAY, STATUS_BUSY,
    STATUS_MESSAGE as PROFILE_SUBKEY_STATUS_MESSAGE, STATUS_OFFLINE, STATUS_ONLINE,
    SUBKEY_COUNT as PROFILE_SUBKEY_COUNT,
};
pub const STATUS_INVISIBLE: u8 = 4;

/// Number of subkeys in the friend inbox DHT record (DFLT).
pub const FRIEND_INBOX_SUBKEY_COUNT: u32 = 32;

// ── Friend list record (DFLT, 1 subkey) ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendEntry {
    pub public_key: String,
    pub nickname: Option<String>,
    pub group: Option<String>,
    pub added_at: u64,
    pub profile_dht_key: Option<String>,
    /// DhtLog spine key for the per-peer DM conversation.
    /// Created during friend accept. Both peers read/write.
    #[serde(default)]
    pub dm_log_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FriendList {
    pub friends: Vec<FriendEntry>,
}

// ── Mailbox record (DFLT, 1 subkey) ──────────────────────────────────

// Mailbox / manifest / registry subkey layout.
//
// Declared once in `rekindle_types::dht_layout`; this crate previously
// kept its own copy of each table. Eight v1.0 owner-block constants
// (REGISTRY_POLICY, _SCHEMA_VERSION, _OPS_LOG and the five RESERVED_*)
// went with the o_cnt:0 migration — they had no readers.
pub use rekindle_types::dht_layout::channel::{
    MEMBER_SUBKEY_COUNT as CHANNEL_MEMBER_SUBKEY_COUNT,
    OWNER_SUBKEY_COUNT as CHANNEL_OWNER_SUBKEY_COUNT,
};
pub use rekindle_types::dht_layout::mailbox::{
    ROUTE_BLOB as MAILBOX_SUBKEY_ROUTE_BLOB, SUBKEY_COUNT as MAILBOX_SUBKEY_COUNT,
};
pub use rekindle_types::dht_layout::manifest::{
    AUDIT_LOG_KEY as MANIFEST_AUDIT_LOG_KEY, AUTOMOD as MANIFEST_AUTOMOD, BANS as MANIFEST_BANS,
    CATEGORIES as MANIFEST_CATEGORIES, CHANNELS as MANIFEST_CHANNELS,
    COORDINATOR as MANIFEST_COORDINATOR, INVITES as MANIFEST_INVITES,
    METADATA as MANIFEST_METADATA, ONBOARDING as MANIFEST_ONBOARDING,
    POLICIES as MANIFEST_POLICIES, REGISTRY_SPINE_V1 as MANIFEST_REGISTRY_SPINE,
    ROLES as MANIFEST_ROLES, SUBKEY_COUNT as MANIFEST_SUBKEY_COUNT, WELCOME as MANIFEST_WELCOME,
};

/// Maximum member slots per registry segment.
///
/// Re-exported from the desktop track so both agree by construction.
/// This was locally defined as 245 ("256 total - 11 owner subkeys"),
/// the v1.0 derivation — under `o_cnt: 0` there are no owner subkeys
/// and the value is 255, which is what every record is actually built
/// with.
pub use rekindle_protocol::dht::community::member_registry::SLOTS_PER_SEGMENT;
pub use rekindle_protocol::dht::community::member_registry::SLOTS_PER_SEGMENT as REGISTRY_MAX_MEMBERS;

// ── Community metadata ──────────────────────────────────────────────
//
// `JoinPolicy` + `CommunityMetadata` lived here: the v1.0
// governance-manifest metadata subkey, carrying `owner_pseudonym`,
// `operator_pseudonyms`, `community_mailbox_key`, `join_inbox_key`
// and its published keypair. Every one of those is a coordinator
// concept flat governance removed, and the struct had no reader left
// on either track — `GovernanceState.metadata` is the community's
// name and description now, merged from `CommunityMeta` entries that
// each peer validates for itself.

// ── Channel types ───────────────────────────────────────────────────

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
    /// Member's profile DHT key for route resolution.
    ///
    /// Routes are published to the member's own profile record (which they
    /// own and can update independently). Other members read the profile to
    /// get the current route blob for RPC calls. This avoids the DFLT
    /// ownership problem where only the registry creator can write to the
    /// registry — each member controls their own profile.
    #[serde(default)]
    pub profile_dht_key: Option<String>,
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

// ── Channel message record ──────────────────────────────────────────

/// A message entry written to a channel record subkey.
///
/// Re-exported from the desktop track, which owns the definition. This
/// crate declared a second one that diverged in two ways, both of which
/// broke cross-track reads outright:
///
///   * `ciphertext` had no `#[serde(with = "base64_bytes")]`, so it
///     serialized as a JSON number array where the desktop track writes
///     a base64 string — a daemon-written message failed to parse there
///     with `invalid type: sequence, expected a string`.
///   * it lacked `attachment`, `flags`, `mentioned_pseudonyms` and
///     `mentioned_roles`, silently dropping file offers, voice-message
///     and @everyone/@here flags, and mention routing on any round trip.
pub use rekindle_protocol::dht::community::channel_record::ChannelMessage;

// ── Friend inbox types (DHT-based async friend requests) ────────────

/// A friend request written to the target's friend inbox DHT record.
/// The inbox is a DFLT(32) record with a published keypair — anyone
/// can write. The target's daemon polls their inbox for new requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendRequestEntry {
    /// Sender's Ed25519 public key (hex).
    pub sender_public_key: String,
    /// Sender's display name.
    pub display_name: String,
    /// Message attached to the request.
    pub message: String,
    /// Sender's profile DHT key.
    pub profile_dht_key: String,
    /// Sender's mailbox DHT key (for route resolution).
    pub mailbox_dht_key: String,
    /// Sender's friend inbox key (so the target can write responses back).
    pub sender_friend_inbox_key: String,
    /// Sender's friend inbox keypair hex (so the target can write responses).
    pub sender_friend_inbox_keypair_hex: String,
    /// Prekey bundle bytes for Signal session establishment.
    pub prekey_bundle: Vec<u8>,
    /// Epoch ms when the request was sent.
    pub sent_at: u64,
    /// Current status of this request.
    #[serde(default = "default_friend_request_status")]
    pub status: FriendRequestStatus,
}

fn default_friend_request_status() -> FriendRequestStatus {
    FriendRequestStatus::Pending
}

/// Status of a friend request in the DHT inbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FriendRequestStatus {
    /// Awaiting target's decision.
    Pending,
    /// Target accepted — includes their profile key and DM log key.
    Accepted {
        responder_profile_dht_key: String,
        responder_mailbox_dht_key: String,
        dm_log_key: String,
        dm_log_keypair_hex: String,
        accepted_at: u64,
    },
    /// Target rejected.
    Rejected { rejected_at: u64 },
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
