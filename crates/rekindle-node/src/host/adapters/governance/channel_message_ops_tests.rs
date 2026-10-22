//! The admin-delete refusals (plan E3.1 E2d): what readers would drop is
//! refused before it is written.

use std::collections::{HashMap, HashSet};

use rekindle_governance::state::{GovernanceState, RoleState};
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::permissions::MANAGE_MESSAGES;

use super::{may_delete_messages, message_id_bytes, ModerationRefusal};

fn key(byte: u8) -> PseudonymKey {
    PseudonymKey([byte; 32])
}

const CHANNEL: ChannelId = ChannelId([5; 16]);

/// A community whose creator is `key(1)`; `key(2)` holds a role with
/// `MANAGE_MESSAGES`, `key(3)` holds none.
fn community() -> GovernanceState {
    let role = RoleId([9; 16]);
    GovernanceState {
        creator: Some(key(1)),
        roles: HashMap::from([(
            role,
            RoleState {
                name: "moderator".into(),
                permissions: MANAGE_MESSAGES,
                position: 1,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
                lamport: 1,
            },
        )]),
        role_assignments: HashMap::from([(key(2), HashSet::from([role]))]),
        ..GovernanceState::default()
    }
}

#[test]
fn a_member_without_manage_messages_is_refused() {
    let refused = may_delete_messages(&community(), &key(3), CHANNEL).unwrap_err();
    assert!(
        matches!(refused, ModerationRefusal::NotPermitted(_)),
        "{refused}"
    );
    assert_eq!(refused.code(), 403);
}

#[test]
fn the_creator_and_a_moderator_may_delete() {
    let state = community();
    assert!(may_delete_messages(&state, &key(1), CHANNEL).is_ok());
    assert!(may_delete_messages(&state, &key(2), CHANNEL).is_ok());
}

#[test]
fn a_banned_moderator_is_refused() {
    let mut state = community();
    state.bans.insert(key(2));
    assert!(may_delete_messages(&state, &key(2), CHANNEL).is_err());
}

#[test]
fn a_malformed_message_id_is_not_written() {
    let id = [7u8; 16];
    assert_eq!(
        message_id_bytes(&format!("msg_{}", hex::encode(id))),
        Some(id)
    );
    assert_eq!(
        message_id_bytes(&uuid::Uuid::from_bytes(id).hyphenated().to_string()),
        Some(id)
    );
    assert_eq!(message_id_bytes("msg_not-an-id"), None);
    assert_eq!(message_id_bytes(""), None);
    assert_eq!(ModerationRefusal::BadMessage.code(), 400);
}
