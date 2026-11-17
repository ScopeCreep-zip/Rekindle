//! The power row of every [`GovernanceEntry`] variant (plan E3.2).
//!
//! Matrix state resolution v2 resolves "power events" (power levels, join
//! rules, kicks, bans) first, then orders everything else along the
//! mainline of resolved power events. Here the power ops are the ones that
//! change who may do what: roles and their assignment, overwrites,
//! admission, bans, timeouts and removals. The match is exhaustive, so a
//! new variant cannot compile without a row.

use std::collections::HashSet;

use rekindle_types::governance::{GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;

use crate::op::VerifiedOp;

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
        | G::MemberRemoved { .. } => OpClass::Power,
        G::ChannelCreated { .. }
        | G::ChannelArchived { .. }
        | G::ChannelUpdated { .. }
        | G::MemberJoined { .. }
        | G::MemberRecordMoved { .. }
        | G::MemberLeft { .. }
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
        | G::CommunityPolicy { .. }
        // It changes no state (plan E3.2 S7c.3).
        | G::Checkpoint(_) => OpClass::NonPower,
    }
}

/// The member an op removes or demotes, for strong removal: a durable
/// kick, a ban, or taking a role away from someone else.
#[must_use]
pub fn removal_target(entry: &GovernanceEntry) -> Option<&PseudonymKey> {
    match entry {
        GovernanceEntry::MemberRemoved { target, .. }
        | GovernanceEntry::BanEntry { target, .. }
        | GovernanceEntry::RoleUnassignment { target, .. } => Some(target),
        _ => None,
    }
}

/// The seq frontier a `MemberRemoved` or `BanEntry` carries: the number of
/// its target's ops in the removal's causal past (plan E3.2 S7c.3a). Every
/// op of the target at a seq at or above it is masked; `None` for any
/// other entry (a `RoleUnassignment` demotes without one).
#[must_use]
pub fn target_seq_frontier(entry: &GovernanceEntry) -> Option<u64> {
    match entry {
        GovernanceEntry::MemberRemoved {
            target_seq_frontier,
            ..
        }
        | GovernanceEntry::BanEntry {
            target_seq_frontier,
            ..
        } => Some(*target_seq_frontier),
        _ => None,
    }
}

/// `entry` with its seq frontier set to `frontier`, as the writer fills it
/// from its own op set when it builds the op; any other entry unchanged.
#[must_use]
pub fn with_target_seq_frontier(mut entry: GovernanceEntry, frontier: u64) -> GovernanceEntry {
    if let GovernanceEntry::MemberRemoved {
        target_seq_frontier,
        ..
    }
    | GovernanceEntry::BanEntry {
        target_seq_frontier,
        ..
    } = &mut entry
    {
        *target_seq_frontier = frontier;
    }
    entry
}

/// The number of `target`'s ops in the causal past of `deps` (the deps
/// themselves included): its highest seq there plus one, 0 when none.
/// `known` resolves a dep by hash; a dep it does not resolve adds nothing.
/// A pure function of the deps, so every reader computes what the writer
/// put on the wire.
#[must_use]
pub fn seq_frontier<'a>(
    deps: impl IntoIterator<Item = OpHash>,
    known: impl Fn(&OpHash) -> Option<&'a VerifiedOp>,
    target: &PseudonymKey,
) -> u64 {
    let mut seen: HashSet<OpHash> = HashSet::new();
    let mut stack: Vec<OpHash> = deps.into_iter().collect();
    let mut frontier = 0;
    while let Some(hash) = stack.pop() {
        if !seen.insert(hash) {
            continue;
        }
        let Some(op) = known(&hash) else {
            continue;
        };
        if op.author() == target {
            frontier = frontier.max(op.op().seq + 1);
        }
        stack.extend(op.op().deps.iter().map(|d| d.op_hash));
    }
    frontier
}
