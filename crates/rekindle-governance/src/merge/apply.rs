//! `merge` op folding: judging an op against the state, its CRDT apply
//! rule, and the departure and removal facts its fold records.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::op::{OpStamp, VerifiedOp};
use crate::state::{AdminDeleteState, GovernanceState};
use crate::validate::validate_write;

use super::channels::apply_channels;
use super::community::apply_community;
use super::events::apply_events;
use super::expression::apply_expression;
use super::membership::apply_membership;
use super::moderation::{apply_admission, apply_mod_report, apply_moderation};
use super::onboarding::apply_onboarding;
use super::roles::apply_roles;

pub(super) fn try_apply(op: &VerifiedOp, state: &mut GovernanceState) -> bool {
    tally!(try_apply, 1);
    if !validate_write(op.author(), op.body(), state) {
        return false;
    }
    fold(op, state);
    true
}

/// The effects of an op already judged: its CRDT apply rule, the slot
/// heads a departure states, and who removed whom.
pub(super) fn fold(op: &VerifiedOp, state: &mut GovernanceState) {
    apply(op.author(), op.body(), op.stamp(), state);
    // Every departure states the member's slot heads: the remover's view
    // for a kick, the leaver's own for a leave (A/B finding R3-8). Readers
    // judge by every merged departure's heads: a later departure from
    // another slot never drops what an earlier one kept (A/B F8-1).
    let departed = match op.body() {
        GovernanceEntry::MemberLeft { heads } => Some((op.author(), heads)),
        GovernanceEntry::MemberRemoved { target, heads, .. } => Some((target, heads)),
        _ => None,
    };
    if let Some((gone, heads)) = departed {
        let held = state.departed_heads.entry(gone.clone()).or_default();
        for head in heads {
            (held.entry(head.record_key.clone()).or_default()).insert((head.seq, head.digest));
        }
    }
    let remover_of = match op.body() {
        GovernanceEntry::MemberRemoved { target, .. } => {
            state.removed.get(target).is_some_and(|s| *s == op.stamp())
        }
        GovernanceEntry::BanEntry { target, .. } => state
            .ban_stamps
            .get(target)
            .is_some_and(|s| *s == op.stamp()),
        _ => false,
    }
    .then(|| crate::power::removal_target(op.body()))
    .flatten();
    if let Some(target) = remover_of {
        state.removed_by.insert(target.clone(), op.author().clone());
        match removal_reason(op.body()) {
            Some(reason) => state
                .removal_reasons
                .insert(target.clone(), reason.to_owned()),
            None => state.removal_reasons.remove(target),
        };
    }
}

/// The reason a removal op gives, when it gives one.
fn removal_reason(entry: &GovernanceEntry) -> Option<&str> {
    match entry {
        GovernanceEntry::MemberRemoved { reason, .. }
        | GovernanceEntry::BanEntry { reason, .. } => reason.as_deref().filter(|r| !r.is_empty()),
        _ => None,
    }
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
fn apply(
    author: &PseudonymKey,
    entry: &GovernanceEntry,
    stamp: OpStamp,
    state: &mut GovernanceState,
) {
    use GovernanceEntry as G;
    match entry {
        G::ChannelCreated { .. }
        | G::ChannelArchived { .. }
        | G::ChannelUpdated { .. }
        | G::CategoryCreated { .. }
        | G::CategoryArchived { .. }
        | G::CategoryUpdated { .. }
        | G::PermissionOverwrite { .. } => apply_channels(entry, stamp, state),
        G::MemberApproved { .. } | G::MemberRejected { .. } => {
            apply_admission(entry, stamp, state);
        }
        G::MemberJoined { .. }
        | G::MemberRecordMoved { .. }
        | G::MemberLeft { .. }
        | G::MemberRemoved { .. } => apply_membership(author, entry, stamp, state),
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
        | G::AutoModRule { .. } => apply_moderation(entry, stamp, state),
        // A tombstone (plan E3.3 M4): the first delete merged keeps its
        // moderator, so every reader names the same one.
        G::AdminDelete {
            message_id,
            channel_id,
            author: post_author,
            ..
        } => {
            (state.admin_deletes.entry(*message_id)).or_insert_with(|| AdminDeleteState {
                by: author.clone(),
                channel_id: *channel_id,
                author: post_author.clone(),
            });
        }
        G::ModReport { .. } => apply_mod_report(author, entry, stamp, state),
        G::OnboardingConfig { .. } | G::WelcomeScreen { .. } => {
            apply_onboarding(entry, stamp, state);
        }
        G::RoleDefinition { .. }
        | G::RoleAssignment { .. }
        | G::RoleUnassignment { .. }
        | G::RoleArchived { .. } => apply_roles(entry, stamp, state),
        // A hint over a cut changes no state (plan E3.2 S7c.3). The device
        // endorsement set is harvested from every carrier, quarantined ones
        // included (ADR 0022 rule 2), and a readmit counts in the deps of
        // the op it admits (rule 7): `resolve::prepare` harvests them from
        // the op set into `GovernanceState::devices` (K-CP), never here.
        G::Checkpoint(_) | G::DeviceKey(_) | G::DeviceRetired(_) | G::DeviceReadmit(_) => {}
        // Kept whole: the gates judge acceptances against applied offers,
        // and readers show every successor (plan E3.2 S7s.3e).
        G::CommunitySuccessor(successor) => {
            let row = crate::successor::SuccessorState {
                author: author.clone(),
                successor: successor.clone(),
                stamp,
            };
            state.successors.insert(stamp.hash, row);
        }
        G::SuccessorLinkRequest(_) | G::SuccessorLink(_) => {
            crate::link_round::apply(author, entry, stamp, state);
        }
    }
}
