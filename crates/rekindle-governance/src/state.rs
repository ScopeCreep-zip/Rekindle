//! Materialized governance state — the output of CRDT merge.
//!
//! `GovernanceState` is the single source of truth for a community's
//! configuration: channels, roles, bans, metadata, MEK generation, etc.
//! It's computed deterministically from all member subkeys and cached
//! in memory for fast permission checks and UI rendering.

use std::collections::{HashMap, HashSet};

use rekindle_types::id::{CategoryId, ChannelId, EventId, PseudonymKey, RoleId, ThreadId};

/// The merged governance state — produced by `merge::merge()`,
/// consumed by `permissions::compute_permissions()` and `validate::validate_write()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GovernanceState {
    /// Active channels (ChannelCreated minus ChannelArchived).
    pub channels: HashMap<ChannelId, ChannelState>,

    /// Role definitions (LWW per role_id).
    pub roles: HashMap<RoleId, RoleState>,

    /// Role assignments: member → set of assigned role_ids.
    pub role_assignments: HashMap<PseudonymKey, HashSet<RoleId>>,

    /// Currently banned pseudonyms.
    pub bans: HashSet<PseudonymKey>,

    /// Currently timed-out members with expiry info.
    pub timeouts: HashMap<PseudonymKey, TimeoutState>,

    /// Current MEK generation (Max-Register — highest wins).
    pub mek_generation: u64,

    /// Community metadata (LWW — highest lamport replaces all).
    pub metadata: Option<MetadataState>,

    /// Active categories (OR-Set minus archived).
    pub categories: HashMap<CategoryId, CategoryState>,

    /// Permission overwrites per (channel, target). LWW per key.
    pub overwrites: HashMap<(ChannelId, String), OverwriteState>,

    /// Active threads (OR-Set).
    pub threads: HashMap<ThreadId, ThreadState>,

    /// Active events (LWW per event_id).
    pub events: HashMap<EventId, EventState>,

    /// Admin-deleted message IDs (tombstones).
    pub admin_deletes: HashSet<[u8; 16]>,

    /// The pseudonym that wrote the genesis entries (seq 1 / lamport 1).
    /// Used as the implicit community creator for "owner has all perms".
    pub creator: Option<PseudonymKey>,

    /// AutoMod rules (LWW per rule_id).
    pub automod_rules: HashMap<[u8; 16], AutoModRuleState>,

    /// Onboarding config (LWW).
    pub onboarding: Option<OnboardingState>,

    /// Track the highest lamport seen per author for ban-entry ordering.
    /// Key: (target_pseudonym) → highest lamport of ban/unban for that target.
    pub ban_lamports: HashMap<PseudonymKey, u64>,
}

// ── Sub-states ──

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelState {
    pub name: String,
    pub channel_type: String,
    pub record_key: String,
    pub category_id: Option<CategoryId>,
    pub position: u32,
    pub topic: Option<String>,
    pub slowmode_seconds: Option<u32>,
    pub nsfw: Option<bool>,
    /// Lamport of the ChannelCreated entry (for archive comparison).
    pub created_lamport: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoleState {
    pub name: String,
    pub permissions: u64,
    pub position: u32,
    pub color: u32,
    pub hoist: bool,
    pub mentionable: bool,
    pub lamport: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetadataState {
    pub name: String,
    pub description: Option<String>,
    pub icon_hash: Option<String>,
    pub banner_hash: Option<String>,
    pub lamport: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CategoryState {
    pub name: String,
    pub position: u32,
    pub created_lamport: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverwriteState {
    pub target_type: String,
    pub allow: u64,
    pub deny: u64,
    pub lamport: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadState {
    pub parent_channel_id: ChannelId,
    pub name: String,
    pub record_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventState {
    pub name: String,
    pub description: Option<String>,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub channel_id: Option<ChannelId>,
    pub lamport: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimeoutState {
    pub duration_seconds: u64,
    pub started_at: u64,
    pub lamport: u64,
}

impl TimeoutState {
    /// Check if this timeout has expired given a current unix timestamp.
    pub fn is_expired(&self, now_unix: u64) -> bool {
        now_unix >= self.started_at.saturating_add(self.duration_seconds)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AutoModRuleState {
    pub name: String,
    pub enabled: bool,
    pub trigger_json: String,
    pub action: String,
    pub lamport: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OnboardingState {
    pub enabled: bool,
    pub mode: String,
    pub welcome_message: Option<String>,
    pub lamport: u64,
}
