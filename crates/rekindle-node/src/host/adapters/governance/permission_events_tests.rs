//! `permission_transitions`: one member per changed effective permission
//! set and one channel per changed overwrite set, nothing for a merge that
//! moves neither.

use rekindle_governance::fixture::stamp;
use rekindle_governance::op::OpStamp;
use rekindle_governance::state::{
    ChannelState, GovernanceState, MemberState, OverwriteState, RoleState, TimeoutState,
};
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::permissions::{SEND_MESSAGES, VIEW_CHANNELS};

use rekindle_types::dto::community_capability::CommunityCapability;

use super::{permission_transitions, PermissionTransitions};

/// The members a transition names, in order.
fn moved(changes: &PermissionTransitions) -> Vec<String> {
    changes.members.keys().cloned().collect()
}

fn key(byte: u8) -> PseudonymKey {
    PseudonymKey([byte; 32])
}

/// Two members, one channel, and an `@everyone` that may view and send.
fn base() -> GovernanceState {
    let mut state = GovernanceState::default();
    state.roles.insert(
        RoleId([0; 16]),
        RoleState {
            name: "@everyone".into(),
            permissions: VIEW_CHANNELS | SEND_MESSAGES,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            stamp: OpStamp::MIN,
        },
    );
    for m in [1, 2] {
        state.members.insert(
            key(m),
            MemberState {
                member_record: "record".into(),
                member_record_owner: "owner".into(),
                joined: OpStamp::MIN,
            },
        );
    }
    state.channels.insert(
        ChannelId([7; 16]),
        ChannelState {
            name: "general".into(),
            channel_type: "text".into(),
            category_id: None,
            position: 0,
            topic: None,
            forum_tags: None,
            slowmode_seconds: None,
            nsfw: None,
            parent_voice_channel_id: None,
            created: OpStamp::MIN,
        },
    );
    state
}

#[test]
fn an_unchanged_merge_is_no_transition() {
    assert!(permission_transitions(&base(), &base(), &base().diff(&base()), 100).is_empty());
}

#[test]
fn a_member_overwrite_moves_that_member_and_its_channel() {
    let prev = base();
    let mut next = prev.clone();
    next.overwrites.insert(
        (ChannelId([7; 16]), key(2).to_hex()),
        OverwriteState {
            target_type: "member".into(),
            allow: 0,
            deny: SEND_MESSAGES,
            stamp: stamp(3),
        },
    );
    let changes = permission_transitions(&prev, &next, &prev.diff(&next), 100);
    assert_eq!(moved(&changes), vec![key(2).to_hex()]);
    assert_eq!(changes.channels, vec![hex::encode([7; 16])]);
}

#[test]
fn a_timeout_moves_the_member_and_its_expiry_between_merges_does_not() {
    let prev = base();
    let mut next = prev.clone();
    next.timeouts.insert(
        key(1),
        TimeoutState {
            duration_seconds: 60,
            started_at: 100,
            reason: None,
            stamp: stamp(4),
        },
    );
    assert_eq!(
        moved(&permission_transitions(
            &prev,
            &next,
            &prev.diff(&next),
            120
        )),
        vec![key(1).to_hex()]
    );
    // Run out: both sides are read at the same instant, so no change.
    assert!(
        permission_transitions(&next, &next.clone(), &next.diff(&next.clone()), 500).is_empty()
    );
}

/// A co-creator's seat ends only by its own leave (lead decision (53)),
/// merged as a real `MemberLeft`: the member is named.
#[test]
fn a_co_creator_who_left_is_named() {
    use rekindle_governance::fixture::{at, cert, merge_entries_under};
    use rekindle_types::governance::{AdmissionMode, GenesisCert, GovernanceEntry};
    let cert = GenesisCert {
        additional_creators: vec![key(2)],
        ..cert(&key(1), AdmissionMode::default())
    };
    let joined = GovernanceEntry::MemberJoined {
        member_record: "VLD0:record-2".into(),
        member_record_owner: "owner-2".into(),
        invite_proof: None,
        owner_proof: vec![0; 64],
        first_device: None,
    };
    let left = GovernanceEntry::MemberLeft { heads: Vec::new() };
    let prev = merge_entries_under(&cert, &[(key(2), vec![at(5, joined.clone())])]);
    let next = merge_entries_under(&cert, &[(key(2), vec![at(5, joined), at(6, left)])]);
    assert_eq!(
        moved(&permission_transitions(&prev, &next, &prev.diff(&next), 0)),
        vec![key(2).to_hex()]
    );
}

/// Granting a member a role that carries `MANAGE_CHANNELS` names it, and
/// its event's map no longer denies the channel controls; the revoke
/// denies them again with the permission that grants them.
#[test]
fn a_grant_carries_the_new_denied_map() {
    use rekindle_types::permissions::MANAGE_CHANNELS;
    let mut prev = base();
    prev.roles.insert(
        RoleId([5; 16]),
        RoleState {
            name: "Builders".into(),
            permissions: MANAGE_CHANNELS,
            position: 1,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            stamp: OpStamp::MIN,
        },
    );
    let mut next = prev.clone();
    next.role_assignments
        .entry(key(1))
        .or_default()
        .insert(RoleId([5; 16]));
    let plain = permission_transitions(&next, &prev, &next.diff(&prev), 100);
    assert_eq!(
        plain.members[&key(1).to_hex()].get(&CommunityCapability::ManageChannels),
        Some(&"You need the Manage Channels permission.".to_owned())
    );
    let granted = permission_transitions(&prev, &next, &prev.diff(&next), 100);
    assert_eq!(moved(&granted), vec![key(1).to_hex()]);
    let denied = &granted.members[&key(1).to_hex()];
    for cap in [
        CommunityCapability::ManageChannels,
        CommunityCapability::SetTopic,
        CommunityCapability::ManageServers,
    ] {
        assert!(!denied.contains_key(&cap), "{cap:?} still denied");
    }
    assert_eq!(
        denied.get(&CommunityCapability::ManageRoles),
        Some(&"You need the Manage Roles permission.".to_owned())
    );
}
