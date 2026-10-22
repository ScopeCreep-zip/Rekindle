//! `AdminDelete` is checked against the writer's permissions in the
//! message's channel: `MANAGE_MESSAGES` after that channel's overwrites,
//! and only where the writer can read.

use std::collections::{HashMap, HashSet};

use rekindle_types::id::{ChannelId, RoleId};
use rekindle_types::permissions::{DEFAULT_EVERYONE, VIEW_CHANNELS};

use super::super::validate_write;
use crate::state::{GovernanceState, OverwriteState, RoleState};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::MANAGE_MESSAGES;

const DELETE_CHANNEL: ChannelId = ChannelId([5; 16]);
const MODERATOR_ROLE: RoleId = RoleId([9; 16]);

fn key(byte: u8) -> PseudonymKey {
    PseudonymKey([byte; 32])
}

fn role(name: &str, permissions: u64, position: u32) -> RoleState {
    RoleState {
        name: name.into(),
        permissions,
        position,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
        lamport: 1,
    }
}

/// `key(1)` created the community; `key(2)` holds a role with
/// `MANAGE_MESSAGES`; `key(3)` holds only @everyone.
fn community() -> GovernanceState {
    GovernanceState {
        creator: Some(key(1)),
        roles: HashMap::from([
            (RoleId([0; 16]), role("everyone", DEFAULT_EVERYONE, 0)),
            (MODERATOR_ROLE, role("moderator", MANAGE_MESSAGES, 1)),
        ]),
        role_assignments: HashMap::from([(key(2), HashSet::from([MODERATOR_ROLE]))]),
        ..GovernanceState::default()
    }
}

fn delete() -> GovernanceEntry {
    GovernanceEntry::AdminDelete {
        message_id: [7; 16],
        channel_id: DELETE_CHANNEL,
        reason: None,
        lamport: 4,
    }
}

/// A member overwrite's target id: the pseudonym in lowercase hex.
fn member_hex(member: &PseudonymKey) -> String {
    use std::fmt::Write as _;
    member
        .0
        .iter()
        .fold(String::with_capacity(64), |mut acc, b| {
            let _ = write!(acc, "{b:02x}");
            acc
        })
}

fn deny_for_moderator(state: &mut GovernanceState, deny: u64) {
    state.overwrites.insert(
        (DELETE_CHANNEL, member_hex(&key(2))),
        OverwriteState {
            target_type: "member".into(),
            allow: 0,
            deny,
            lamport: 3,
        },
    );
}

#[test]
fn a_manage_messages_holder_may_delete_in_a_channel_they_read() {
    assert!(validate_write(&key(2), &delete(), &community()));
}

#[test]
fn a_member_without_manage_messages_may_not_delete() {
    assert!(!validate_write(&key(3), &delete(), &community()));
}

#[test]
fn a_channel_hidden_from_the_moderator_refuses_the_delete() {
    let mut state = community();
    deny_for_moderator(&mut state, VIEW_CHANNELS);
    assert!(!validate_write(&key(2), &delete(), &state));
}

#[test]
fn a_channel_denying_manage_messages_refuses_the_delete() {
    let mut state = community();
    deny_for_moderator(&mut state, MANAGE_MESSAGES);
    assert!(!validate_write(&key(2), &delete(), &state));
}
