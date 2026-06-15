//! DHT record value types — structured data stored in DHT subkeys.
//!
//! Every DHT subkey value is frame-encoded (version + type + length + payload)
//! before writing, ensuring all stored data is self-describing and versioned.

use serde::{Deserialize, Serialize};
use rekindle_identity::{DhKey, IdentityRoot};

// ── Profile record (DFLT, 10 subkeys) ───────────────────────────────

pub const PROFILE_SUBKEY_DISPLAY_NAME: u32 = 0;
pub const PROFILE_SUBKEY_STATUS_MESSAGE: u32 = 1;
pub const PROFILE_SUBKEY_STATUS: u32 = 2;
pub const PROFILE_SUBKEY_AVATAR: u32 = 3;
pub const PROFILE_SUBKEY_GAME_INFO: u32 = 4;
pub const PROFILE_SUBKEY_PREKEY_BUNDLE: u32 = 5;
pub const PROFILE_SUBKEY_ROUTE_BLOB: u32 = 6;
pub const PROFILE_SUBKEY_METADATA: u32 = 7;
pub const PROFILE_SUBKEY_FRIEND_INBOX_KEY: u32 = 8;
pub const PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR: u32 = 9;
/// X25519 DH public key (32 bytes) for DM MEK wrapping ECDH.
/// Published so community members can wrap group DM MEKs for this peer
/// without needing a friendship prerequisite.
pub const PROFILE_SUBKEY_X25519_PUB: u32 = 10;
pub const PROFILE_SUBKEY_COUNT: u32 = 11;

pub const STATUS_ONLINE: u8 = 0;
pub const STATUS_AWAY: u8 = 1;
pub const STATUS_BUSY: u8 = 2;
pub const STATUS_OFFLINE: u8 = 3;
pub const STATUS_INVISIBLE: u8 = 4;

pub const FRIEND_INBOX_SUBKEY_COUNT: u32 = 32;
pub const JOIN_INBOX_SUBKEY_COUNT: u32 = 32;

// ── Friend list record (DFLT, 1 subkey) ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendEntry {
    pub public_key: IdentityRoot,
    pub nickname: Option<String>,
    pub group: Option<String>,
    pub added_at: u64,
    pub profile_dht_key: Option<String>,
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

pub const MANIFEST_METADATA: u32 = 0;
pub const MANIFEST_CHANNELS: u32 = 1;
pub const MANIFEST_CATEGORIES: u32 = 2;
pub const MANIFEST_ROLES: u32 = 3;
pub const MANIFEST_BANS: u32 = 4;
pub const MANIFEST_COORDINATOR: u32 = 5;
pub const MANIFEST_POLICIES: u32 = 6;
pub const MANIFEST_INVITES: u32 = 7;
pub const MANIFEST_THREADS: u32 = 8;
pub const MANIFEST_AUTOMOD: u32 = 9;
pub const MANIFEST_ONBOARDING: u32 = 10;
pub const MANIFEST_WELCOME: u32 = 11;
pub const MANIFEST_REGISTRY_SPINE: u32 = 12;
pub const MANIFEST_REACTIONS: u32 = 13;
pub const MANIFEST_AUDIT_LOG_KEY: u32 = 14;
pub const MANIFEST_PINS: u32 = 15;
pub const MANIFEST_EVENTS: u32 = 16;
pub const MANIFEST_SUBKEY_COUNT: u32 = 17;

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
pub const REGISTRY_OWNER_SUBKEY_COUNT: u16 = 0;
pub const REGISTRY_MEMBER_SUBKEY_COUNT: u16 = 1;
pub const REGISTRY_TOTAL_SUBKEY_COUNT: u16 = 255;

pub const REGISTRY_MAX_MEMBERS: u32 = 255;
pub const SLOTS_PER_SEGMENT: u32 = 255;

pub const CHANNEL_OWNER_SUBKEY_COUNT: u16 = 0;
pub const CHANNEL_MEMBER_SUBKEY_COUNT: u16 = 1;

// ── Community metadata ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JoinPolicy {
    #[default]
    AutoAllow,
    WaitingRoom,
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
    /// Ed25519 pseudonym hex of the community owner.
    pub owner_pseudonym: String,
    #[serde(default)]
    pub last_refreshed: u64,
    #[serde(default)]
    pub join_policy: JoinPolicy,
    #[serde(default)]
    pub community_mailbox_key: String,
    /// Ed25519 pseudonym hex values of operators.
    #[serde(default)]
    pub operator_pseudonyms: Vec<String>,
    #[serde(default = "default_max_members")]
    pub max_members: u32,
    #[serde(default = "default_mek_rotation_hours")]
    pub mek_rotation_interval_hours: u32,
    #[serde(default)]
    pub join_inbox_key: String,
    #[serde(default)]
    pub join_inbox_keypair_hex: String,
    /// Shared slot_seed for SMPL keypair derivation. ECDH-wrapped per
    /// member in the MEK vault's `wrapped_slot_seeds`. This field is
    /// populated at creation time and read by the operator for wrapping.
    /// Joiners receive the slot_seed via ECDH unwrap, never from this field.
    #[serde(default)]
    pub slot_seed_hex: String,
    #[serde(default)]
    pub mek_vault_key: String,
    #[serde(default)]
    pub mek_vault_keypair_hex: String,
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
    /// Per-channel member-log-index DHT record. DFLT(256).
    /// Subkey = member's slot_index. Value = that member's DhtLog key.
    /// Written by each member at join. Read by catch-up to discover
    /// other members' DhtLog keys for slow-path message retrieval.
    #[serde(default)]
    pub member_log_index_key: Option<String>,
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
    /// Ed25519 pseudonym public key hex. Community-scoped.
    pub pseudonym_key: String,
    pub display_name: String,
    pub role_ids: Vec<u32>,
    pub joined_at: u64,
    pub subkey_index: u32,
    #[serde(default)]
    pub onboarding_complete: bool,
    pub timeout_until: Option<u64>,
    /// X25519 DH public key hex for MEK wrapping.
    #[serde(default)]
    pub x25519_pub: Option<String>,
    /// Profile DHT key for route resolution.
    #[serde(default)]
    pub profile_dht_key: Option<String>,
    #[serde(default)]
    pub channel_records: std::collections::HashMap<String, String>,
    /// Ed25519 pseudonym signature over signing_bytes(). Proves the
    /// pseudonym holder authored this entry, not just anyone with the
    /// slot keypair (slot keypair proves SMPL write access, pseudonym
    /// signature proves identity).
    #[serde(default)]
    pub signature: Vec<u8>,
}

impl MemberSummary {
    /// Canonical bytes for pseudonym signature: all identity-bearing
    /// fields concatenated in a deterministic order.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(128);
        buf.extend_from_slice(b"rekindle-member-v1:");
        buf.extend_from_slice(self.pseudonym_key.as_bytes());
        buf.extend_from_slice(b":");
        buf.extend_from_slice(self.display_name.as_bytes());
        buf.extend_from_slice(b":");
        buf.extend_from_slice(&self.subkey_index.to_le_bytes());
        buf.extend_from_slice(b":");
        buf.extend_from_slice(&self.joined_at.to_le_bytes());
        buf
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberPresence {
    /// Ed25519 pseudonym public key hex. Community-scoped.
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
    /// Ed25519 pseudonym public key hex. Community-scoped.
    pub pseudonym_key: String,
    pub reason: Option<String>,
    /// Ed25519 pseudonym hex of the member who issued the ban.
    pub banned_by: String,
    pub banned_at: u64,
}

// ── Pending join queue (moderation queue subkey 5) ──────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingJoinEntry {
    /// Ed25519 pseudonym public key hex. Community-scoped.
    pub requester_pseudonym_hex: String,
    pub display_name: String,
    pub profile_dht_key: String,
    #[serde(default)]
    pub x25519_pub_hex: String,
    pub invite_code_hash: Option<String>,
    pub requested_at: u64,
    pub status: PendingJoinStatus,
    #[serde(default)]
    pub signature_hex: String,
}

impl PendingJoinEntry {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PendingJoinStatus {
    Pending,
    Approved { approved_by: String, approved_at: u64 },
    Rejected { rejected_by: String, reason: String, rejected_at: u64 },
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
    /// Ed25519 pseudonym hex of the member who rotated this MEK.
    pub rotator_pseudonym: String,
    /// X25519 DH public key of the rotator for ECDH unwrap.
    #[serde(default)]
    pub rotator_x25519_pub: Option<DhKey>,
    pub copies: Vec<EncryptedMekCopy>,
    /// ECDH-wrapped slot_seed per member. The operator wraps the shared
    /// slot_seed for each member using the same X25519 ECDH mechanism
    /// as MEK wrapping. Members unwrap during warm_mek_cache at join.
    /// Only present on the first vault entry (channel_id doesn't matter
    /// for slot_seed — it's community-wide).
    #[serde(default)]
    pub wrapped_slot_seeds: Vec<WrappedSlotSeed>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WrappedSlotSeed {
    pub target_pseudonym: String,
    pub encrypted_slot_seed: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedMekCopy {
    /// Ed25519 pseudonym hex of the target member.
    pub target_pseudonym: String,
    pub encrypted_mek: Vec<u8>,
}

// ── Signed governance payload ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceSubkeyPayload {
    pub author_pseudonym: String,
    pub subkey_index: u32,
    pub data: Vec<u8>,
    #[serde(default)]
    pub lamport_ts: u64,
    #[serde(default)]
    pub signature: Vec<u8>,
}

impl GovernanceSubkeyPayload {
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64 + self.data.len());
        buf.extend_from_slice(b"rekindle-governance-v1:");
        buf.extend_from_slice(self.author_pseudonym.as_bytes());
        buf.extend_from_slice(b":");
        buf.extend_from_slice(&self.subkey_index.to_le_bytes());
        buf.extend_from_slice(b":");
        buf.extend_from_slice(&self.lamport_ts.to_le_bytes());
        buf.extend_from_slice(b":");
        buf.extend_from_slice(&self.data);
        buf
    }
}

// ── Channel message record ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessage {
    pub sequence: u64,
    /// Ed25519 pseudonym public key hex. Community-scoped sender.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendRequestEntry {
    /// Sender's Ed25519 identity root.
    pub sender_public_key: IdentityRoot,
    pub display_name: String,
    pub message: String,
    pub profile_dht_key: String,
    pub mailbox_dht_key: String,
    pub sender_friend_inbox_key: String,
    pub sender_friend_inbox_keypair_hex: String,
    pub prekey_bundle: Vec<u8>,
    pub sent_at: u64,
    #[serde(default)]
    pub x25519_pub_hex: String,
    pub dm_log_key: String,
    pub dm_log_keypair_hex: String,
    pub signature_hex: String,
    pub status: FriendRequestStatus,
}

impl FriendRequestEntry {
    pub fn parse_inbox_data(data: &[u8]) -> std::result::Result<Vec<Self>, serde_json::Error> {
        serde_json::from_slice::<Vec<Self>>(data)
    }

    pub fn signature_content(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend_from_slice(b"rekindle-friend-request:");
        content.extend_from_slice(self.sender_public_key.as_bytes());
        content.extend_from_slice(b":");
        content.extend_from_slice(self.profile_dht_key.as_bytes());
        content.extend_from_slice(b":");
        content.extend_from_slice(self.dm_log_key.as_bytes());
        content.extend_from_slice(b":");
        content.extend_from_slice(&self.sent_at.to_le_bytes());
        if let FriendRequestStatus::Accepted {
            ref pqxdh_init_message,
            ref dm_smpl_record_key,
            ref dm_smpl_slot_seed_hex,
            ..
        } = self.status {
            content.extend_from_slice(b":pqxdh:");
            content.extend_from_slice(pqxdh_init_message);
            if let Some(ref rk) = dm_smpl_record_key {
                content.extend_from_slice(b":smpl:");
                content.extend_from_slice(rk.as_bytes());
            }
            if let Some(ref ss) = dm_smpl_slot_seed_hex {
                content.extend_from_slice(b":seed:");
                content.extend_from_slice(ss.as_bytes());
            }
        }
        content
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FriendRequestStatus {
    Pending,
    Accepted {
        responder_profile_dht_key: String,
        responder_mailbox_dht_key: String,
        responder_outbound_log_key: String,
        responder_outbound_log_keypair_hex: String,
        pqxdh_init_message: Vec<u8>,
        accepted_at: u64,
        /// SMPL DM record key — created by the lexicographically lower
        /// Ed25519 pubkey peer. None if this peer is the higher-key peer.
        dm_smpl_record_key: Option<String>,
        /// Hex-encoded 32-byte slot seed for SMPL keypair derivation.
        dm_smpl_slot_seed_hex: Option<String>,
    },
    Rejected { rejected_at: u64 },
}

// ── Pins (governance MANIFEST_PINS subkey 15) ──────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinEntry {
    pub channel_id: String,
    pub message_id: String,
    pub pinned_by: String,
    pub pinned_at: u64,
}

// ── Reactions (governance MANIFEST_REACTIONS subkey 13) ─────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionEntry {
    pub channel_id: String,
    pub message_id: String,
    pub emoji: String,
    pub reactor_pseudonym: String,
    pub created_at: u64,
}

// ── Audit log (governance MANIFEST_AUDIT_LOG_KEY subkey 14 + REGISTRY_OPS_LOG subkey 4) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogEntry {
    pub action: String,
    pub actor_pseudonym: String,
    pub target: Option<String>,
    pub timestamp: u64,
    pub details: Option<String>,
}

// ── Onboarding (governance MANIFEST_ONBOARDING subkey 10) ───────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingConfig {
    pub enabled: bool,
    pub steps: Vec<OnboardingStep>,
    pub require_rules_agreement: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingStep {
    pub title: String,
    pub description: String,
    pub questions: Vec<OnboardingQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingQuestion {
    pub id: String,
    pub prompt: String,
    pub kind: QuestionKind,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuestionKind {
    FreeText,
    MultipleChoice { options: Vec<String> },
    Checkbox,
}

// ── Welcome screen (governance MANIFEST_WELCOME subkey 11) ──────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WelcomeScreen {
    pub title: String,
    pub body: String,
    pub rules: Vec<String>,
}

// ── Invite blob (for friend invites) ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteBlob {
    pub public_key: IdentityRoot,
    pub display_name: String,
    pub mailbox_dht_key: String,
    pub profile_dht_key: String,
    pub route_blob: Vec<u8>,
    pub prekey_bundle: Vec<u8>,
    pub invite_id: Option<String>,
    pub signature: Vec<u8>,
}
