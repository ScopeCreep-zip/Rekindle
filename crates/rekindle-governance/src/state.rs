//! Materialized governance state — the output of CRDT merge.
//!
//! `GovernanceState` is the single source of truth for a community's
//! configuration: channels, roles, bans, metadata, MEK generation, etc.
//! It's computed deterministically from all member subkeys and cached
//! in memory for fast permission checks and UI rendering.

use std::collections::{HashMap, HashSet};

use rekindle_types::governance::{GuideStep, OnboardingQuestion, WelcomeChannel};
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

    /// Active expressions after OR-Set add/remove merge.
    pub expressions: HashMap<[u8; 16], ExpressionState>,

    /// Admin-deleted message IDs (tombstones).
    pub admin_deletes: HashSet<[u8; 16]>,

    /// The pseudonym that wrote the genesis entries (seq 1 / lamport 1).
    /// Used as the implicit community creator for "owner has all perms".
    pub creator: Option<PseudonymKey>,

    /// AutoMod rules (LWW per rule_id).
    pub automod_rules: HashMap<[u8; 16], AutoModRuleState>,

    /// Onboarding config (LWW).
    pub onboarding: Option<OnboardingState>,

    /// Welcome screen (LWW).
    pub welcome_screen: Option<WelcomeScreenState>,

    /// Track the highest lamport seen per author for ban-entry ordering.
    /// Key: (target_pseudonym) → highest lamport of ban/unban for that target.
    pub ban_lamports: HashMap<PseudonymKey, u64>,

    /// Segment expansions for communities >255 members (Plate Gates).
    /// Tracks additional governance and registry records.
    pub segments: Vec<SegmentState>,

    /// Active invites (InviteCreated minus InviteRevoked).
    /// Key: invite_id. Used for fast lookup by code_hash during join.
    pub invites: HashMap<[u8; 16], InviteState>,

    /// Highest remove lamport seen per expression_id.
    pub expression_remove_lamports: HashMap<[u8; 16], u64>,
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
    pub self_assignable: bool,
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
pub struct ExpressionState {
    pub name: String,
    pub kind: String,
    pub content_hash: String,
    pub inline_data: Option<Vec<u8>>,
    pub animated: bool,
    pub tags: Vec<String>,
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
    pub default_channels: Vec<ChannelId>,
    pub questions: Vec<OnboardingQuestion>,
    pub welcome_message: Option<String>,
    pub guide_steps: Vec<GuideStep>,
    pub lamport: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WelcomeScreenState {
    pub description: String,
    pub channels: Vec<WelcomeChannel>,
    pub lamport: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentState {
    pub segment_index: u32,
    pub registry_key: String,
    pub governance_key: String,
    pub slot_range_start: u32,
    pub slot_range_end: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InviteState {
    pub code_hash: String,
    pub max_uses: u32,
    pub expires_at: Option<u64>,
    pub encrypted_secrets: String,
    pub created_lamport: u64,
}
