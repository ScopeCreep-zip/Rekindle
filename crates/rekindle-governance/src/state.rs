//! Materialized governance state — the output of CRDT merge.
//!
//! `GovernanceState` is the single source of truth for a community's
//! configuration: channels, roles, bans, metadata, MEK generation, etc.
//! It's computed deterministically from all member subkeys and cached
//! in memory for fast permission checks and UI rendering.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::op::OpStamp;
use rekindle_types::governance::{AdmissionMode, GuideStep, OnboardingQuestion, WelcomeChannel};
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

    /// Counted members, from their `MemberJoined` ops (plan E3.2).
    pub members: HashMap<PseudonymKey, MemberState>,

    /// Where each author's op log lives (`M`/2), from its latest merged
    /// `MemberJoined` or `MemberRecordMoved`, kept after the author leaves
    /// or is removed: its ops stay in the DAG, so its own last op is
    /// published there and frontiers still name it (plan E3.2, evidence 4.4).
    pub op_log_records: HashMap<PseudonymKey, String>,

    /// Highest `MemberRemoved` stamp per removed member; a re-join must
    /// come after it.
    pub removed: HashMap<PseudonymKey, OpStamp>,

    /// Prospective members who wrote a `MemberJoined` and have not yet
    /// been approved or rejected. Grow-Only Set, cleared per target by a
    /// decision entry.
    ///
    /// Populated only under `AdmissionMode::ApprovalRequired`; under
    /// `Open` a joiner claims a slot without asking, so this stays empty
    /// and the approve/reject IPC commands are inert.
    pub pending_members: HashMap<PseudonymKey, PendingMemberState>,

    /// Recorded admission decisions, keyed by target. LWW per target.
    ///
    /// Kept after the pending row is cleared so a replayed or
    /// out-of-order `JoinRequested` cannot resurrect a settled request —
    /// the merge must converge regardless of delivery order.
    pub admitted: HashMap<PseudonymKey, AdmissionDecision>,

    /// Whether the community admits freely or by approval.
    ///
    /// `None` until the creator sets it, and settable exactly once — see
    /// `validate_write`. That is the immutability guarantee: not "must be
    /// the literal first entry" (a creator writes several genesis
    /// entries, so only one could ever qualify), but "only the creator,
    /// and only while unset". Same effect as Jami fixing the mode in the
    /// initial commit, without depending on write ordering.
    ///
    /// Read through [`Self::effective_admission_mode`].
    pub admission_mode: Option<AdmissionMode>,

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

    /// Members' RSVPs per event (LWW per (event_id, author), plan C7.15).
    /// Kept per event id whether or not the event is live yet, so the
    /// merge does not depend on an event's create entry surviving
    /// compaction; an event's archive clears its RSVPs, and readers go
    /// through [`GovernanceState::live_event_rsvps`].
    pub event_rsvps: HashMap<EventId, HashMap<PseudonymKey, RsvpState>>,

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
    pub attachment_pin_stamps: HashMap<[u8; 16], OpStamp>,

    /// The `GenesisCert`'s `creator_pseudonym` (MSC4289): set from the
    /// pinned cert, never from an op.
    pub creator: Option<PseudonymKey>,

    /// AutoMod rules (LWW per rule_id).
    pub automod_rules: HashMap<[u8; 16], AutoModRuleState>,

    /// Onboarding config (LWW).
    pub onboarding: Option<OnboardingState>,

    /// Welcome screen (LWW).
    pub welcome_screen: Option<WelcomeScreenState>,

    /// Track the highest lamport seen per author for ban-entry ordering.
    /// Key: (target_pseudonym) → highest lamport of ban/unban for that target.
    pub ban_stamps: HashMap<PseudonymKey, OpStamp>,

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
    pub expression_remove_stamps: HashMap<[u8; 16], OpStamp>,

    /// Architecture §17.1 + §20.6 — community-wide policy (default
    /// notification level + raid-protection thresholds). LWW; `None`
    /// until the first `CommunityPolicy` governance entry is merged.
    pub community_policy: Option<CommunityPolicyState>,

    /// The moderator queue (plan E3.1, D26): every merged
    /// `GovernanceEntry::ModReport`, by report id. Readers show it to
    /// members holding `MANAGE_MESSAGES` only.
    pub mod_reports: BTreeMap<[u8; 16], ModReportState>,
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
    pub created: OpStamp,
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
    pub stamp: OpStamp,
}

/// The outcome of an admission decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionDecision {
    pub approved: bool,
    pub stamp: OpStamp,
}

/// A pending admission request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMemberState {
    pub member_record: String,
    pub member_record_owner: String,
    pub stamp: OpStamp,
}

/// A counted member: its `MemberJoined` applied (and, under
/// `ApprovalRequired`, approved), not since removed or left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberState {
    pub member_record: String,
    pub member_record_owner: String,
    pub joined: OpStamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetadataState {
    pub name: String,
    pub description: Option<String>,
    pub icon_hash: Option<String>,
    pub banner_hash: Option<String>,
    pub stamp: OpStamp,
}

/// Architecture §17.1 tier-1 default notification level for a community.
/// One of "all" | "mentions" | "nothing"; the resolver in
/// `services/community/notifications.rs` falls back to "all" when this
/// is unset. LWW per community.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationDefaultState {
    pub level: String,
    pub stamp: OpStamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CategoryState {
    pub name: String,
    pub position: u32,
    pub created: OpStamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverwriteState {
    pub target_type: String,
    pub allow: u64,
    pub deny: u64,
    pub stamp: OpStamp,
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
    pub created: OpStamp,
    pub archived: Option<OpStamp>,
    /// The newest `ThreadUnarchived` lamport: an archive at or below it
    /// is older than the reopening and does not apply.
    pub unarchived: Option<OpStamp>,
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
    pub stamp: OpStamp,
}

/// One merged moderator report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModReportState {
    pub reporter: PseudonymKey,
    pub target: rekindle_types::governance::ModReportTarget,
    pub reason: String,
    pub stamp: OpStamp,
}

/// One member's answer to one event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsvpState {
    pub status: rekindle_types::event::RsvpStatus,
    pub stamp: OpStamp,
}

impl GovernanceState {
    /// RSVPs of live events, each event's list sorted by pseudonym so a
    /// reader's view is deterministic.
    #[must_use]
    pub fn live_event_rsvps(
        &self,
    ) -> Vec<(
        EventId,
        Vec<(PseudonymKey, rekindle_types::event::RsvpStatus)>,
    )> {
        let mut out: Vec<_> = self
            .event_rsvps
            .iter()
            .filter(|(event_id, _)| self.events.contains_key(*event_id))
            .map(|(event_id, rsvps)| {
                let mut list: Vec<_> = rsvps
                    .iter()
                    .map(|(member, rsvp)| (member.clone(), rsvp.status))
                    .collect();
                list.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
                (*event_id, list)
            })
            .collect();
        out.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
        out
    }
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
    pub stamp: OpStamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimeoutState {
    pub duration_seconds: u64,
    pub started_at: u64,
    pub stamp: OpStamp,
}

/// Architecture §10.7 + §20.6 — merged-state form of `CommunityPolicy`.
#[derive(Debug, Clone, PartialEq)]
pub struct CommunityPolicyState {
    pub policy_text: Option<String>,
    pub max_joins_per_interval: u32,
    pub join_interval_seconds: u32,
    pub stamp: OpStamp,
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
    /// The admission mode in force. `Open` when the community never set
    /// one — which is every community created before this existed, so
    /// the default preserves current behaviour.
    #[must_use]
    pub fn effective_admission_mode(&self) -> AdmissionMode {
        self.admission_mode.unwrap_or_default()
    }

    /// Architecture §9.3 line 1946 — return the highest `RoleState.position`
    /// across all roles assigned to `member`. Higher position = higher rank
    /// (Discord convention). Members with no role assignments default to 0
    /// (the @everyone-equivalent floor).
    ///
    /// Used by `validate_write` to enforce role hierarchy on Ban / Timeout /
    /// RoleAssignment / RoleUnassignment entries: a writer can only act on a
    /// target whose max position is strictly less than the writer's.
    pub fn member_max_position(&self, member: &PseudonymKey) -> u32 {
        self.role_assignments.get(member).map_or(0, |role_ids| {
            role_ids
                .iter()
                .filter_map(|rid| self.roles.get(rid))
                .map(|role| role.position)
                .max()
                .unwrap_or(0)
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AutoModRuleState {
    pub name: String,
    pub enabled: bool,
    pub trigger_json: String,
    pub action: rekindle_types::governance::AutoModAction,
    pub stamp: OpStamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OnboardingState {
    pub enabled: bool,
    pub mode: String,
    pub default_channels: Vec<ChannelId>,
    pub questions: Vec<OnboardingQuestion>,
    pub welcome_message: Option<String>,
    pub guide_steps: Vec<GuideStep>,
    pub stamp: OpStamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WelcomeScreenState {
    pub description: String,
    pub channels: Vec<WelcomeChannel>,
    pub stamp: OpStamp,
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
    pub linked: OpStamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InviteState {
    pub code_hash: String,
    pub max_uses: u32,
    pub expires_at: Option<u64>,
    pub secrets_record_key: String,
    pub created: OpStamp,
    /// M10.3 — the inviter's pseudonym, populated from the writing
    /// governance subkey at merge time. Used for the per-inviter
    /// active-invite quota in `invite_quota.rs` and for audit-log
    /// attribution.
    pub creator_pseudonym: PseudonymKey,
}
