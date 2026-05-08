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

    /// Community-wide default notification level (architecture §17.1).
    /// `None` until an admin sets one; falls back to "all" at the
    /// resolver. LWW per (community).
    pub notification_default: Option<NotificationDefaultState>,

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

    /// Pinned attachment IDs — exempt from LRU eviction in local file
    /// caches (Lost Cargo, architecture §28.9 line 3283). Driven by
    /// `GovernanceEntry::AttachmentPinned` LWW merge.
    pub pinned_attachments: HashSet<[u8; 16]>,

    /// Highest lamport seen per attachment for `AttachmentPinned` LWW —
    /// later writes overwrite earlier ones for the same attachment_id.
    pub attachment_pin_lamports: HashMap<[u8; 16], u64>,

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

    /// Plate Gate lazy channel records (architecture §15.4): per
    /// `(channel_id, segment_index)` SMPL record key. Populated by
    /// `ChannelSegmentLinked` governance entries written by the first
    /// segment-N member to message in that channel. Segment 0's record
    /// keys live in `channels[id].record_key` and are NOT mirrored here.
    pub channel_segment_records: HashMap<(ChannelId, u32), ChannelSegmentRecord>,

    /// Active invites (InviteCreated minus InviteRevoked).
    /// Key: invite_id. Used for fast lookup by code_hash during join.
    pub invites: HashMap<[u8; 16], InviteState>,

    /// Highest remove lamport seen per expression_id.
    pub expression_remove_lamports: HashMap<[u8; 16], u64>,

    /// Architecture §17.1 + §20.6 — community-wide policy (default
    /// notification level + raid-protection thresholds). LWW; `None`
    /// until the first `CommunityPolicy` governance entry is merged.
    pub community_policy: Option<CommunityPolicyState>,
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
    pub forum_tags: Option<Vec<String>>,
    pub slowmode_seconds: Option<u32>,
    pub nsfw: Option<bool>,
    /// Architecture §10.8 — text-in-voice association. When set, this
    /// channel is the text companion of the named voice channel; the
    /// frontend hides it unless the local member is connected to that
    /// voice channel. Set at creation via `ChannelCreated` and
    /// preserved across merges.
    pub parent_voice_channel_id: Option<ChannelId>,
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
    /// Architecture §19.4 — see `GovernanceEntry::RoleDefinition`.
    pub exclusion_group: Option<String>,
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

/// Architecture §17.1 tier-1 default notification level for a community.
/// One of "all" | "mentions" | "nothing"; the resolver in
/// `services/community/notifications.rs` falls back to "all" when this
/// is unset. LWW per community.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationDefaultState {
    pub level: String,
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
    pub thread_type: String,
    pub record_key: Option<String>,
    pub invited: Vec<PseudonymKey>,
    pub forum_tag: Option<String>,
    pub auto_archive_seconds: u64,
    pub creator: PseudonymKey,
    pub created_lamport: u64,
    pub archived_lamport: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventState {
    pub name: String,
    pub description: Option<String>,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub channel_id: Option<ChannelId>,
    pub cover_image_ref: Option<String>,
    pub creator_pseudonym: Option<rekindle_types::id::PseudonymKey>,
    pub recurrence: Option<rekindle_types::event::RecurrenceRule>,
    pub location: Option<rekindle_types::event::EventLocation>,
    pub status: rekindle_types::event::EventStatus,
    pub lamport: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpressionState {
    pub name: String,
    pub kind: String,
    pub content_hash: String,
    /// Architecture §18.4 — Lost Cargo manifest. Receivers fetch the
    /// asset bytes via the existing RequestAttachment pipeline.
    /// Replaces the legacy `inline_data` (which exceeded the 32 KiB
    /// SMPL subkey limit for any non-trivial expression).
    pub attachment: Option<rekindle_types::attachment::AttachmentOffer>,
    pub animated: bool,
    pub tags: Vec<String>,
    /// Architecture §18.3 — present only on `kind == "soundboard"`
    /// entries. Emoji/sticker entries leave this `None`.
    pub sound_meta: Option<rekindle_types::expression::SoundboardMeta>,
    /// Architecture §18.1 line 2455 — author of the asset.
    pub creator_pseudonym: Option<PseudonymKey>,
    /// Architecture §18.1 line 2456 — wall-clock seconds at upload.
    pub created_at: Option<u64>,
    /// Architecture §18.1 line 2459 — gates `USE_EXTERNAL_EMOJIS`.
    pub available_to_peers: bool,
    pub lamport: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimeoutState {
    pub duration_seconds: u64,
    pub started_at: u64,
    pub lamport: u64,
}

/// Architecture §10.7 + §20.6 — merged-state form of `CommunityPolicy`.
#[derive(Debug, Clone, PartialEq)]
pub struct CommunityPolicyState {
    pub policy_text: Option<String>,
    pub max_joins_per_interval: u32,
    pub join_interval_seconds: u32,
    pub lamport: u64,
}

impl CommunityPolicyState {
    /// Defaults from architecture §20.6 — used when no `CommunityPolicy`
    /// entry has been merged yet.
    pub const DEFAULT_MAX_JOINS_PER_INTERVAL: u32 = 20;
    pub const DEFAULT_JOIN_INTERVAL_SECONDS: u32 = 600;
}

impl TimeoutState {
    /// Check if this timeout has expired given a current unix timestamp.
    pub fn is_expired(&self, now_unix: u64) -> bool {
        now_unix >= self.started_at.saturating_add(self.duration_seconds)
    }
}

impl GovernanceState {
    /// Architecture §9.3 line 1946 — return the highest `RoleState.position`
    /// across all roles assigned to `member`. Higher position = higher rank
    /// (Discord convention). Members with no role assignments default to 0
    /// (the @everyone-equivalent floor).
    ///
    /// Used by `validate_write` to enforce role hierarchy on Ban / Timeout /
    /// RoleAssignment / RoleUnassignment entries: a writer can only act on a
    /// target whose max position is strictly less than the writer's.
    pub fn member_max_position(&self, member: &PseudonymKey) -> u32 {
        self.role_assignments
            .get(member)
            .map(|role_ids| {
                role_ids
                    .iter()
                    .filter_map(|rid| self.roles.get(rid))
                    .map(|role| role.position)
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0)
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

/// Plate Gate (architecture §15.4): one row per `(channel_id, segment_index)`
/// in `channel_segment_records`. The record_key points at the SMPL DHT
/// record holding messages from members of `segment_index` in that channel.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelSegmentRecord {
    pub record_key: String,
    /// Lamport of the `ChannelSegmentLinked` entry that announced this
    /// record. Used for LWW disambiguation when two members race to
    /// write the first message in a previously-empty (channel, segment)
    /// pair (architecture §6.4 slot collision pattern, repurposed for
    /// channel-segment creation).
    pub linked_lamport: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InviteState {
    pub code_hash: String,
    pub max_uses: u32,
    pub expires_at: Option<u64>,
    pub encrypted_secrets: String,
    pub created_lamport: u64,
    /// M10.3 — the inviter's pseudonym, populated from the writing
    /// governance subkey at merge time. Used for the per-inviter
    /// active-invite quota in `invite_quota.rs` and for audit-log
    /// attribution.
    pub creator_pseudonym: PseudonymKey,
}
