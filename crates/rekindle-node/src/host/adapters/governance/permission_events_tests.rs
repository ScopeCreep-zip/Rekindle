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

use super::{permission_transitions, PermissionTransitions};

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
    assert!(permission_transitions(&base(), &base(), 100).is_empty());
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
    assert_eq!(
        permission_transitions(&prev, &next, 100),
        PermissionTransitions {
            members: vec![key(2).to_hex()],
            channels: vec![hex::encode([7; 16])],
        }
    );
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
        permission_transitions(&prev, &next, 120).members,
        vec![key(1).to_hex()]
    );
    // Run out: both sides are read at the same instant, so no change.
    assert!(permission_transitions(&next, &next.clone(), 500).is_empty());
}
