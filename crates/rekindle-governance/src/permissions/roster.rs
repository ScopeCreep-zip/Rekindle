//! Who reads a channel (plan E3.1 E1): the members whose effective
//! permissions in the channel include `VIEW_CHANNELS`. The intent review
//! counts them (`intent_facts::channel_members`) and E3.5 withholds a
//! channel key's wraps from everyone else.

use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::permissions::VIEW_CHANNELS;

use super::compute_permissions;
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rekindle_types::id::RoleId;
    use rekindle_types::permissions::{DEFAULT_EVERYONE, READ_HISTORY};

    use super::*;
    use crate::state::{OverwriteState, RoleState};

    fn key(b: u8) -> PseudonymKey {
        PseudonymKey([b; 32])
    }

    fn role(permissions: u64) -> RoleState {
        RoleState {
            name: "r".into(),
            permissions,
            position: 1,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 1,
        }
    }

    fn state() -> GovernanceState {
        let mut state = GovernanceState {
            creator: Some(key(1)),
            ..GovernanceState::default()
        };
        state.roles.insert(RoleId([0; 16]), role(DEFAULT_EVERYONE));
        state
    }

    #[test]
    fn every_member_reads_an_open_channel_once_and_in_order() {
        let channel = ChannelId([9; 16]);
        let roster = channel_read_roster(&state(), &channel, &[key(3), key(1), key(2), key(3)]);
        assert_eq!(roster, vec![key(1), key(2), key(3)]);
    }

    #[test]
    fn a_denied_or_banned_member_is_not_a_reader() {
        let channel = ChannelId([9; 16]);
        let mut state = state();
        state.overwrites.insert(
            (channel, format!("{:?}", RoleId([0; 16]))),
            OverwriteState {
                target_type: "role".into(),
                allow: 0,
                deny: VIEW_CHANNELS,
                lamport: 2,
            },
        );
        let reader = RoleId([5; 16]);
        state
            .roles
            .insert(reader, role(VIEW_CHANNELS | READ_HISTORY));
        state.overwrites.insert(
            (channel, format!("{reader:?}")),
            OverwriteState {
                target_type: "role".into(),
                allow: VIEW_CHANNELS,
                deny: 0,
                lamport: 2,
            },
        );
        state
            .role_assignments
            .insert(key(3), HashSet::from([reader]));
        state.bans.insert(key(4));
        state
            .role_assignments
            .insert(key(4), HashSet::from([reader]));

        let roster = channel_read_roster(&state, &channel, &[key(1), key(2), key(3), key(4)]);
        assert_eq!(
            roster,
            vec![key(1), key(3)],
            "the creator reads everything, @everyone is denied, the role overwrite allows, a ban removes"
        );
        assert_eq!(
            channel_read_roster(&state, &ChannelId([8; 16]), &[key(2)]),
            vec![key(2)],
            "an overwrite on one channel leaves the others readable"
        );
    }
}
