//! v2.0 Governance entry types for flat SMPL CRDT.
//!
//! Each member writes GovernanceEntry variants to their own subkey in the
//! governance SMPL record (o_cnt: 0). All entries carry a Lamport timestamp
//! for deterministic CRDT merge ordering.
//!
//! See architecture doc §4.3 Record 1 and §4.4 for merge rules.
//! See rekindle-architecture-v2.md §4 for field specifications.

use serde::{Deserialize, Serialize};

use crate::id::{CategoryId, ChannelId, EventId, PseudonymKey, RoleId, ThreadId};

/// A single governance entry written by a member to their SMPL subkey.
///
/// The CRDT merge engine (`rekindle-governance`) processes all entries from
/// all member subkeys, sorts by `(lamport, author_pseudonym)`, and applies
/// deterministic merge rules to produce a `GovernanceState`.
///
/// Permission enforcement is reader-side: each reader validates whether the
/// writer had permission for the entry type based on the accumulated CRDT state.
/// Invalid entries are silently excluded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GovernanceEntry {
    /// Create a new channel. Creates an associated SMPL channel record.
    ChannelCreated {
        channel_id: ChannelId,
        name: String,
        /// "text", "voice", "announcement", "forum", "stage", "media"
        channel_type: String,
        /// DHT key of the channel's SMPL record (same universal schema)
        record_key: String,
        category_id: Option<CategoryId>,
        position: u32,
        /// Architecture §10.8 — text-in-voice. When set, this channel
        /// is the text companion of the named voice channel; the
        /// frontend hides it from the channel list unless the local
        /// member is currently connected to that voice channel. The
        /// channel record itself is a standard SMPL record (i.e. its
        /// privacy is client-side filtering, not cryptographic
        /// gating). `None` for normal channels.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_voice_channel_id: Option<ChannelId>,
        lamport: u64,
    },

    /// Archive (soft-delete) a channel. CRDT: removes from active set.
    ChannelArchived { channel_id: ChannelId, lamport: u64 },

    /// Update channel metadata. CRDT: LWW per field per channel_id.
    ///
    /// `category_id`: `None` = no change, `Some(None)` = remove from category,
    /// `Some(Some(id))` = move to category.
    ChannelUpdated {
        channel_id: ChannelId,
        name: Option<String>,
        topic: Option<String>,
        forum_tags: Option<Vec<String>>,
        position: Option<u32>,
        slowmode_seconds: Option<u32>,
        nsfw: Option<bool>,
        category_id: Option<Option<CategoryId>>,
        lamport: u64,
    },

    /// Define or redefine a role. CRDT: LWW per role_id (highest lamport wins).
    RoleDefinition {
        role_id: RoleId,
        name: String,
        /// 64-bit permission bitmask (see `permissions` module)
        permissions: u64,
        /// Architecture §9.3 line 1946 — higher position = higher rank
        /// (Discord convention). Used for moderation hierarchy: a member
        /// can only ban/timeout/manage another member whose max role
        /// position is strictly less than the actor's own max position.
        position: u32,
        color: u32,
        hoist: bool,
        mentionable: bool,
        self_assignable: bool,
        /// Architecture §19.4 — when set, only one role per group may
        /// be active per member. Assigning a role in this group
        /// auto-unassigns peers in the same group with a lower
        /// Lamport. Ignored when `None`. Conventionally a short slug
        /// like `"pronouns"` or `"region"`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exclusion_group: Option<String>,
        lamport: u64,
    },

    /// Assign a role to a member. CRDT: LWW-Flag per (target, role_id).
    RoleAssignment {
        target: PseudonymKey,
        role_id: RoleId,
        lamport: u64,
    },

    /// Remove a role from a member. CRDT: LWW-Flag per (target, role_id).
    RoleUnassignment {
        target: PseudonymKey,
        role_id: RoleId,
        lamport: u64,
    },

    /// Ban a member. CRDT: LWW-Flag per target (UnbanEntry reverses).
    BanEntry {
        target: PseudonymKey,
        reason: Option<String>,
        lamport: u64,
    },

    /// Unban a member. Must have higher lamport than the corresponding BanEntry.
    UnbanEntry { target: PseudonymKey, lamport: u64 },

    /// Timeout a member temporarily. Strips all permissions except view.
    TimeoutEntry {
        target: PseudonymKey,
        duration_seconds: u64,
        reason: Option<String>,
        started_at: u64,
        lamport: u64,
    },

    /// Remove an active timeout from a member.
    RemoveTimeoutEntry { target: PseudonymKey, lamport: u64 },

    /// Update community metadata. CRDT: LWW (highest lamport replaces all).
    CommunityMeta {
        name: Option<String>,
        description: Option<String>,
        icon_hash: Option<String>,
        banner_hash: Option<String>,
        lamport: u64,
    },

    /// Community-wide default notification level (architecture §17.1
    /// three-tier cascade tier 1). The most-specific setting wins:
    /// per-channel local override > community default > implicit "all".
    /// `level` is one of "all" | "mentions" | "nothing". CRDT: LWW.
    CommunityNotificationDefault {
        level: String,
        lamport: u64,
    },

    /// Bump MEK generation. CRDT: Max-Register (highest generation wins).
    /// Written by the deterministic rotator after peer-to-peer MEK distribution.
    ///
    /// Readers validate that the writer is the correct deterministic rotator
    /// (or cascade successor) for the departure event specified by `trigger_departed`.
    MEKGenerationBump {
        generation: u64,
        /// The pseudonym of the departed member whose leaving triggered this rotation.
        /// Used by readers to independently verify the writer is the correct rotator.
        trigger_departed: PseudonymKey,
        /// For cascading fallback: rotator candidates that timed out before this writer
        /// took over. Readers verify each was offline (no heartbeat in 30s window).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        cascade_skipped: Vec<PseudonymKey>,
        lamport: u64,
    },

    /// Create a category (display-only channel grouping).
    CategoryCreated {
        category_id: CategoryId,
        name: String,
        position: u32,
        lamport: u64,
    },

    /// Archive a category.
    CategoryArchived {
        category_id: CategoryId,
        lamport: u64,
    },

    /// Set per-channel permission overwrite. CRDT: LWW per (channel_id, target_id).
    PermissionOverwrite {
        channel_id: ChannelId,
        /// "role" or "member"
        target_type: String,
        /// Role ID hex or member pseudonym hex
        target_id: String,
        allow: u64,
        deny: u64,
        lamport: u64,
    },

    /// Create a thread (lazy SMPL record creation).
    ThreadCreated {
        thread_id: ThreadId,
        parent_channel_id: ChannelId,
        name: String,
        /// "public" | "private" | "announcement" | "forum_post"
        thread_type: String,
        /// DHT key — created lazily on first message, None initially.
        record_key: Option<String>,
        /// Explicit invitees for private threads.
        invited: Vec<PseudonymKey>,
        /// Tag selected from the parent forum channel, when applicable.
        forum_tag: Option<String>,
        /// Auto-archive after this many seconds of inactivity.
        auto_archive_seconds: u64,
        lamport: u64,
    },

    /// Archive a thread. CRDT: tombstone (archived threads excluded from merged state).
    ThreadArchived { thread_id: ThreadId, lamport: u64 },

    /// Create or update a scheduled event (architecture §21).
    /// RSVPs stored in MemberPresence.event_rsvps, not here. CRDT:
    /// LWW per `event_id` — bumping `lamport` re-publishes any field
    /// (e.g. status: Scheduled → Active → Completed).
    EventCreated {
        event_id: EventId,
        name: String,
        description: Option<String>,
        start_time: u64,
        end_time: Option<u64>,
        channel_id: Option<ChannelId>,
        /// Spec §21 line 2624 — peer-cached cover image
        /// (`ContentRef.blake3_hash` style, encoded as a hex string).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cover_image_ref: Option<String>,
        /// Spec §21 line 2625 — author for display + permission audit.
        /// Optional for backward compatibility with rows written before
        /// this field existed; readers fall back to the SMPL author
        /// pseudonym if `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        creator_pseudonym: Option<PseudonymKey>,
        /// Spec §21 line 2628 — recurrence rule.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recurrence: Option<crate::event::RecurrenceRule>,
        /// Spec §21 line 2629 — voice-channel / stage / external /
        /// in-game. Defaults to whatever `channel_id` referenced (or
        /// `External("")` if none) for legacy rows.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<crate::event::EventLocation>,
        /// Spec §21 line 2630 — Scheduled / Active / Completed /
        /// Cancelled. `None` decoded from legacy rows = Scheduled.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<crate::event::EventStatus>,
        lamport: u64,
    },

    /// Add a custom expression asset. CRDT: OR-Set on expression_id.
    ExpressionAdded {
        expression_id: [u8; 16],
        name: String,
        /// "emoji" | "sticker" | "soundboard"
        kind: String,
        content_hash: String,
        /// Architecture §18.4 — Lost Cargo manifest. Receivers fetch the
        /// asset bytes via the existing `RequestAttachment` /
        /// `AttachmentChunk` flow. `None` only for Removed-tombstone
        /// echoes and the rare in-tree fixture; new uploads always set
        /// this. Replaces the legacy `inline_data` path which could not
        /// fit in Veilid's 32 KiB subkey limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attachment: Option<crate::attachment::AttachmentOffer>,
        animated: bool,
        tags: Vec<String>,
        /// Architecture §18.3 — soundboard-specific metadata (duration,
        /// volume, optional emoji). `None` for emoji/sticker entries
        /// and for legacy soundboard entries written before the field
        /// existed; legacy entries default to `volume: 1.0` at read.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sound_meta: Option<crate::expression::SoundboardMeta>,
        /// Architecture §18.1 line 2455 — author of the asset. Used by
        /// the audit log and the "uploaded by" display in the picker.
        /// `None` for legacy entries written before this field existed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        creator_pseudonym: Option<PseudonymKey>,
        /// Architecture §18.1 line 2456 — wall-clock seconds at upload.
        /// `None` for legacy entries; readers fall back to the entry's
        /// own Lamport for ordering displays.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_at: Option<u64>,
        /// Architecture §18.1 line 2459 — when `Some(false)` peers
        /// outside this community must not see the asset (gates the
        /// `USE_EXTERNAL_EMOJIS` cross-community path). `None` and
        /// `Some(true)` mean shareable; readers treat `None` as `true`
        /// because the previous behaviour was effectively shareable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        available_to_peers: Option<bool>,
        lamport: u64,
    },

    /// Remove a custom expression. CRDT: OR-Set tombstone by expression_id.
    ExpressionRemoved {
        expression_id: [u8; 16],
        lamport: u64,
    },

    /// Archive a scheduled event. CRDT: tombstone.
    EventArchived { event_id: EventId, lamport: u64 },

    /// Onboarding configuration. CRDT: LWW (latest lamport wins).
    OnboardingConfig {
        enabled: bool,
        /// "default", "guided", "gated"
        mode: String,
        default_channels: Vec<ChannelId>,
        questions: Vec<OnboardingQuestion>,
        welcome_message: Option<String>,
        guide_steps: Vec<GuideStep>,
        lamport: u64,
    },

    /// Welcome screen shown after onboarding. CRDT: LWW (latest lamport wins).
    WelcomeScreen {
        description: String,
        channels: Vec<WelcomeChannel>,
        lamport: u64,
    },

    /// Admin delete a message (tombstone). Requires MANAGE_MESSAGES.
    AdminDelete {
        message_id: [u8; 16],
        channel_id: ChannelId,
        reason: Option<String>,
        lamport: u64,
    },

    /// Plate Gate (architecture §15.4 lazy channel records) — announces
    /// that a channel has acquired a segment-N SMPL record for messages
    /// from members in segment N. Written by the first segment-N member
    /// to send a message in `channel_id` (architecture §15.4 + DS-aligned
    /// v2 plan line 993). Other members merge this entry, then
    /// `open_record` + watch the new key so messages from segment-N peers
    /// are discoverable. CRDT: LWW per `(channel_id, segment_index)`.
    ChannelSegmentLinked {
        channel_id: ChannelId,
        segment_index: u32,
        /// SMPL record key for messages from segment-N members in this
        /// channel. The schema mirrors the segment-0 record (255 slots).
        record_key: String,
        lamport: u64,
    },

    /// Plate Gate (architecture §15) — adds a new registry + governance
    /// segment when the highest existing segment has its 255 slots filled.
    /// Admin-only (`MANAGE_COMMUNITY`) per §15.2; merged as ORMap-of-CRDTs
    /// per Shapiro 2011 / Almeida et al. 2016 (arXiv:1603.01529).
    ///
    /// Wire-format note: §4.6 of the architecture spec writes this with
    /// `segment_index: u16, slot_range: (u16, u16), keys: TypedKey`. The
    /// in-tree shape uses `u32` and `String` for forward compatibility
    /// (Veilid `TypedKey` round-trips through `String` everywhere else in
    /// the codebase; widening `u16→u32` costs nothing and avoids cascade
    /// edits across merge.rs / validate.rs / audit.rs / proptests).
    /// Functionally identical.
    ///
    /// Slot indices are GLOBAL across segments (architecture §15.2:2271
    /// "slots 255–509"). A member's `subkey_index` in segment N's record
    /// is the LOCAL index `0..255`; their global slot is
    /// `slot_range_start + local_subkey`. Slot keypair derivation
    /// (§8.3:1659-1676) takes the global index, so per-segment uniqueness
    /// is automatic — no `segment_index` parameter is needed in the HKDF
    /// input.
    SegmentAdded {
        segment_index: u32,
        registry_key: String,
        governance_key: String,
        slot_range_start: u32,
        slot_range_end: u32,
        lamport: u64,
    },

    /// AutoMod rule. CRDT: LWW per rule_id.
    AutoModRule {
        rule_id: [u8; 16],
        name: String,
        enabled: bool,
        /// JSON-encoded trigger config (keyword list, regex patterns)
        trigger_json: String,
        /// "block_locally", "blur_content", "alert_moderators"
        action: String,
        lamport: u64,
    },

    /// Remove a role definition. CRDT: tombstone (archived roles excluded from merged state).
    RoleArchived { role_id: RoleId, lamport: u64 },

    /// Update category metadata. CRDT: LWW per category_id.
    CategoryUpdated {
        category_id: CategoryId,
        name: Option<String>,
        position: Option<u32>,
        lamport: u64,
    },

    /// Create an invite. Stores the encrypted invite secrets in governance.
    /// In v2.0, invites are governance entries rather than manifest subkeys.
    InviteCreated {
        invite_id: [u8; 16],
        /// SHA-256 hash of the invite code (hex). The raw code is never stored.
        code_hash: String,
        max_uses: u32,
        expires_at: Option<u64>,
        /// Base64-encoded encrypted InviteSecrets blob.
        encrypted_secrets: String,
        lamport: u64,
    },

    /// Revoke an invite. CRDT: tombstone by invite_id.
    InviteRevoked { invite_id: [u8; 16], lamport: u64 },

    /// Lost Cargo: admin pin/unpin of an attachment. Pinned files are
    /// exempt from local LRU cache eviction (architecture §28.9 line 3283).
    /// Requires `MANAGE_COMMUNITY`. CRDT: LWW per `attachment_id` —
    /// later lamport wins; merged state is the boolean `pinned` flag.
    AttachmentPinned {
        attachment_id: [u8; 16],
        /// True = pinned, false = unpinned.
        pinned: bool,
        lamport: u64,
    },

    /// Architecture §10.7 + §20.6 — community-wide policy text plus
    /// raid-protection thresholds. Notification defaults live in the
    /// separate `CommunityNotificationDefault` entry (architecture §17.1).
    /// Peer-side rate detection alerts moderators when join volume
    /// exceeds `max_joins_per_interval` within `join_interval_seconds`.
    /// Requires `MANAGE_COMMUNITY`. CRDT: LWW community-wide.
    CommunityPolicy {
        /// Optional policy / rules markdown shown to new members
        /// (architecture §10.7 line 724).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        policy_text: Option<String>,
        /// Architecture §20.6 default: 20 joins per 10 minutes.
        max_joins_per_interval: u32,
        /// Window length for the rate detector. Default 600 s.
        join_interval_seconds: u32,
        lamport: u64,
    },
}

/// Wire format for a governance SMPL subkey value.
///
/// Wraps governance entries with the author's community pseudonym so the
/// CRDT merge engine knows who wrote each subkey without relying on the
/// SMPL slot keypair (which is community-shared by design — see
/// `rekindle_secrets::derive::derive_slot_keypair`).
///
/// Architecture §26 W26 line 4140 — `signature` is an Ed25519 signature
/// by the author's pseudonym secret over [`signing_bytes`]. Any reader
/// MUST verify the signature against `author_pseudonym` (which is itself
/// the Ed25519 public key) before applying the entries to local state;
/// otherwise any community member could impersonate any other by writing
/// a forged payload to any subkey (the slot keypair authentication on
/// `set_dht_value` proves only "some member wrote this," not which one).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GovernanceSubkeyPayload {
    pub author_pseudonym: PseudonymKey,
    pub entries: Vec<GovernanceEntry>,
    /// 64-byte Ed25519 signature over [`signing_bytes`]. Empty `Vec` for
    /// pre-signature payloads in disk fixtures or in-flight legacy
    /// rows; readers treat empty signatures as authentication failure
    /// once SCHEMA_VERSION 59 ships.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signature: Vec<u8>,
}

impl GovernanceSubkeyPayload {
    /// Canonical bytes signed by the author. Receivers reproduce these
    /// bytes from the deserialized payload and verify against
    /// `author_pseudonym`. Including the entry count and a domain tag
    /// stops cross-protocol forgeries (a signature for a presence write
    /// can't be replayed as a governance write).
    pub fn signing_bytes(&self) -> Vec<u8> {
        let entries_json = serde_json::to_vec(&self.entries).unwrap_or_default();
        let mut out =
            Vec::with_capacity(b"rekindle-gov-subkey-v1".len() + 32 + 8 + entries_json.len());
        out.extend_from_slice(b"rekindle-gov-subkey-v1");
        out.extend_from_slice(&self.author_pseudonym.0);
        out.extend_from_slice(&(self.entries.len() as u64).to_le_bytes());
        out.extend_from_slice(&entries_json);
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OnboardingQuestion {
    pub question_id: String,
    pub title: String,
    pub description: Option<String>,
    pub required: bool,
    pub single_select: bool,
    pub options: Vec<OnboardingOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OnboardingOption {
    pub option_id: String,
    pub title: String,
    pub description: Option<String>,
    pub emoji: Option<String>,
    pub roles_to_assign: Vec<RoleId>,
    pub channels_to_show: Vec<ChannelId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GuideStep {
    pub title: String,
    pub description: String,
    pub channel_id: Option<ChannelId>,
    pub emoji: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WelcomeChannel {
    pub channel_id: ChannelId,
    pub description: String,
    pub emoji: Option<String>,
}

impl GovernanceEntry {
    /// Extract the Lamport timestamp for CRDT ordering.
    pub fn lamport(&self) -> u64 {
        match self {
            Self::ChannelCreated { lamport, .. }
            | Self::ChannelArchived { lamport, .. }
            | Self::ChannelUpdated { lamport, .. }
            | Self::RoleDefinition { lamport, .. }
            | Self::RoleAssignment { lamport, .. }
            | Self::RoleUnassignment { lamport, .. }
            | Self::BanEntry { lamport, .. }
            | Self::UnbanEntry { lamport, .. }
            | Self::TimeoutEntry { lamport, .. }
            | Self::RemoveTimeoutEntry { lamport, .. }
            | Self::CommunityMeta { lamport, .. }
            | Self::CommunityNotificationDefault { lamport, .. }
            | Self::MEKGenerationBump { lamport, .. }
            | Self::CategoryCreated { lamport, .. }
            | Self::CategoryArchived { lamport, .. }
            | Self::PermissionOverwrite { lamport, .. }
            | Self::ThreadCreated { lamport, .. }
            | Self::ThreadArchived { lamport, .. }
            | Self::EventCreated { lamport, .. }
            | Self::ExpressionAdded { lamport, .. }
            | Self::ExpressionRemoved { lamport, .. }
            | Self::EventArchived { lamport, .. }
            | Self::OnboardingConfig { lamport, .. }
            | Self::WelcomeScreen { lamport, .. }
            | Self::AdminDelete { lamport, .. }
            | Self::SegmentAdded { lamport, .. }
            | Self::AutoModRule { lamport, .. }
            | Self::RoleArchived { lamport, .. }
            | Self::CategoryUpdated { lamport, .. }
            | Self::InviteCreated { lamport, .. }
            | Self::InviteRevoked { lamport, .. }
            | Self::AttachmentPinned { lamport, .. }
            | Self::ChannelSegmentLinked { lamport, .. }
            | Self::CommunityPolicy { lamport, .. } => *lamport,
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn governance_entry_serde_roundtrip() {
        let entry = GovernanceEntry::ChannelCreated {
            channel_id: ChannelId([1u8; 16]),
            name: "general".into(),
            channel_type: "text".into(),
            record_key: "VLD0:abc123".into(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
            lamport: 1,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: GovernanceEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, back);
    }

    #[test]
    fn lamport_extraction() {
        let entry = GovernanceEntry::BanEntry {
            target: PseudonymKey([0xAA; 32]),
            reason: Some("spam".into()),
            lamport: 42,
        };
        assert_eq!(entry.lamport(), 42);
    }

    #[test]
    fn all_variants_have_lamport() {
        // This test ensures the lamport() match is exhaustive.
        // If a new variant is added without a lamport field,
        // this will fail to compile.
        let entries = vec![
            GovernanceEntry::ChannelCreated {
                channel_id: ChannelId([0; 16]),
                name: String::new(),
                channel_type: String::new(),
                record_key: String::new(),
                category_id: None,
                position: 0,
                parent_voice_channel_id: None,
                lamport: 1,
            },
            GovernanceEntry::ChannelArchived {
                channel_id: ChannelId([0; 16]),
                lamport: 2,
            },
            GovernanceEntry::RoleDefinition {
                role_id: RoleId([0; 16]),
                name: String::new(),
                permissions: 0,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
                lamport: 3,
            },
            GovernanceEntry::MEKGenerationBump {
                generation: 1,
                trigger_departed: PseudonymKey([0; 32]),
                cascade_skipped: vec![],
                lamport: 4,
            },
        ];
        for e in &entries {
            assert!(e.lamport() > 0);
        }
    }
}
