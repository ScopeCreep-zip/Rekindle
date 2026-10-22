//! Governance facts an intent review states (plan U1.4; ADR 0019): who
//! manages the community and how many hold a role. Pure, like the rest of
//! the crate; the host passes the current member set from its registry.

use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::intent::Escalation;
use rekindle_types::permissions::ADMINISTRATOR;

use crate::permissions::{channel_read_roster, compute_permissions};
use crate::state::GovernanceState;

/// Whether `member` can manage the community (the creator, or a role with
/// ADMINISTRATOR), and is not banned.
#[must_use]
pub fn can_manage(state: &GovernanceState, member: &PseudonymKey, now_unix: u64) -> bool {
    !state.bans.contains(member)
        && compute_permissions(member, None, state, now_unix) & ADMINISTRATOR != 0
}

/// Escalations for `me` leaving: E6 when no one else could manage the
/// community afterwards, or no one else is in it.
#[must_use]
pub fn leave_escalations(
    state: &GovernanceState,
    me: &PseudonymKey,
    members: &[PseudonymKey],
    now_unix: u64,
) -> Vec<Escalation> {
    let others: Vec<&PseudonymKey> = members
        .iter()
        .filter(|m| *m != me && !state.bans.contains(m))
        .collect();
    if others.is_empty() {
        return vec![Escalation::SoleMember];
    }
    if can_manage(state, me, now_unix) && !others.iter().any(|m| can_manage(state, m, now_unix)) {
        return vec![Escalation::LastAdmin];
    }
    Vec::new()
}

/// How many members hold `role`.
#[must_use]
pub fn role_holders(state: &GovernanceState, role: &RoleId) -> u32 {
    let held = state
        .role_assignments
        .values()
        .filter(|roles| roles.contains(role))
        .count();
    u32::try_from(held).unwrap_or(u32::MAX)
}

/// How many of `members` read `channel` (`channel_read_roster`): the
/// audience a channel deletion or a forward reaches.
#[must_use]
pub fn channel_members(
    state: &GovernanceState,
    channel: &ChannelId,
    members: &[PseudonymKey],
) -> u32 {
    u32::try_from(channel_read_roster(state, channel, members).len()).unwrap_or(u32::MAX)
}

/// The E7 escalation for deleting `channel`: its stored `messages`, and
/// the members who read it (`channel_members`). None when it holds neither.
#[must_use]
pub fn channel_delete_escalations(
    state: &GovernanceState,
    channel: &ChannelId,
    members: &[PseudonymKey],
    messages: u32,
) -> Vec<Escalation> {
    let members = channel_members(state, channel, members);
    if messages == 0 && members == 0 {
        return Vec::new();
    }
    vec![Escalation::ContainerNotEmpty { messages, members }]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::state::RoleState;

    fn key(b: u8) -> PseudonymKey {
        PseudonymKey([b; 32])
    }

    fn admin_role(state: &mut GovernanceState) -> RoleId {
        let role = RoleId([7; 16]);
        state.roles.insert(
            role,
            RoleState {
                name: "admin".into(),
                permissions: ADMINISTRATOR,
                position: 1,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
                lamport: 1,
            },
        );
        role
    }

    #[test]
    fn the_last_admin_and_the_sole_member_are_told() {
        let mut state = GovernanceState {
            creator: Some(key(1)),
            ..GovernanceState::default()
        };
        let members = [key(1), key(2)];
        assert_eq!(
            leave_escalations(&state, &key(1), &members, 0),
            vec![Escalation::LastAdmin]
        );
        assert!(leave_escalations(&state, &key(2), &members, 0).is_empty());
        assert_eq!(
            leave_escalations(&state, &key(1), &[key(1)], 0),
            vec![Escalation::SoleMember]
        );

        let role = admin_role(&mut state);
        state.role_assignments.insert(key(2), HashSet::from([role]));
        assert!(leave_escalations(&state, &key(1), &members, 0).is_empty());
        assert_eq!(role_holders(&state, &role), 1);

        state.bans.insert(key(2));
        assert_eq!(
            leave_escalations(&state, &key(1), &members, 0),
            vec![Escalation::SoleMember],
            "a banned member is not one"
        );
    }

    #[test]
    fn a_channel_with_history_or_readers_is_not_empty() {
        let state = GovernanceState {
            creator: Some(key(1)),
            ..GovernanceState::default()
        };
        let channel = ChannelId([3; 16]);
        assert_eq!(channel_members(&state, &channel, &[key(1), key(2)]), 1);
        assert_eq!(
            channel_delete_escalations(&state, &channel, &[key(1)], 12),
            vec![Escalation::ContainerNotEmpty {
                messages: 12,
                members: 1
            }]
        );
        assert!(channel_delete_escalations(&state, &channel, &[], 0).is_empty());
    }
}
