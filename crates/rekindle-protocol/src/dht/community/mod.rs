pub mod audit_log;
pub mod automod;
pub(crate) mod base64_bytes;
pub mod channel_record;
pub mod envelope;
pub mod manifest;
pub mod member_registry;
pub mod onboarding;
pub mod types;

// Re-export types for convenient access via `dht::community::*`
pub use audit_log::{AuditAction, AuditChange, AuditLogEntry, AuditTarget};
pub use automod::{
    AutoModAction, AutoModConfig, AutoModRule, AutoModTrigger, RaidAction, RaidProtection,
};
pub use envelope::{
    sign_envelope, verify_envelope, CommunityEnvelope, ControlPayload, OnboardingAnswer,
    PresenceGameInfo, SignedEnvelope,
};
pub use onboarding::{OnboardingConfig, OnboardingMode, OnboardingQuestion, WelcomeScreen};
// `CoordinatorInfo`, `MemberPresence` and `SignedPresence` used to be
// re-exported here. All three were v1.0 coordinator-era types with no
// reachable caller — and `MemberPresence` was a second struct of that
// name, carrying `is_coordinator` / `coordinator_since`. The live
// presence type is `rekindle_types::presence::MemberPresence`.
pub use types::{
    BanEntry, CategoryEntry, ChannelEntryV2, ChannelKind, CommunityMetadataV2, CommunityPolicy,
    InviteEntry, InviteSecrets, MemberSummary, ModerationLevel, RoleEntryV2,
};

use serde::{Deserialize, Serialize};

/// The @everyone role always has ID 0.
pub const ROLE_EVERYONE_ID: u32 = 0;

/// Permission overwrite for a channel, targeting either a role or a specific member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionOverwrite {
    pub target_type: OverwriteType,
    /// Role ID (as string) or member pseudonym key.
    pub target_id: String,
    pub allow: u64,
    pub deny: u64,
}

/// Whether a permission overwrite targets a role or a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverwriteType {
    Role,
    Member,
}
