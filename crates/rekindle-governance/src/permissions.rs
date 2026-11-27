//! Permission computation from CRDT-merged governance state.
//!
//! Implements the 8-step Discord-compatible resolution algorithm
//! from architecture doc §9.2. All permission checks in the v2.0
//! system go through `compute_permissions()`.

use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::permissions::{
    ADMINISTRATOR, ALL, ATTACH_FILES, BAN_MEMBERS, CONNECT, DEAFEN_MEMBERS, EMBED_LINKS,
    MANAGE_COMMUNITY, MANAGE_MESSAGES, MENTION_EVERYONE, MUTE_MEMBERS, PRIORITY_SPEAKER,
    READ_HISTORY, SEND_MESSAGES, SPEAK, STREAM, TIMEOUT_MEMBERS, USE_VOICE_ACTIVITY, VIEW_CHANNELS,
};

use crate::overwrite::OverwriteTarget;
use crate::state::GovernanceState;

mod capabilities;
pub use capabilities::{
    channel_denial, channel_permission_name, denied_in_channel, CapabilityDenial,
};

mod roster;
pub use roster::{channel_read_roster, is_restricted};
#[cfg(test)]
mod overwrite_tests;

/// Whether `member` may rotate the community key under `state` — be
/// elected rotator and write a `MEKGenerationBump` (plan D20). Evaluated
/// exactly as `validate_write` evaluates the bump (base permissions, no
/// timeout clock), so the rotator a peer elects is one every reader
/// accepts.
#[must_use]
pub fn may_rotate_mek(member: &PseudonymKey, state: &GovernanceState) -> bool {
    rekindle_types::permissions::Permissions(compute_permissions(member, None, state, 0))
        .may_rotate_mek()
}

/// Whether `member` holds a creator seat now (plan E3.2 S7s, lead
/// decisions (17), (25), (53)): one of the cert's creator set
/// ([`GovernanceState::is_creator`]), not departed, and never departed.
/// Creator power is granted by this, never by cert membership alone: a
/// creator who left gave the seat up for good, a re-join included (its
/// `MemberLeft` folds a `departed_heads` entry, which a re-join keeps and
/// the state root covers), and the creator set changes only through a
/// successor (decisions (24)–(26)). No creator removes, bans or times out
/// another (decision (53)), so a seat ends only by its holder's leave. A
/// successor community pins its own cert, so its seats are its own
/// creator set.
#[must_use]
pub fn holds_creator_seat(member: &PseudonymKey, state: &GovernanceState) -> bool {
    state.is_creator(member)
        && !state.departed_heads.contains_key(member)
        && !state.is_departed(member)
}

/// Compute effective permissions for a member in a specific context.
///
/// # Arguments
/// * `member` — The member's pseudonym key.
/// * `channel_id` — If Some, apply per-channel overwrites. If None, compute base perms.
/// * `state` — The CRDT-merged governance state.
/// * `now_unix` — Current unix timestamp for timeout expiry checks.
///
/// # Returns
/// A u64 bitmask of effective permissions.
pub fn compute_permissions(
    member: &PseudonymKey,
    channel_id: Option<&ChannelId>,
    state: &GovernanceState,
    now_unix: u64,
) -> u64 {
    // Step 0: a seated creator has all permissions (decision (53))
    if holds_creator_seat(member, state) {
        return ALL;
    }

    // Step 1: Start with @everyone role permissions
    let everyone_role_id = RoleId([0u8; 16]);
    let mut perms = state
        .roles
        .get(&everyone_role_id)
        .map_or(0, |r| r.permissions);

    // Step 2: OR all of member's assigned role permissions
    if let Some(member_roles) = state.role_assignments.get(member) {
        for role_id in member_roles {
            if *role_id == everyone_role_id {
                continue; // already included
            }
            if let Some(role) = state.roles.get(role_id) {
                perms |= role.permissions;
            }
        }
    }

    // Step 3: ADMINISTRATOR bypass — return all permissions
    if perms & ADMINISTRATOR != 0 {
        return ALL;
    }

    // Steps 4-6: Channel-specific overwrites (only if channel specified)
    if let Some(ch_id) = channel_id {
        // Step 4: @everyone channel overwrites
        let everyone_key = (*ch_id, OverwriteTarget::Role(everyone_role_id).key());
        if let Some(ow) = state.overwrites.get(&everyone_key) {
            perms &= !ow.deny;
            perms |= ow.allow;
        }

        // Step 5: Role channel overwrites (union allows, then apply denies)
        let mut role_allow: u64 = 0;
        let mut role_deny: u64 = 0;
        if let Some(member_roles) = state.role_assignments.get(member) {
            for role_id in member_roles {
                let key = (*ch_id, OverwriteTarget::Role(*role_id).key());
                if let Some(ow) = state.overwrites.get(&key) {
                    role_allow |= ow.allow;
                    role_deny |= ow.deny;
                }
            }
        }
        perms = (perms & !role_deny) | role_allow;

        // Step 6: Member-specific channel overwrites (highest priority)
        let member_key = (*ch_id, OverwriteTarget::Member(member.clone()).key());
        if let Some(ow) = state.overwrites.get(&member_key) {
            perms &= !ow.deny;
            perms |= ow.allow;
        }
    }

    // Step 7: a timed-out member *retains* at most VIEW_CHANNELS |
    // READ_HISTORY (architecture §9.1 bit 7 "restrict a member to",
    // §20.5 "retains only"; communities-overview §permissions step 8
    // "clamp"). The timeout masks what steps 1-6 computed; it never
    // grants a bit, so a channel an overwrite closes stays closed.
    if let Some(timeout) = state.timeouts.get(member) {
        if !timeout.is_expired(now_unix) {
            perms &= VIEW_CHANNELS | READ_HISTORY;
        }
    }

    // Step 8: Implicit permission dependencies
    if perms & SEND_MESSAGES == 0 {
        perms &= !MENTION_EVERYONE;
        perms &= !ATTACH_FILES;
        perms &= !EMBED_LINKS;
    }
    if perms & VIEW_CHANNELS == 0 {
        perms = 0;
    }
    if perms & CONNECT == 0 {
        perms &= !SPEAK;
        perms &= !MUTE_MEMBERS;
        perms &= !DEAFEN_MEMBERS;
        perms &= !USE_VOICE_ACTIVITY;
        perms &= !PRIORITY_SPEAKER;
        perms &= !STREAM;
    }

    perms
}

/// Whether `perms` carries any moderation capability.
///
/// Architecture §32 W17 — automod alerts go to any member who could act
/// on them, not just those who can hand out timeouts. The spec says
/// "admins"; in the v2.0 permission model that's the union of
/// `ADMINISTRATOR` plus the message/community/timeout/ban moderation bits.
pub fn has_moderation_capability(perms: u64) -> bool {
    const MOD_MASK: u64 =
        ADMINISTRATOR | MANAGE_COMMUNITY | MANAGE_MESSAGES | TIMEOUT_MEMBERS | BAN_MEMBERS;
    perms & MOD_MASK != 0
}

/// Whether an effective-permission bitmask carries the capability (or any
/// capability) named by `capability`.
///
/// Reader-side single-bit gate: `perms & capability != 0`. This is the
/// canonical capability test — callers in `src-tauri` must use it instead
/// of inlining `perms & FLAG != 0`, so all permission-bit interpretation
/// stays in the governance crate (Invariant 7).
#[must_use]
pub fn has_capability(perms: u64, capability: u64) -> bool {
    perms & capability != 0
}

/// Whether an effective-permission bitmask carries *every* bit in `mask`.
///
/// Reader-side all-of gate: `perms & mask == mask`. Use when an action
/// requires the full set of capabilities named by `mask` (e.g. a
/// multi-bit `perm_mask` checked before a privileged voice operation).
#[must_use]
pub fn has_all_capabilities(perms: u64, mask: u64) -> bool {
    perms & mask == mask
}

#[cfg(test)]
#[path = "permissions_tests.rs"]
mod tests;
