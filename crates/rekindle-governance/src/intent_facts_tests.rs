use std::collections::HashSet;

use super::*;
use crate::state::RoleState;

#[test]
fn the_publish_review_states_the_new_set_who_leaves_and_no_history() {
    let mut state = GovernanceState::default();
    state.creators.extend([key(1), key(2), key(3)]);
    let facts = super::successor_publish_facts(&state, &key(2));
    assert_eq!(
        facts,
        [
            Fact::NewCreatorSet {
                creators: vec![key(2).to_hex()],
            },
            Fact::CreatorsLeave {
                creators: vec![key(1).to_hex(), key(3).to_hex()],
            },
            Fact::NoHistoryCopied,
        ]
    );
}

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
            stamp: crate::fixture::stamp(1),
        },
    );
    role
}

#[test]
fn the_last_admin_and_the_sole_member_are_told() {
    let mut state = GovernanceState {
        creators: std::collections::BTreeSet::from([key(1)]),
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
        creators: std::collections::BTreeSet::from([key(1)]),
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

#[test]
fn a_forward_to_other_readers_names_both_audiences() {
    let mut state = GovernanceState {
        creators: std::collections::BTreeSet::from([key(1)]),
        ..GovernanceState::default()
    };
    let channel = ChannelId([3; 16]);
    let everyone = [key(1), key(2)];
    let audience = |members| ChannelAudience {
        state: &state,
        channel: &channel,
        members,
    };
    assert!(
        forward_escalations(audience(&everyone), audience(&[key(1)])).is_empty(),
        "only the creator reads either channel"
    );

    let role = admin_role(&mut state);
    state.role_assignments.insert(key(2), HashSet::from([role]));
    let audience = |members| ChannelAudience {
        state: &state,
        channel: &channel,
        members,
    };
    assert!(forward_escalations(audience(&everyone), audience(&everyone)).is_empty());
    assert_eq!(
        forward_escalations(audience(&everyone), audience(&[key(1)])),
        vec![Escalation::CrossAudience {
            from_members: 2,
            to_members: 1
        }]
    );
}
