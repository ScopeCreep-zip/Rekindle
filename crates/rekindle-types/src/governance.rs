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
        /// Lower position = more powerful (same as Discord hierarchy)
        position: u32,
        color: u32,
        hoist: bool,
        mentionable: bool,
        self_assignable: bool,
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
        /// DHT key — created lazily on first message, None initially.
        record_key: Option<String>,
        lamport: u64,
    },

    /// Archive a thread. CRDT: tombstone (archived threads excluded from merged state).
    ThreadArchived { thread_id: ThreadId, lamport: u64 },

    /// Create a scheduled event. RSVPs stored in MemberPresence, not here.
    EventCreated {
        event_id: EventId,
        name: String,
        description: Option<String>,
        start_time: u64,
        end_time: Option<u64>,
        channel_id: Option<ChannelId>,
        lamport: u64,
    },

    /// Add a custom expression asset. CRDT: OR-Set on expression_id.
    ExpressionAdded {
        expression_id: [u8; 16],
        name: String,
        /// "emoji" | "sticker" | "soundboard"
        kind: String,
        content_hash: String,
        inline_data: Option<Vec<u8>>,
        animated: bool,
        tags: Vec<String>,
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

    /// Segment expansion — adds a new registry + governance segment for >255 members.
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
}

/// Wire format for a governance SMPL subkey value.
///
/// Wraps governance entries with the author's community pseudonym so the
/// CRDT merge engine knows who wrote each subkey without relying on the
/// SMPL slot keypair (which is different from the pseudonym).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GovernanceSubkeyPayload {
    pub author_pseudonym: PseudonymKey,
    pub entries: Vec<GovernanceEntry>,
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
            | Self::InviteRevoked { lamport, .. } => *lamport,
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
