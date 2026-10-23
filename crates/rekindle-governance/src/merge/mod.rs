//! CRDT merge engine — the core of flat governance (plan E3.2).
//!
//! Takes the community's verified op set and its pinned `GenesisCert` and
//! produces a `GovernanceState`. **Convergence:** the same op set gives the
//! same state on every reader, whatever order the ops arrived in.
//!
//! **Resolution** follows Matrix state resolution v2: power ops
//! ([`crate::power`]) are resolved first, in causal (stamp) order, each
//! checked against the power state so far; every other op is then ordered
//! by its mainline anchor (the latest resolved power op in its causal
//! past), its `issued_at` and its hash, and checked in that order.
//!
//! **Strong removal** follows p2panda-auth: a removed or demoted member's
//! ops concurrent with the removal are invalidated, mutual removals both
//! apply, a re-join after a removal is allowed (its concurrent ops are
//! still invalidated), and invalidation is transitive — resolution reruns
//! until the invalidated set is stable.
//!
//! **The creator** is the cert's `creator_pseudonym` (MSC4289): no op at
//! any lamport becomes the creator.

use rekindle_types::governance::{GenesisCert, GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;

use crate::op::{OpStamp, VerifiedOp};
use crate::power::{class_of, OpClass};
use crate::state::{
    AdmissionDecision, AutoModRuleState, CategoryState, ChannelState, CommunityPolicyState,
    EventState, ExpressionState, GovernanceState, InviteState, MemberState, MetadataState,
    OnboardingState, OverwriteState, RoleState, SegmentState, ThreadState, TimeoutState,
    WelcomeScreenState,
};
use crate::validate::validate_write;

/// What `merge` decided about each op, besides the state.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeOutcome {
    pub state: GovernanceState,
    /// Applied ops, in the order they were applied.
    pub applied: Vec<OpHash>,
    /// Complete ops refused by validation or invalidated by resolution.
    pub refused: Vec<OpHash>,
    /// Ops whose deps are missing from the set.
    pub pending: Vec<OpHash>,
    /// Authors caught signing two ops at one seq.
    pub equivocators: Vec<PseudonymKey>,
}

/// Merge `ops` under the pinned `cert` into a canonical state.
#[must_use]
pub fn merge(cert: &GenesisCert, ops: &[VerifiedOp]) -> GovernanceState {
    merge_outcome(cert, ops).state
}

/// [`merge`], with the fate of every op.
#[must_use]
pub fn merge_outcome(cert: &GenesisCert, ops: &[VerifiedOp]) -> MergeOutcome {
    let set = resolve::prepare(ops);
    let mut invalid = vec![false; set.ops.len()];
    let mut round = resolve_round(cert, &set, &invalid);
    for _ in 0..=set.ops.len() {
        let next = gates::invalidated(cert, &set, &round);
        if next == invalid {
            break;
        }
        invalid = next;
        round = resolve_round(cert, &set, &invalid);
    }
    let applied = round.order.iter().map(|&i| set.ops[i].hash()).collect();
    let refused = (0..set.ops.len())
        .filter(|&i| !round.applied[i])
        .map(|i| set.ops[i].hash())
        .collect();
    MergeOutcome {
        state: round.state,
        applied,
        refused,
        pending: set.pending,
        equivocators: set.equivocators,
    }
}

/// One resolution pass with the ops in `invalid` left out.
pub(super) struct Round {
    pub state: GovernanceState,
    pub applied: Vec<bool>,
    /// Applied ops in application order (power ops first).
    pub order: Vec<usize>,
}

fn resolve_round(cert: &GenesisCert, set: &resolve::OpSet<'_>, invalid: &[bool]) -> Round {
    let mut state = GovernanceState {
        creator: Some(cert.creator_pseudonym.clone()),
        admission_mode: Some(cert.policy),
        ..GovernanceState::default()
    };
    let mut applied = vec![false; set.ops.len()];
    let mut order = Vec::new();
    let candidates = (0..set.ops.len()).filter(|&i| !invalid[i]);
    let (power, rest): (Vec<usize>, Vec<usize>) =
        candidates.partition(|&i| class_of(set.ops[i].body()) == OpClass::Power);
    for i in power {
        if try_apply(set.ops[i], &mut state) {
            applied[i] = true;
            order.push(i);
        }
    }
    let power_order = order.clone();
    let mut mainline: Vec<(Option<usize>, u64, OpHash, usize)> = rest
        .into_iter()
        .map(|i| {
            let anchor = power_order.iter().rposition(|&p| set.past.contains(i, p));
            (anchor, set.ops[i].op().issued_at, set.ops[i].hash(), i)
        })
        .collect();
    mainline.sort();
    for (_, _, _, i) in mainline {
        if try_apply(set.ops[i], &mut state) {
            applied[i] = true;
            order.push(i);
        }
    }
    Round {
        state,
        applied,
        order,
    }
}

fn try_apply(op: &VerifiedOp, state: &mut GovernanceState) -> bool {
    if !validate_write(op.author(), op.body(), state) {
        return false;
    }
    apply(op.author(), op.body(), op.stamp(), state);
    true
}

/// Apply a single governance entry, stamped by its op, to the state.
/// Public for incremental local updates after validation.
pub fn apply_entry(
    author: &PseudonymKey,
    entry: &GovernanceEntry,
    stamp: OpStamp,
    state: &mut GovernanceState,
) {
    apply(author, entry, stamp, state);
}

/// Route an entry to its category-specific CRDT apply rule.
fn apply(author: &PseudonymKey, entry: &GovernanceEntry, stamp: OpStamp, state: &mut GovernanceState) {
    use GovernanceEntry as G;
    match entry {
        G::ChannelCreated { .. }
        | G::ChannelArchived { .. }
        | G::ChannelUpdated { .. }
        | G::CategoryCreated { .. }
        | G::CategoryArchived { .. }
        | G::CategoryUpdated { .. }
        | G::PermissionOverwrite { .. }
        | G::ChannelSegmentLinked { .. } => apply_channels(entry, stamp, state),
        G::MemberApproved { .. } | G::MemberRejected { .. } => {
            apply_admission(entry, stamp, state);
        }
        G::MemberJoined { .. }
        | G::MemberRecordMoved { .. }
        | G::MemberLeft {}
        | G::MemberRemoved { .. } => apply_membership(author, entry, stamp, state),
        // `validate_write` already refused any write after the first,
        // so reaching here means this is the creator's one-and-only
        // policy entry.
        G::AdmissionPolicy { mode, .. } => state.admission_mode = Some(*mode),
        G::CommunityMeta { .. }
        | G::CommunityNotificationDefault { .. }
        | G::MEKGenerationBump { .. }
        | G::SegmentAdded { .. }
        | G::CommunityPolicy { .. }
        | G::InviteCreated { .. }
        | G::InviteRevoked { .. } => apply_community(author, entry, stamp, state),
        G::ThreadCreated { .. }
        | G::ThreadArchived { .. }
        | G::ThreadUnarchived { .. }
        | G::EventCreated { .. }
        | G::EventArchived { .. }
        | G::EventRsvp { .. } => apply_events(author, entry, stamp, state),
        G::ExpressionAdded { .. } | G::ExpressionRemoved { .. } | G::AttachmentPinned { .. } => {
            apply_expression(entry, stamp, state);
        }
        G::BanEntry { .. }
        | G::UnbanEntry { .. }
        | G::TimeoutEntry { .. }
        | G::RemoveTimeoutEntry { .. }
        | G::AdminDelete { .. }
        | G::AutoModRule { .. } => apply_moderation(entry, stamp, state),
        G::ModReport { .. } => apply_mod_report(author, entry, stamp, state),
        G::OnboardingConfig { .. } | G::WelcomeScreen { .. } => apply_onboarding(entry, stamp, state),
        G::RoleDefinition { .. }
        | G::RoleAssignment { .. }
        | G::RoleUnassignment { .. }
        | G::RoleArchived { .. } => apply_roles(entry, stamp, state),
    }
}

mod channels;
mod community;
mod events;
mod expression;
mod gates;
mod membership;
mod moderation;
mod onboarding;
mod resolve;
mod roles;

#[cfg(test)]
mod proptests;
#[cfg(test)]
mod tests;

use channels::apply_channels;
use community::apply_community;
use events::apply_events;
use expression::apply_expression;
use membership::apply_membership;
use moderation::{apply_admission, apply_mod_report, apply_moderation};
use onboarding::apply_onboarding;
use roles::apply_roles;
