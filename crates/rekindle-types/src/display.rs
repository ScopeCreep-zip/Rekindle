//! Display-ready types for the daemon→client boundary.
//!
//! These structs cross the daemon→CLI/TUI boundary via JSON serialization.
//! Each type's shape MUST match the return type of the corresponding
//! ChatService method exactly. A field mismatch causes silent deserialization
//! failure — the TUI renders empty panels with zero error feedback.
//!
//! Types that wrap DHT types use `serde(flatten)` so the DHT fields appear
//! at the top level alongside enrichment fields (unread counts, presence).
//! All DHT types use `serde(rename_all = "camelCase")` — display types that
//! flatten them inherit camelCase field names in JSON.
//!
//! Source of truth for each type is documented inline with the ChatService
//! method that produces it.

use serde::{Deserialize, Serialize};

// ── Community ───────────────────────────────────────────────────────────

/// Overview of a joined community for list display.
///
/// Source: `rekindle_chat::community::membership::CommunitySummary`
/// Produced by: `ChatService::list_communities()`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityOverview {
    pub governance_key: String,
    pub name: String,
    pub pseudonym: String,
    pub is_operator: bool,
}

/// Detailed community info for the info command and TUI view.
///
/// Source: `rekindle_chat::service::delegate::community::CommunityInfoResult`
/// Produced by: `ChatService::community_info()`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityDetail {
    pub governance_key: String,
    pub name: String,
    pub description: String,
    pub owner_pseudonym: String,
    pub join_policy: String,
    #[serde(default)]
    pub created_at: u64,
    pub channels: Vec<crate::dht_types::ChannelEntry>,
    /// Channel UUID → unread message count. Only populated channels included.
    #[serde(default)]
    pub channel_unreads: std::collections::HashMap<String, u32>,
    pub roles: Vec<crate::dht_types::RoleEntry>,
    pub members: Vec<MemberWithPresence>,
    pub channel_count: usize,
    pub role_count: usize,
    #[serde(default)]
    pub member_count: usize,
    pub our_pseudonym: String,
    pub is_operator: bool,
    #[serde(default)]
    pub locked_down: bool,
}

// ── Channel ─────────────────────────────────────────────────────────────

/// Channel entry enriched with unread count from subscription state.
///
/// Source: `rekindle_chat::service::delegate::governance::ChannelWithUnread`
/// Produced by: `ChatService::list_channels()`
/// Uses `serde(flatten)` — all `ChannelEntry` camelCase fields appear at
/// the top level alongside `unread_count`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelOverviewDisplay {
    #[serde(flatten)]
    pub channel: crate::dht_types::ChannelEntry,
    #[serde(default)]
    pub unread_count: u32,
}

/// Community member enriched with live presence and resolved role name.
///
/// Source: `MemberSummary` from DHT + `PresenceInfo` from pipeline state.
/// Produced by: `ChatService::community_info()`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberWithPresence {
    #[serde(flatten)]
    pub member: crate::dht_types::MemberSummary,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub role_name: Option<String>,
}

/// Delivery status for messages sent by the local user.
///
/// Messages from remote peers are always `Confirmed` (they arrived via
/// the network, so durability is already proven). Self-sent messages
/// transition: `Sending` → `Confirmed` (DHT write succeeded) or
/// `Sending` → `Failed` (DHT write or gossip broadcast failed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryStatus {
    Sending,
    Confirmed,
    Failed,
}

/// Channel message for history display.
///
/// Source: `rekindle_storage::messages::ChannelRecord`
/// Produced by: `ChatService::channel_history()`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecryptedMessageDisplay {
    pub message_id: String,
    pub sequence: u64,
    pub author_pseudonym: String,
    #[serde(default)]
    pub author_display_name: String,
    pub body: String,
    pub timestamp: u64,
    #[serde(default)]
    pub reply_to_sequence: Option<u64>,
    pub mek_generation: u64,
    #[serde(default)]
    pub is_encrypted: bool,
    #[serde(default)]
    pub needs_mek: Option<u64>,
    #[serde(default = "default_delivery_status")]
    pub delivery_status: DeliveryStatus,
    #[serde(default)]
    pub thread_id: Option<String>,
}

fn default_delivery_status() -> DeliveryStatus {
    DeliveryStatus::Confirmed
}

// ── Friends / DMs ───────────────────────────────────────────────────────

/// Friend for list display.
///
/// Source: `rekindle_chat::service::delegate::friendship::FriendSummary`
/// Produced by: `ChatService::list_friends()`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendDisplay {
    pub public_key: String,
    pub display_name: String,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub status_message: String,
    #[serde(default)]
    pub last_seen_ms: Option<u64>,
    #[serde(default)]
    pub profile_dht_key: Option<String>,
    #[serde(default)]
    pub has_route: bool,
}

fn default_status() -> String {
    "unknown".to_string()
}

/// DM conversation for inbox display.
///
/// Source: `rekindle_types::dm_store::DmConversation`
/// Produced by: `ChatService::dm_inbox()`
///
/// Uses serde aliases so the TUI can deserialize from DmConversation JSON.
/// The daemon enriches record_key with the peer's Ed25519 pubkey before
/// serialization, so peer_key contains the pubkey hex (not a VLD0: DHT key).
/// initiator_pseudonym is enriched with the friend display name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmThreadDisplay {
    #[serde(alias = "record_key")]
    pub peer_key: String,
    #[serde(alias = "initiator_pseudonym", alias = "display_name")]
    pub peer_name: String,
    #[serde(default)]
    pub last_message_at: Option<u64>,
    #[serde(default)]
    pub unread_count: u32,
    #[serde(default)]
    pub is_group: bool,
    #[serde(default)]
    pub messages: Vec<DmMessageDisplay>,
}

/// Single DM message for display.
///
/// Source: `rekindle_types::dm_store::DmMessageRecord`
/// Produced by: `ChatService::dm_thread()`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmMessageDisplay {
    #[serde(default)]
    pub sender_key: String,
    #[serde(default, alias = "sender_pseudonym")]
    pub sender_name: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub timestamp: u64,
    #[serde(default)]
    pub is_self: bool,
    #[serde(default)]
    pub sequence: u64,
}

// ── Pending Friend Requests ─────────────────────────────────────────────

/// Pending friend request for TUI friend list view.
///
/// Source: `rekindle_types::session_types::PendingFriendRequest`
/// Produced by: `ChatService::list_pending_requests()`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendRequestDisplay {
    #[serde(alias = "sender_public_key")]
    pub from_key: String,
    pub display_name: String,
    #[serde(default)]
    pub message: String,
    #[serde(default, alias = "received_at")]
    pub sent_at: u64,
}

// ── Status ──────────────────────────────────────────────────────────────

/// Transport-layer point-in-time status snapshot.
///
/// Used by `rekindle-transport-veilid` for `status_snapshot()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportSnapshot {
    pub attachment: String,
    pub is_attached: bool,
    pub public_internet_ready: bool,
    pub uptime_secs: u64,
    pub peer_count: usize,
    pub route_allocated: bool,
    pub route_age_secs: Option<u64>,
}

/// Complete daemon status — single response type for `DaemonRequest::Status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusSnapshot {
    // ── Lifecycle ────────────────────────────────────────────
    pub state: String,
    pub has_identity: bool,
    pub identity_public_key: Option<String>,
    pub identity_display_name: Option<String>,

    // ── Transport ───────────────────────────────────────────
    pub attachment: String,
    pub is_attached: bool,
    pub public_internet_ready: bool,
    pub uptime_secs: u64,
    pub peer_count: usize,
    pub route_allocated: bool,
    pub route_age_secs: Option<u64>,

    // ── Subscription system ─────────────────────────────────
    pub active_watches: usize,
    pub gossip_meshes: usize,
    pub gossip_mesh_peers: usize,
    pub unread_channels: usize,
    pub unread_dms: usize,
    pub unread_friend_requests: u32,
    pub dedup_entries: usize,
    pub dedup_suppressed: u64,
    pub poll_loop_active: bool,
    pub renewal_loop_active: bool,

    // ── Social ──────────────────────────────────────────────
    pub community_count: usize,
    pub friend_count: usize,

    // ── Network detail ──────────────────────────────────────
    pub circuit_summary: CircuitSummary,

    // ── Bulk transfer plane ────────────────────────────────
    #[serde(default)]
    pub bulk_frames_sent: u64,
    #[serde(default)]
    pub bulk_frames_received: u64,
    #[serde(default)]
    pub bulk_bytes_sent: u64,
    #[serde(default)]
    pub bulk_bytes_received: u64,
    #[serde(default)]
    pub bulk_transfers_active: usize,

    // ── Diagnostic checks ───────────────────────────────────
    pub checks: Vec<Check>,
}

/// Summary of peer health states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitSummary {
    pub total: usize,
    pub healthy: usize,
    pub degraded: usize,
    pub circuit_open: usize,
}

/// Point-in-time snapshot of a single peer for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerSnapshot {
    pub key: String,
    #[serde(default)]
    pub key_short: String,
    #[serde(default)]
    pub has_route: bool,
    #[serde(default)]
    pub route_age_secs: u64,
    #[serde(default)]
    pub circuit_open: bool,
    #[serde(default)]
    pub failure_count: u32,
}

// ── Crypto ──────────────────────────────────────────────────────────────

/// Display-ready snapshot of a single cached MEK entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MekCacheEntrySnapshot {
    pub channel_id: String,
    pub generation: u64,
    #[serde(default)]
    pub age_secs: u64,
}

// ── Doctor ──────────────────────────────────────────────────────────────

/// Status of a single diagnostic check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

/// A single diagnostic check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub category: String,
    pub status: CheckStatus,
    pub value: String,
    pub description: String,
}

impl Check {
    pub fn pass(id: impl Into<String>, category: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            category: category.into(),
            status: CheckStatus::Pass,
            value: value.into(),
            description: String::new(),
        }
    }

    pub fn warn(id: impl Into<String>, category: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            category: category.into(),
            status: CheckStatus::Warn,
            value: value.into(),
            description: String::new(),
        }
    }

    pub fn fail(id: impl Into<String>, category: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            category: category.into(),
            status: CheckStatus::Fail,
            value: value.into(),
            description: String::new(),
        }
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
}
