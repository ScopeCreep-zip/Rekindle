//! A projected merge announces each changed table exactly once, from the
//! state diff (plan E3.2 S9): a channel create, edit and delete, a category
//! edit, a role definition and a member's role assignment each publish one
//! event; a merge that changes nothing publishes none.

use std::collections::HashSet;

use rekindle_governance::fixture::stamp;
use rekindle_governance::state::{CategoryState, ChannelState, GovernanceState, RoleState};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_types::id::{CategoryId, ChannelId, PseudonymKey, RoleId};
use rekindle_types::subscription_events::{
    GovernanceEvent, MembershipEvent, StreamFrame, SubscriptionEvent,
};

use super::TableChanges;
use crate::host::test_fixtures::{create_ann, offline_host};

const C: &str = "c1";

fn channel(name: &str) -> ChannelState {
    ChannelState {
        name: name.into(),
        channel_type: "text".into(),
        category_id: None,
        position: 0,
        topic: None,
        forum_tags: None,
        slowmode_seconds: None,
        nsfw: None,
        parent_voice_channel_id: None,
        created: stamp(1),
    }
}

fn role(name: &str) -> RoleState {
    RoleState {
        name: name.into(),
        permissions: 0,
        position: 1,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
        stamp: stamp(1),
    }
}

/// The table events counted per kind: channels, roles, and each published
/// `MembershipEvent::RolesChanged` payload (observed events, not a role cache).
#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    channels: usize,
    roles: usize,
    role_set_events: Vec<(String, Vec<String>)>,
}

/// Project `prev` then merge `next` over it through the runtime's own
/// install path, and count the table events the merge published.
async fn merge_counts(prev: GovernanceState, next: GovernanceState) -> Counts {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    host.ctx.communities.set_governance_state(C, prev);
    let mut events = host.ctx.journal.subscribe(None);
    host.governance().set_governance_state(C, next);
    let mut counts = Counts::default();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(200), events.next()).await
    {
        let StreamFrame::Event(s) = frame else {
            continue;
        };
        match s.event {
            SubscriptionEvent::Governance(GovernanceEvent::ChannelsChanged { .. }) => {
                counts.channels += 1;
            }
            SubscriptionEvent::Governance(GovernanceEvent::RolesChanged { .. }) => {
                counts.roles += 1;
            }
            SubscriptionEvent::Membership(MembershipEvent::RolesChanged {
                pseudonym,
                role_ids,
                ..
            }) => counts.role_set_events.push((pseudonym, role_ids)),
            _ => {}
        }
    }
    counts
}

fn with_channel(name: &str) -> GovernanceState {
    let mut s = GovernanceState::default();
    s.channels.insert(ChannelId([1; 16]), channel(name));
    s
}

fn one_channels_event() -> Counts {
    Counts {
        channels: 1,
        ..Counts::default()
    }
}

#[tokio::test]
async fn a_channel_create_edit_and_delete_each_publish_one_channels_changed() {
    let created = merge_counts(GovernanceState::default(), with_channel("general")).await;
    assert_eq!(created, one_channels_event());
    let edited = merge_counts(with_channel("general"), with_channel("lobby")).await;
    assert_eq!(edited, one_channels_event());
    let deleted = merge_counts(with_channel("lobby"), GovernanceState::default()).await;
    assert_eq!(deleted, one_channels_event());
}

#[tokio::test]
async fn a_category_edit_publishes_one_channels_changed() {
    let category = |name: &str| {
        let mut s = GovernanceState::default();
        s.categories.insert(
            CategoryId([2; 16]),
            CategoryState {
                name: name.into(),
                position: 0,
                created: stamp(1),
            },
        );
        s
    };
    assert_eq!(
        merge_counts(category("Text"), category("Voice")).await,
        one_channels_event()
    );
}

#[tokio::test]
async fn a_role_definition_publishes_one_roles_changed() {
    let mut next = GovernanceState::default();
    next.roles.insert(RoleId([7; 16]), role("Mods"));
    assert_eq!(
        merge_counts(GovernanceState::default(), next).await,
        Counts {
            roles: 1,
            ..Counts::default()
        }
    );
}

#[tokio::test]
async fn a_role_assignment_publishes_the_members_new_role_set_once() {
    let member = PseudonymKey([9; 32]);
    let mut prev = GovernanceState::default();
    prev.roles.insert(RoleId([7; 16]), role("Mods"));
    let mut next = prev.clone();
    next.role_assignments
        .insert(member.clone(), HashSet::from([RoleId([7; 16])]));
    assert_eq!(
        merge_counts(prev, next).await,
        Counts {
            role_set_events: vec![(member.to_hex(), vec![RoleId([7; 16]).to_hex()])],
            ..Counts::default()
        }
    );
}

#[tokio::test]
async fn a_merge_that_changes_no_table_publishes_nothing() {
    let mut state = with_channel("general");
    state.roles.insert(RoleId([7; 16]), role("Mods"));
    assert_eq!(merge_counts(state.clone(), state).await, Counts::default());
}

#[test]
fn table_changes_read_only_the_diffs_role_channel_and_category_lists() {
    let mut prev = with_channel("general");
    prev.bans.insert(PseudonymKey([3; 32]));
    let next = with_channel("general");
    assert_eq!(TableChanges::of(&prev.diff(&next)), TableChanges::default());
}
