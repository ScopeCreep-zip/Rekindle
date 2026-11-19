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
#[serde(rename_all = "camelCase")]
pub struct CommunityOverview {
    pub anchor: String,
    pub name: String,
    pub description: String,
    pub member_count: u32,
    pub channel_count: u32,
    pub our_pseudonym: String,
    /// Set when we were removed: the community is read-only history.
    #[serde(default)]
    pub removed: Option<CommunityRemovalDisplay>,
}

/// Our own removal from a community (plan E3.2 S9 own removal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityRemovalDisplay {
    /// Banned, not only kicked.
    pub banned: bool,
    /// The remover's pseudonym, when the merged op names one.
    pub by: Option<String>,
    /// The remover's display name as known when the removal was first
    /// merged (its verified card or profile), when known.
    pub by_name: Option<String>,
    /// The reason the removal op gave (A/B R5-2; Element shows
    /// "X removed you: reason").
    pub reason: Option<String>,
}

/// Detailed community info for the info command and TUI view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityDetail {
    pub anchor: String,
    pub name: String,
    pub description: String,
    pub owner_pseudonym: String,
    pub created_at: u64,
    pub member_count: u32,
    pub channels: Vec<ChannelOverviewDisplay>,
    pub roles: Vec<RoleDisplay>,
    pub our_pseudonym: String,
    pub our_roles: Vec<String>,
    /// Set when we were removed: the community is read-only history.
    #[serde(default)]
    pub removed: Option<CommunityRemovalDisplay>,
}

// ── Channel ─────────────────────────────────────────────────────────────

/// Channel info for list and tree display.
#[derive(Debug, Clone, Serialize, Deserialize)]
// camelCase: rides in `GovernanceEvent` to the webview, which reads
// camelCase keys (plan C7.26).
#[serde(rename_all = "camelCase")]
pub struct ChannelOverviewDisplay {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub category_id: Option<String>,
    pub topic: String,
    pub mek_generation: u64,
    pub log_key: Option<String>,
    pub sort_order: u16,
    /// Minimum seconds between a member's messages, when slowmode is on.
    pub slowmode_seconds: Option<u32>,
}

/// Channel category, for grouping the channel tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
// camelCase: rides in `GovernanceEvent` to the webview, which reads
// camelCase keys (plan C7.26).
#[serde(rename_all = "camelCase")]
pub struct CategoryDisplay {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
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
    pub reply_to_message_id: Option<String>,
    pub mek_generation: u64,
    pub is_encrypted: bool,
    pub needs_mek: Option<u64>,
    /// The D26 gate hid it (an automod `BlurContent` match): it renders
    /// blurred.
    pub automod_blurred: bool,
    /// We sent it: only our own message is edited or deleted by us.
    #[serde(default)]
    pub is_own: bool,
    /// Its author edited it (plan E3.3): it shows "(edited)".
    #[serde(default)]
    pub edited: bool,
    /// The moderator who removed it (plan E3.3 M4): its body is gone and it
    /// shows "Removed by <name>".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed_by: Option<String>,
}

// ── DMs ─────────────────────────────────────────────────────────────────

/// DM conversation thread for the TUI inbox's display.
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
    /// Its client id, which `MessagesExpired` names (plan E7.1); `None`
    /// when the source carried none.
    pub client_id: Option<String>,
    /// A timeline notice (a disappearing-timer change, plan E7.1) drawn
    /// as a line of its own, not a message.
    pub notice: bool,
}

// ── Roles ───────────────────────────────────────────────────────────────

/// Role info for display.
///
/// `permissions` stays a plain `u64` deliberately. src-tauri's `RoleDto`
/// encodes the same field as a JSON *string*, guarding against
/// JavaScript's 53-bit integer precision — but that guard's deserializer
/// is `#[serde(untagged)]`, which needs `deserialize_any` and so cannot
/// decode under postcard, the daemon IPC format. Applying it here would
/// break the CLI the way `#[serde(flatten)]` did.
///
/// It is also not needed yet: `permissions::ALL` is `(1 << 51) - 1`,
/// below JavaScript's `2^53 - 1`, so no bit is lost today. **Two bits of
/// headroom.** Whoever adds permission bit 53 has to solve the JSON
/// encoding here without reaching for `untagged`.
#[derive(Debug, Clone, Serialize, Deserialize)]
// camelCase: rides in `GovernanceEvent` to the webview, which reads
// camelCase keys (plan C7.26).
#[serde(rename_all = "camelCase")]
pub struct RoleDisplay {
    /// The full 16-byte role id, 32 hex characters.
    pub id: String,
    pub name: String,
    pub color: u32,
    /// The role name's text colour on each base, readable on the tag
    /// (plan E7.3, SC 1.4.3); the Host works it out from `color`.
    pub tag_colours: crate::dto::community::RoleTagColours,
    pub permissions: u64,
    pub position: u32,
    /// Show members with this role in a separate sidebar group.
    pub hoist: bool,
    /// Whether `@role` pings everyone holding it.
    pub mentionable: bool,
    /// Members may take this role themselves, without a moderator.
    pub self_assignable: bool,
    /// Architecture §19.4 — when set, only one role per group may be
    /// active per member.
    pub exclusion_group: Option<String>,
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
    pub gossip_meshes: usize,
    pub gossip_mesh_peers: usize,
    pub unread_channels: usize,
    pub unread_dms: usize,
    pub unread_friend_requests: u32,
    pub dedup_entries: usize,
    pub dedup_suppressed: u64,

    // ── Social ──────────────────────────────────────────────
    pub community_count: usize,
    pub friend_count: usize,

    // ── Network detail ──────────────────────────────────────
    pub circuit_summary: CircuitSummary,

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
    /// The channel whose own key this is; `None` for the community key.
    pub channel_id: Option<String>,
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
    pub fn pass(
        id: impl Into<String>,
        category: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            category: category.into(),
            status: CheckStatus::Pass,
            value: value.into(),
            description: String::new(),
        }
    }

    /// Create a warning check.
    pub fn warn(
        id: impl Into<String>,
        category: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            category: category.into(),
            status: CheckStatus::Warn,
            value: value.into(),
            description: String::new(),
        }
    }

    /// Create a failing check.
    pub fn fail(
        id: impl Into<String>,
        category: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
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
