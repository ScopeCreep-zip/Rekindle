//! The admin-delete refusals (plan E3.1 E2d): what readers would drop is
//! refused before it is written.

use rekindle_governance::fixture::{at, merge_entries, Stamped};
use rekindle_governance::state::GovernanceState;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::permissions::{DEFAULT_EVERYONE, MANAGE_MESSAGES};

use super::{may_delete_messages, message_id_bytes, ModerationRefusal};

fn key(byte: u8) -> PseudonymKey {
    PseudonymKey([byte; 32])
}

const MODERATED_CHANNEL: ChannelId = ChannelId([5; 16]);

/// A role at `position` (its lamport is one above, so @everyone comes first).
fn role_definition(role_id: RoleId, name: &str, permissions: u64, position: u32) -> Stamped {
    let entry = GovernanceEntry::RoleDefinition {
        role_id,
        name: name.into(),
        permissions,
        position,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
    };
    at(u64::from(position) + 1, entry)
}

/// A community merged from its creator's entries: `key(1)` writes the
/// @everyone role (the genesis entry), a moderator role with
/// `MANAGE_MESSAGES` and its `RoleAssignment` to `key(2)`; `key(3)` holds
/// only @everyone.
fn community() -> GovernanceState {
    let moderator = RoleId([9; 16]);
    merge_entries(
        &key(1),
        &[(
            key(1),
            vec![
                role_definition(RoleId([0; 16]), "everyone", DEFAULT_EVERYONE, 0),
                role_definition(moderator, "moderator", MANAGE_MESSAGES, 1),
                at(
                    3,
                    GovernanceEntry::RoleAssignment {
                        target: key(2),
                        role_id: moderator,
                    },
                ),
            ],
        )],
    )
}

#[test]
fn a_member_without_manage_messages_is_refused() {
    let refused =
        may_delete_messages(&community(), &key(3), MODERATED_CHANNEL, &key(4)).unwrap_err();
    assert!(
        matches!(refused, ModerationRefusal::NotPermitted(_)),
        "{refused}"
    );
    assert_eq!(refused.code(), 403);
}

#[test]
fn the_creator_and_a_moderator_may_delete() {
    let state = community();
    assert!(may_delete_messages(&state, &key(1), MODERATED_CHANNEL, &key(4)).is_ok());
    assert!(may_delete_messages(&state, &key(2), MODERATED_CHANNEL, &key(4)).is_ok());
}

#[test]
fn a_banned_moderator_is_refused() {
    let mut state = community();
    state.bans.insert(key(2));
    assert!(may_delete_messages(&state, &key(2), MODERATED_CHANNEL, &key(4)).is_err());
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

#[test]
fn a_moderator_may_not_delete_a_higher_ranked_authors_post() {
    let state = community();
    let refused = may_delete_messages(&state, &key(2), MODERATED_CHANNEL, &key(1)).unwrap_err();
    assert_eq!(
        refused.code(),
        403,
        "M1: the creator outranks the moderator"
    );
    assert!(may_delete_messages(&state, &key(1), MODERATED_CHANNEL, &key(2)).is_ok());
}

/// `community()` with the channel and `key(2)`'s delete of a post by
/// `author`, as every reader merges it.
fn merged_delete(author: u8) -> GovernanceState {
    let moderator = RoleId([9; 16]);
    let channel = GovernanceEntry::ChannelCreated {
        channel_id: MODERATED_CHANNEL,
        name: "general".into(),
        channel_type: "text".into(),
        record_key: "VLD0:c".into(),
        category_id: None,
        position: 0,
        parent_voice_channel_id: None,
    };
    let delete = GovernanceEntry::AdminDelete {
        message_id: [7; 16],
        channel_id: MODERATED_CHANNEL,
        author: key(author),
        reason: None,
    };
    merge_entries(
        &key(1),
        &[
            (
                key(1),
                vec![
                    role_definition(RoleId([0; 16]), "everyone", DEFAULT_EVERYONE, 0),
                    role_definition(moderator, "moderator", MANAGE_MESSAGES, 1),
                    at(
                        3,
                        GovernanceEntry::RoleAssignment {
                            target: key(2),
                            role_id: moderator,
                        },
                    ),
                    at(4, channel),
                ],
            ),
            (key(2), vec![at(5, delete)]),
        ],
    )
}

const MESSAGE: &str = "msg_07070707070707070707070707070707";

#[test]
fn every_reader_applies_a_delete_of_a_lower_authors_post_to_that_author_only() {
    use super::super::channel_messages::removed_by;
    let state = merged_delete(3);
    let channel = MODERATED_CHANNEL.to_hex();
    assert_eq!(
        removed_by(&state, &channel, MESSAGE, &hex::encode(key(3).0)),
        Some(key(2))
    );
    assert_eq!(
        removed_by(&state, &channel, MESSAGE, &hex::encode(key(1).0)),
        None,
        "a post by another author is not this delete's"
    );
}

#[test]
fn no_reader_applies_a_delete_of_a_higher_authors_post() {
    use super::super::channel_messages::removed_by;
    let state = merged_delete(1);
    assert!(state.admin_deletes.is_empty(), "M1: the merge refuses it");
    let channel = MODERATED_CHANNEL.to_hex();
    assert_eq!(
        removed_by(&state, &channel, MESSAGE, &hex::encode(key(1).0)),
        None
    );
}
