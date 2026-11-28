//! Who reads a channel (plan E3.1 E1): the members whose effective
//! permissions in the channel include `VIEW_CHANNELS`. The intent review
//! counts them (`intent_facts::channel_members`) and E3.5 withholds a
//! channel key's wraps from everyone else. A restricted channel's key
//! goes only to its roster (plan E3.5 "Restricted channel, defined";
//! FX3 d: the `RequestMEK` responder's gate).

use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::permissions::VIEW_CHANNELS;

use super::compute_permissions;
use crate::overwrite::OverwriteTarget;
use crate::state::GovernanceState;

/// The members of `members` who can read `channel` under `state`, sorted
/// and without duplicates. Banned members are not readers. Evaluated with
/// no clock: a timeout leaves a member able to read
/// (`compute_permissions` keeps `VIEW_CHANNELS` for one), so it cannot
/// change the roster.
///
/// The governance state names no complete member set (an open community
/// admits without an entry), so the host passes its registry's members,
/// as `intent_facts::leave_escalations` takes them.
#[must_use]
pub fn channel_read_roster(
    state: &GovernanceState,
    channel: &ChannelId,
    members: &[PseudonymKey],
) -> Vec<PseudonymKey> {
    let mut roster: Vec<PseudonymKey> = members
        .iter()
        .filter(|m| !state.bans.contains(*m))
        .filter(|m| compute_permissions(m, Some(channel), state, 0) & VIEW_CHANNELS != 0)
        .cloned()
        .collect();
    roster.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    roster.dedup();
    roster
}

/// Whether `channel` is restricted under `state` (plan E3.5, "Restricted
/// channel, defined"): an effective `PermissionOverwrite` on it denies
/// `VIEW_CHANNELS` to `@everyone` or to any role. Effective means
/// `compute_permissions` applies it: its target is `@everyone` or a role
/// the state holds (a member overwrite narrows one member, not the
/// channel).
#[must_use]
pub fn is_restricted(state: &GovernanceState, channel: &ChannelId) -> bool {
    let everyone = OverwriteTarget::Role(RoleId::EVERYONE).key();
    state
        .overwrites
        .iter()
        .filter(|((ch, _), ow)| ch == channel && ow.deny & VIEW_CHANNELS != 0)
        .any(|((_, target), _)| {
            *target == everyone
                || state
                    .roles
                    .keys()
                    .any(|role| OverwriteTarget::Role(*role).key() == *target)
        })
}

#[cfg(test)]
#[path = "roster_tests.rs"]
mod tests;
