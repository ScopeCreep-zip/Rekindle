//! Display-ready types for the IPC boundary.
//!
//! These structs cross the daemon→CLI boundary via JSON serialization.
//! They are the single source of truth — `rekindle-transport::QueryEngine`
//! returns them, `rekindle-node` dispatch serializes them into `IpcResponse`,
//! and `rekindle-cli` deserializes them for rendering.
//!
//! All types: `Serialize + Deserialize + Clone + Debug`. No Veilid types,
//! no locks, no async — pure display-ready primitives.

use serde::{Deserialize, Serialize};

// ── Community ───────────────────────────────────────────────────────────

/// Overview of a joined community for list display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityOverview {
    pub governance_key: String,
    pub name: String,
    pub description: String,
    pub member_count: u32,
    pub channel_count: u32,
    pub our_pseudonym: String,
}

/// Detailed community info for the info command and TUI view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityDetail {
    pub governance_key: String,
    pub name: String,
    pub description: String,
    pub owner_pseudonym: String,
    pub created_at: u64,
    pub member_count: u32,
    pub channels: Vec<ChannelOverviewDisplay>,
    pub roles: Vec<RoleDisplay>,
    pub our_pseudonym: String,
    pub our_roles: Vec<u32>,
    /// Community members for peer list display.
    #[serde(default)]
    pub members: Vec<MemberPresence>,
}

// ── Channel ─────────────────────────────────────────────────────────────

/// Channel info for list and tree display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelOverviewDisplay {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub category_id: Option<String>,
    pub topic: String,
    pub mek_generation: u64,
    pub log_key: Option<String>,
    pub sort_order: u16,
}

/// Delivery status for messages sent by the local user.
///
/// Messages from remote peers are always `Confirmed` (they arrived via
/// the network, so durability is already proven). Self-sent messages
/// transition: `Sending` → `Confirmed` (DHT write succeeded) or
/// `Sending` → `Failed` (DHT write or gossip broadcast failed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryStatus {
    /// Message is being written to DHT / sent via app_message.
    Sending,
    /// Message is durably persisted on the network (DHT write confirmed).
    Confirmed,
    /// Send failed — the message was not delivered.
    Failed,
}

/// Decrypted channel message for history display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecryptedMessageDisplay {
    pub message_id: String,
    pub sequence: u64,
    pub author_pseudonym: String,
    pub author_display_name: String,
    pub body: String,
    pub timestamp: u64,
    pub reply_to_sequence: Option<u64>,
    pub mek_generation: u64,
    pub is_encrypted: bool,
    pub needs_mek: Option<u64>,
    /// Delivery status for self-sent messages. Remote messages are always `Confirmed`.
    pub delivery_status: DeliveryStatus,
}

// ── Friends / DMs ───────────────────────────────────────────────────────

/// Friend with resolved display name and presence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendDisplay {
    pub public_key: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub status: String,
    pub status_message: String,
    pub last_seen_ms: Option<u64>,
    pub profile_dht_key: Option<String>,
    pub has_route: bool,
}

/// DM conversation thread for inbox display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmThreadDisplay {
    pub peer_key: String,
    pub peer_name: String,
    pub last_message_at: u64,
    pub unread_count: u32,
    pub messages: Vec<DmMessageDisplay>,
}

/// Single DM message for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmMessageDisplay {
    pub sender_key: String,
    pub sender_name: String,
    pub body: String,
    pub timestamp: u64,
    pub is_self: bool,
}

// ── Channel ─────────────────────────────────────────────────────────────

/// Channel info for TUI community info and channel tree views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelDisplay {
    pub channel_id: String,
    pub name: String,
    pub kind: String,
    pub topic: String,
    pub unread_count: u32,
}

// ── Member Presence ─────────────────────────────────────────────────────

/// Member presence for TUI peer list and community info views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberPresence {
    pub pseudonym: String,
    pub display_name: Option<String>,
    pub status: String,
    pub role_name: Option<String>,
}

// ── Friend Request ──────────────────────────────────────────────────────

/// Pending friend request for TUI friend list view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendRequestDisplay {
    pub from_key: String,
    pub display_name: String,
    pub message: String,
    pub sent_at: u64,
}

// ── Roles ───────────────────────────────────────────────────────────────

/// Role info for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleDisplay {
    pub id: u32,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    pub position: i32,
}

// ── Status ──────────────────────────────────────────────────────────────

/// Transport-layer point-in-time status snapshot.
///
/// Produced by `TransportNode::status_snapshot()`. Consumed by the daemon's
/// `handle_status` to compose the full `StatusSnapshot` for IPC delivery.
/// Contains only what the transport node knows about itself.
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

/// Complete daemon status — single response type for `IpcRequest::Status`.
///
/// Serves all consumers:
/// - CLI `rekindle status` → compact rendering (lifecycle + transport fields)
/// - CLI `rekindle status --doctor` → expanded rendering (+ checks)
/// - CLI `rekindle status --watch` → streaming refresh of same data
/// - TUI dashboard "Node" pane → compact (lifecycle + transport)
/// - TUI doctor view → full (all fields + checks)
///
/// The daemon always returns the complete snapshot. Renderers decide depth.
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
    /// Full check list. CLI `--doctor` and TUI doctor view render these.
    /// Compact mode ignores this field during rendering.
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
    pub key_short: String,
    pub has_route: bool,
    pub route_age_secs: u64,
    pub circuit_open: bool,
    pub failure_count: u32,
}

// ── Crypto ──────────────────────────────────────────────────────────────

/// Display-ready snapshot of a single cached MEK entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MekCacheEntrySnapshot {
    pub channel_id: String,
    pub generation: u64,
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
    /// Create a passing check.
    pub fn pass(id: impl Into<String>, category: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            category: category.into(),
            status: CheckStatus::Pass,
            value: value.into(),
            description: String::new(),
        }
    }

    /// Create a warning check.
    pub fn warn(id: impl Into<String>, category: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            category: category.into(),
            status: CheckStatus::Warn,
            value: value.into(),
            description: String::new(),
        }
    }

    /// Create a failing check.
    pub fn fail(id: impl Into<String>, category: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            category: category.into(),
            status: CheckStatus::Fail,
            value: value.into(),
            description: String::new(),
        }
    }

    /// Add a description (remediation hint) to a check.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
}
