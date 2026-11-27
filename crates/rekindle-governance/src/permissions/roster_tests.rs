use std::collections::HashSet;

use rekindle_types::id::RoleId;
use rekindle_types::permissions::{DEFAULT_EVERYONE, READ_HISTORY};

use super::*;
use crate::overwrite::OverwriteTarget;
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
        stamp: crate::fixture::stamp(1),
    }
}

fn state() -> GovernanceState {
    let mut state = GovernanceState {
        creators: std::collections::BTreeSet::from([key(1)]),
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
        (channel, OverwriteTarget::Role(RoleId::EVERYONE).key()),
        OverwriteState {
            target_type: "role".into(),
            allow: 0,
            deny: VIEW_CHANNELS,
            stamp: crate::fixture::stamp(2),
        },
    );
    let reader = RoleId([5; 16]);
    state
        .roles
        .insert(reader, role(VIEW_CHANNELS | READ_HISTORY));
    state.overwrites.insert(
        (channel, OverwriteTarget::Role(reader).key()),
        OverwriteState {
            target_type: "role".into(),
            allow: VIEW_CHANNELS,
            deny: 0,
            stamp: crate::fixture::stamp(2),
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
    assert!(is_restricted(&state, &channel), "@everyone is denied");
    assert!(
        !is_restricted(&state, &ChannelId([8; 16])),
        "no overwrite on the other channel"
    );
    assert_eq!(
        channel_read_roster(&state, &ChannelId([8; 16]), &[key(2)]),
        vec![key(2)],
        "an overwrite on one channel leaves the others readable"
    );
}

fn deny_view(state: &mut GovernanceState, channel: ChannelId, target: String, kind: &str) {
    state.overwrites.insert(
        (channel, target),
        OverwriteState {
            target_type: kind.into(),
            allow: 0,
            deny: VIEW_CHANNELS,
            stamp: crate::fixture::stamp(2),
        },
    );
}

#[test]
fn a_channel_is_restricted_only_by_a_role_or_everyone_view_deny() {
    let channel = ChannelId([9; 16]);
    let mut open = state();
    assert!(!is_restricted(&open, &channel), "no overwrite: open");
    deny_view(&mut open, channel, "ab".repeat(32), "member");
    assert!(
        !is_restricted(&open, &channel),
        "a member overwrite narrows one member, not the channel"
    );
    deny_view(
        &mut open,
        channel,
        OverwriteTarget::Role(RoleId([7; 16])).key(),
        "role",
    );
    assert!(
        !is_restricted(&open, &channel),
        "an overwrite naming no held role is not effective"
    );

    let mut by_everyone = state();
    deny_view(
        &mut by_everyone,
        channel,
        OverwriteTarget::Role(RoleId::EVERYONE).key(),
        "role",
    );
    assert!(is_restricted(&by_everyone, &channel));
    assert!(
        !is_restricted(&by_everyone, &ChannelId([8; 16])),
        "only the overwritten channel is restricted"
    );

    let mut by_role = state();
    let role_id = RoleId([5; 16]);
    by_role.roles.insert(role_id, role(VIEW_CHANNELS));
    deny_view(
        &mut by_role,
        channel,
        OverwriteTarget::Role(role_id).key(),
        "role",
    );
    assert!(is_restricted(&by_role, &channel));
}
