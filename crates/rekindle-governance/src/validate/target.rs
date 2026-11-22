//! Per-target moderation verdicts (plan E3.2 S9): what a member may do to
//! another member, from the same rule [`super::validate_write`] applies to
//! the entry, so a frontend shows exactly the actions every reader accepts
//! (architecture §9.3, MSC4289's immutable creator).

use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::{BAN_MEMBERS, KICK_MEMBERS, TIMEOUT_MEMBERS};

use super::has;
use crate::permissions::{compute_permissions, holds_creator_seat};
use crate::state::GovernanceState;

/// A moderation action against one member, each a governance entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetAction {
    /// `MemberRemoved`: the durable kick.
    Kick,
    /// `BanEntry`.
    Ban,
    /// `TimeoutEntry`.
    Timeout,
}

impl TargetAction {
    const fn bit(self) -> u64 {
        match self {
            Self::Kick => KICK_MEMBERS,
            Self::Ban => BAN_MEMBERS,
            Self::Timeout => TIMEOUT_MEMBERS,
        }
    }
}

/// The rank-strict rule shared by the kick, ban and timeout arms of the
/// validator: the writer holds `bit`, the target holds no creator seat, and
/// the target's highest role ranks strictly below the writer's.
pub(super) fn acts_on_lower(
    perms: u64,
    bit: u64,
    writer: &PseudonymKey,
    target: &PseudonymKey,
    state: &GovernanceState,
) -> bool {
    has(perms, bit)
        && !holds_creator_seat(target, state)
        && state.member_max_position(target) < state.member_max_position(writer)
}

/// The rank rule of a moderator's delete (plan E3.3 M1): the deleter
/// holds `bit` and a role no lower than the post's author's. It is the
/// rank half of [`acts_on_lower`] made non-strict, as M1 words it ("a role
/// ≥ the author's"): a moderator may remove a peer moderator's post but
/// not a higher one's, and no one but the creator (who passes the
/// validator's creator bypass first) removes the creator's.
pub(super) fn acts_on_post_of(
    perms: u64,
    bit: u64,
    writer: &PseudonymKey,
    author: &PseudonymKey,
    state: &GovernanceState,
) -> bool {
    has(perms, bit)
        && !holds_creator_seat(author, state)
        && state.member_max_position(author) <= state.member_max_position(writer)
}

/// Whether a seated creator `writer` may act on `target` (lead decision
/// (53), Matrix power levels: a kick or ban needs a level strictly above
/// the target's): on itself, or on anyone holding no creator seat, so
/// equal rank never removes, bans, times out or demotes equal rank.
#[must_use]
pub fn creator_may_target(
    writer: &PseudonymKey,
    target: &PseudonymKey,
    state: &GovernanceState,
) -> bool {
    writer == target || !holds_creator_seat(target, state)
}

/// Whether `writer`'s `action` against `target` passes the validator's
/// permission half (the entry itself well formed): a seated creator on
/// anyone but another seated creator ([`creator_may_target`]), a banned
/// writer never, else [`acts_on_lower`].
#[must_use]
pub fn may_moderate(
    writer: &PseudonymKey,
    target: &PseudonymKey,
    action: TargetAction,
    state: &GovernanceState,
) -> bool {
    if holds_creator_seat(writer, state) {
        return creator_may_target(writer, target, state);
    }
    if state.bans.contains(writer) {
        return false;
    }
    let perms = compute_permissions(writer, None, state, 0);
    acts_on_lower(perms, action.bit(), writer, target, state)
}

/// Whether `writer` outranks `target` in the role hierarchy, as the
/// validator's `RoleUnassignment` arm decides it: a seated creator
/// outranks everyone but another seated creator (equal rank, decision
/// (53)), no one outranks a seated creator, else the target's highest
/// role ranks strictly below the writer's. No one outranks themselves.
#[must_use]
pub fn outranks(writer: &PseudonymKey, target: &PseudonymKey, state: &GovernanceState) -> bool {
    if writer == target {
        return false;
    }
    if holds_creator_seat(writer, state) {
        return !holds_creator_seat(target, state);
    }
    !holds_creator_seat(target, state)
        && !state.bans.contains(writer)
        && state.member_max_position(target) < state.member_max_position(writer)
}
