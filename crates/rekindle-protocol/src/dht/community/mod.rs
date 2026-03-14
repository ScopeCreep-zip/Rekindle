pub mod audit_log;
pub mod automod;
pub mod channel_record;
pub mod envelope;
pub mod manifest;
pub mod member_registry;
pub mod onboarding;
pub mod permissions_v2;
pub mod types;

// Re-export types for convenient access via `dht::community::*`
pub use audit_log::{AuditAction, AuditChange, AuditLogEntry, AuditTarget};
pub use automod::{AutoModAction, AutoModConfig, AutoModRule, AutoModTrigger, RaidAction, RaidProtection};
pub use envelope::{
    sign_envelope, verify_envelope, CommunityEnvelope, ControlPayload, OnboardingAnswer,
    PresenceGameInfo, SignedEnvelope,
};
pub use onboarding::{OnboardingConfig, OnboardingMode, OnboardingQuestion, WelcomeScreen};
pub use permissions_v2::{calculate_permissions_v2, has_permission_v2, Permissions};
pub use types::{
    BanEntry, CategoryEntry, ChannelEntryV2, ChannelKind, CommunityMetadataV2, CommunityPolicy,
    CoordinatorInfo, EncryptedMEKCopy, InviteEntry, InviteSecrets, MEKVaultEntry, MemberPresence,
    MemberSummary, ModerationLevel, RegistrySegmentInfo, RegistrySpine, RoleEntryV2,
    SignedPresence, MANIFEST_REGISTRY_SPINE,
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
