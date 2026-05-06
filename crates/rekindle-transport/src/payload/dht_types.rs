//! DHT record value types — structured data stored in DHT subkeys.
//!
//! Every DHT subkey value is frame-encoded (version + type + length + payload)
//! before writing, ensuring all stored data is self-describing and versioned.

use serde::{Deserialize, Serialize};

// ── Profile record (DFLT, 10 subkeys) ───────────────────────────────

pub const PROFILE_SUBKEY_DISPLAY_NAME: u32 = 0;
pub const PROFILE_SUBKEY_STATUS_MESSAGE: u32 = 1;
pub const PROFILE_SUBKEY_STATUS: u32 = 2;
pub const PROFILE_SUBKEY_AVATAR: u32 = 3;
pub const PROFILE_SUBKEY_GAME_INFO: u32 = 4;
pub const PROFILE_SUBKEY_PREKEY_BUNDLE: u32 = 5;
pub const PROFILE_SUBKEY_ROUTE_BLOB: u32 = 6;
pub const PROFILE_SUBKEY_METADATA: u32 = 7;
/// Friend request inbox key — the DHT key of this user's friend inbox
/// record. Published so anyone can discover where to send requests.
pub const PROFILE_SUBKEY_FRIEND_INBOX_KEY: u32 = 8;
/// Hex-encoded keypair for the friend inbox. Published so anyone can
/// open the inbox for writing to submit friend requests.
pub const PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR: u32 = 9;
pub const PROFILE_SUBKEY_COUNT: u32 = 10;

/// Status byte encoding for profile subkey 2.
pub const STATUS_ONLINE: u8 = 0;
pub const STATUS_AWAY: u8 = 1;
pub const STATUS_BUSY: u8 = 2;
pub const STATUS_OFFLINE: u8 = 3;
pub const STATUS_INVISIBLE: u8 = 4;

/// Number of subkeys in the friend inbox DHT record (DFLT).
pub const FRIEND_INBOX_SUBKEY_COUNT: u32 = 32;

/// Number of subkeys in the community join inbox DHT record (DFLT).
pub const JOIN_INBOX_SUBKEY_COUNT: u32 = 32;

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

/// Registry subkey layout.
///
/// Subkeys 0-10: owner-controlled infrastructure (index, vault, policy,
/// metadata, operations, moderation, reserved for federation/audit/future).
/// Subkeys 11-255: per-member presence slots (245 members per segment).
///
/// Total: 256 subkeys per registry record.
pub const REGISTRY_MEMBER_INDEX: u32 = 0;
pub const REGISTRY_MEK_VAULT: u32 = 1;
pub const REGISTRY_POLICY: u32 = 2;
pub const REGISTRY_SCHEMA_VERSION: u32 = 3;
pub const REGISTRY_OPS_LOG: u32 = 4;
pub const REGISTRY_MODERATION_QUEUE: u32 = 5;
pub const REGISTRY_RESERVED_FEDERATION: u32 = 6;
pub const REGISTRY_RESERVED_AUDIT: u32 = 7;
pub const REGISTRY_RESERVED_8: u32 = 8;
pub const REGISTRY_RESERVED_9: u32 = 9;
pub const REGISTRY_RESERVED_10: u32 = 10;
pub const REGISTRY_OWNER_SUBKEY_COUNT: u16 = 11;
pub const REGISTRY_MEMBER_SUBKEY_COUNT: u16 = 1;
pub const REGISTRY_TOTAL_SUBKEY_COUNT: u16 = 256;

/// Maximum member slots per registry segment.
/// 256 total - 11 owner subkeys = 245 member presence slots.
pub const REGISTRY_MAX_MEMBERS: u32 = 245;
pub const SLOTS_PER_SEGMENT: u32 = 245;

/// Channel SMPL record constants.
pub const CHANNEL_OWNER_SUBKEY_COUNT: u16 = 0;
pub const CHANNEL_MEMBER_SUBKEY_COUNT: u16 = 1;

// ── Community metadata ──────────────────────────────────────────────

/// Join policy for community membership requests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JoinPolicy {
    /// Any user can join immediately. MEK is distributed automatically on approval.
    #[default]
    AutoAllow,
    /// Users enter a waiting room. An owner/admin/mod with admission privileges
    /// must approve before MEK distribution and channel access.
    WaitingRoom,
    /// Users must have a valid invite code. Approval may still be required
    /// depending on the invite's configuration.
    InviteOnly,
}

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
    /// How new members are admitted to the community.
    #[serde(default)]
    pub join_policy: JoinPolicy,
    /// Community mailbox DHT key — stores the community's route blob for
    /// receiving join requests and governance operations via Veilid RPC.
    /// The mailbox is owned by the governance keypair.
    #[serde(default)]
    pub community_mailbox_key: String,
    /// Pseudonyms of members who hold the governance keypair and can
    /// execute governance writes. The owner is always in this set.
    /// Additional operators provide high-availability when the owner is offline.
    #[serde(default)]
    pub operator_pseudonyms: Vec<String>,
    /// Maximum members per registry segment (default 245).
    #[serde(default = "default_max_members")]
    pub max_members: u32,
    /// Automatic MEK rotation interval in hours. 0 = manual only.
    /// Default 168 (7 days).
    #[serde(default = "default_mek_rotation_hours")]
    pub mek_rotation_interval_hours: u32,
    /// DHT key for the join request inbox (DFLT, 32 subkeys).
    /// Prospective members write join requests here using the published
    /// keypair. The owner's daemon reads and processes them asynchronously.
    /// No RPC, no routes, no timeouts — pure DHT.
    #[serde(default)]
    pub join_inbox_key: String,
    /// Hex-encoded keypair (64 bytes: 32 pub + 32 secret) for the join inbox.
    /// Published so any prospective member can open the record for writing.
    #[serde(default)]
    pub join_inbox_keypair_hex: String,
}

fn default_max_members() -> u32 { 245 }
fn default_mek_rotation_hours() -> u32 { 168 }

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
    /// Member's profile DHT key for route resolution.
    ///
    /// Routes are published to the member's own profile record (which they
    /// own and can update independently). Other members read the profile to
    /// get the current route blob for RPC calls. This avoids the DFLT
    /// ownership problem where only the registry creator can write to the
    /// registry — each member controls their own profile.
    #[serde(default)]
    pub profile_dht_key: Option<String>,
    /// Per-channel DhtLog spine keys owned by this member.
    /// Maps channel_id → DhtLog spine key (append-only log, member-owned).
    /// Each member creates and owns their own DhtLog per channel.
    /// No shared secrets — write access is per-member, enforced by Veilid.
    #[serde(default)]
    pub channel_records: std::collections::HashMap<String, String>,
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

// ── Pending join queue (moderation queue subkey 5) ──────────────────

/// A pending join request in the moderation queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingJoinEntry {
    pub requester_pseudonym_hex: String,
    pub display_name: String,
    pub profile_dht_key: String,
    pub invite_code_hash: Option<String>,
    pub requested_at: u64,
    pub status: PendingJoinStatus,
    /// Ed25519 signature over the canonical content bytes, signed with
    /// the requester's pseudonym signing key. Verified by process_inbox
    /// before approval. Empty for legacy entries (pre-signature migration).
    #[serde(default)]
    pub signature_hex: String,
}

impl PendingJoinEntry {
    /// Canonical bytes for signature verification.
    /// Covers identity-binding fields only (not status/timestamp which change).
    pub fn signature_content(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend_from_slice(b"rekindle-join-request-v1:");
        content.extend_from_slice(self.requester_pseudonym_hex.as_bytes());
        content.extend_from_slice(b":");
        content.extend_from_slice(self.profile_dht_key.as_bytes());
        content.extend_from_slice(b":");
        content.extend_from_slice(self.display_name.as_bytes());
        content
    }
}

/// Status of a pending join request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PendingJoinStatus {
    Pending,
    Approved { approved_by: String, approved_at: u64 },
    Rejected { rejected_by: String, reason: String, rejected_at: u64 },
    /// Member is leaving the community. Written to the join inbox by the
    /// leaving member so the owner can process cleanup + rekey.
    Left { left_at: u64 },
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
