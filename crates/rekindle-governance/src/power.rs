//! The power row of every [`GovernanceEntry`] variant (plan E3.2).
//!
//! Matrix state resolution v2 resolves "power events" (power levels, join
//! rules, kicks, bans) first, then orders everything else along the
//! mainline of resolved power events. Here the power ops are the ones that
//! change who may do what: roles and their assignment, overwrites,
//! admission, bans, timeouts and removals. The match is exhaustive, so a
//! new variant cannot compile without a row.

use rekindle_types::governance::GovernanceEntry;

/// Which phase of resolution an op belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpClass {
    /// Resolved first, in causal order, against the power state.
    Power,
    /// Ordered by mainline anchor, then `issued_at`, then op hash.
    NonPower,
}

/// The class of `entry`.
#[must_use]
pub fn class_of(entry: &GovernanceEntry) -> OpClass {
    use GovernanceEntry as G;
    match entry {
        G::RoleDefinition { .. }
        | G::RoleAssignment { .. }
        | G::RoleUnassignment { .. }
        | G::RoleArchived { .. }
        | G::PermissionOverwrite { .. }
        | G::BanEntry { .. }
        | G::UnbanEntry { .. }
        | G::TimeoutEntry { .. }
        | G::RemoveTimeoutEntry { .. }
        | G::MemberApproved { .. }
        | G::MemberRejected { .. }
        | G::MemberRemoved { .. }
        | G::AdmissionPolicy { .. } => OpClass::Power,
        G::ChannelCreated { .. }
        | G::ChannelArchived { .. }
        | G::ChannelUpdated { .. }
        | G::MemberJoined { .. }
        | G::MemberRecordMoved { .. }
        | G::MemberLeft {}
        | G::CommunityMeta { .. }
        | G::CommunityNotificationDefault { .. }
        | G::MEKGenerationBump { .. }
        | G::CategoryCreated { .. }
        | G::CategoryArchived { .. }
        | G::CategoryUpdated { .. }
        | G::ThreadCreated { .. }
        | G::ThreadArchived { .. }
        | G::ThreadUnarchived { .. }
        | G::EventCreated { .. }
        | G::EventArchived { .. }
        | G::EventRsvp { .. }
        | G::ExpressionAdded { .. }
        | G::ExpressionRemoved { .. }
        | G::ModReport { .. }
        | G::OnboardingConfig { .. }
        | G::WelcomeScreen { .. }
        | G::AdminDelete { .. }
        | G::ChannelSegmentLinked { .. }
        | G::SegmentAdded { .. }
        | G::AutoModRule { .. }
        | G::InviteCreated { .. }
        | G::InviteRevoked { .. }
        | G::AttachmentPinned { .. }
        | G::CommunityPolicy { .. } => OpClass::NonPower,
    }
}

/// The member an op removes or demotes, for strong removal: a durable
/// kick, a ban, or taking a role away from someone else.
#[must_use]
pub fn removal_target(entry: &GovernanceEntry) -> Option<&rekindle_types::id::PseudonymKey> {
    match entry {
        GovernanceEntry::MemberRemoved { target, .. }
        | GovernanceEntry::BanEntry { target, .. }
        | GovernanceEntry::RoleUnassignment { target, .. } => Some(target),
        _ => None,
    }
}
