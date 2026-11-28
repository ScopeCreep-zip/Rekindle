//! Per-target moderation verdicts (plan E3.2 S9): what a member may do to
//! another member, from the same rule [`super::validate_write`] applies to
//! the entry, so a frontend shows exactly the actions every reader accepts
//! (architecture §9.3, MSC4289's immutable creator).

use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::{
    BAN_MEMBERS, DEAFEN_MEMBERS, KICK_MEMBERS, MANAGE_ROLES, MUTE_MEMBERS, TIMEOUT_MEMBERS,
};

use super::has;
use crate::permissions::{compute_permissions, holds_creator_seat};
use crate::state::GovernanceState;

/// A moderation action against one member: a governance entry, or a
/// voice moderation (server mute or deafen) under the same permission and
/// rank rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetAction {
    /// `MemberRemoved`: the durable kick.
    Kick,
    /// `BanEntry`.
    Ban,
    /// `TimeoutEntry`.
    Timeout,
    /// `RoleUnassignment` of a role the target cannot self-assign (and the
    /// matching `RoleAssignment`): the validator's `MANAGE_ROLES` arm.
    ManageRoles,
    /// Server mute: `MUTE_MEMBERS` on a member ranked below the writer
    /// (the voice moderation [`outranks`] gates). It writes no governance
    /// entry; whether the target is in voice is not known to the Host
    /// before E3.4 S8b, so that reason is not given here.
    Mute,
    /// Server deafen: `DEAFEN_MEMBERS`, the same rule as [`Self::Mute`].
    Deafen,
}

/// Why `writer` may not take an action against `target` (plan E3.2 S9):
/// the first rule of [`may_moderate`] that refuses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRefusal {
    /// The target holds a creator seat (decision (53), MSC4289).
    SeatedCreator,
    /// The writer is banned.
    WriterBanned,
    /// The writer lacks the action's permission, named as the role editor
    /// shows it.
    Missing { permission: &'static str },
    /// The target's highest role is not strictly below the writer's.
    Rank,
    /// The writer may manage this member's roles, but the community has
    /// no roles at all ([`role_menu_refusal`]).
    NoRoles,
    /// The writer may manage this member's roles, but every role sits at
    /// or above the writer's highest one, so none can be given
    /// ([`role_menu_refusal`]).
    NoAssignableRoles,
}

impl TargetAction {
    const fn bit(self) -> u64 {
        match self {
            Self::Kick => KICK_MEMBERS,
            Self::Ban => BAN_MEMBERS,
            Self::Timeout => TIMEOUT_MEMBERS,
            Self::ManageRoles => MANAGE_ROLES,
            Self::Mute => MUTE_MEMBERS,
            Self::Deafen => DEAFEN_MEMBERS,
        }
    }

    /// The permission's name as the role editor shows it.
    const fn permission_name(self) -> &'static str {
        match self {
            Self::Kick => "Kick Members",
            Self::Ban => "Ban Members",
            Self::Timeout => "Moderate Members",
            Self::ManageRoles => "Manage Roles",
            Self::Mute => "Mute Members",
            Self::Deafen => "Deafen Members",
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
    moderation_refusal(writer, target, action, state).is_none()
}

/// Why `writer`'s `action` against `target` fails [`may_moderate`], the
/// rules in its order; `None` when it passes. The refusals past the
/// creator and ban rules are [`acts_on_lower`]'s three conditions.
#[must_use]
pub fn moderation_refusal(
    writer: &PseudonymKey,
    target: &PseudonymKey,
    action: TargetAction,
    state: &GovernanceState,
) -> Option<TargetRefusal> {
    if holds_creator_seat(writer, state) {
        return (!creator_may_target(writer, target, state))
            .then_some(TargetRefusal::SeatedCreator);
    }
    if state.bans.contains(writer) {
        return Some(TargetRefusal::WriterBanned);
    }
    let perms = compute_permissions(writer, None, state, 0);
    if !has(perms, action.bit()) {
        return Some(TargetRefusal::Missing {
            permission: action.permission_name(),
        });
    }
    if holds_creator_seat(target, state) {
        return Some(TargetRefusal::SeatedCreator);
    }
    (state.member_max_position(target) >= state.member_max_position(writer))
        .then_some(TargetRefusal::Rank)
}

/// Why the role menu on `target` offers `writer` nothing (plan E3.2 S9):
/// [`moderation_refusal`]'s `ManageRoles` verdict, else no role exists,
/// else no role ranks strictly below the writer's highest (the
/// validator's `RoleAssignment` rank rule; a seated creator may give any
/// role). `None` when at least one role can be given.
#[must_use]
pub fn role_menu_refusal(
    writer: &PseudonymKey,
    target: &PseudonymKey,
    state: &GovernanceState,
) -> Option<TargetRefusal> {
    if let Some(refusal) = moderation_refusal(writer, target, TargetAction::ManageRoles, state) {
        return Some(refusal);
    }
    if state.roles.is_empty() {
        return Some(TargetRefusal::NoRoles);
    }
    if holds_creator_seat(writer, state) {
        return None;
    }
    let mine = state.member_max_position(writer);
    (!state.roles.values().any(|role| role.position < mine))
        .then_some(TargetRefusal::NoAssignableRoles)
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
