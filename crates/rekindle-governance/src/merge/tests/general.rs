//! general merge tests.

use super::*;

#[test]
fn empty_merge() {
    let state = merge(&[]);
    assert!(state.channels.is_empty());
    assert!(state.roles.is_empty());
    assert!(state.creator.is_none());
}

#[test]
fn genesis_sets_creator() {
    let creator = pseudo(1);
    let entries = vec![GovernanceEntry::CommunityMeta {
        name: Some("Test".into()),
        description: None,
        icon_hash: None,
        banner_hash: None,
        lamport: 1,
    }];
    let state = merge(&[(creator.clone(), entries)]);
    assert_eq!(state.creator, Some(creator));
    assert_eq!(state.metadata.unwrap().name, "Test");
}

#[test]
fn multi_member_merge_converges() {
    // Two members write entries independently — merge should converge
    let member_a = pseudo(1);
    let member_b = pseudo(2);
    let rid = role_id(0);
    let ch = channel_id(1);

    let a_entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        GovernanceEntry::RoleDefinition {
            role_id: rid,
            name: "everyone".into(),
            permissions: rekindle_types::permissions::ALL,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 2,
        },
        GovernanceEntry::ChannelCreated {
            channel_id: ch,
            name: "general".into(),
            channel_type: "text".into(),
            record_key: "VLD0:abc".into(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
            lamport: 3,
        },
    ];

    let b_entries = vec![GovernanceEntry::MEKGenerationBump {
        generation: 2,
        trigger_departed: PseudonymKey([0xEE; 32]),
        cascade_skipped: vec![],
        lamport: 4,
    }];

    // Order A, B
    let state1 = merge(&[
        (member_a.clone(), a_entries.clone()),
        (member_b.clone(), b_entries.clone()),
    ]);

    // Order B, A — should produce same result
    let state2 = merge(&[(member_b, b_entries), (member_a, a_entries)]);

    assert_eq!(
        state1, state2,
        "CRDT convergence: different subkey order must produce same state"
    );
}

fn meta(lamport: u64) -> GovernanceEntry {
    GovernanceEntry::CommunityMeta {
        name: Some("C".into()),
        description: None,
        icon_hash: None,
        banner_hash: None,
        lamport,
    }
}

#[test]
fn accepted_clock_is_highest_accepted_plus_one() {
    let (_, clock) = merge_with_accepted(&[(pseudo(1), vec![meta(1), meta(2), meta(7)])]);
    assert_eq!(clock, 8);
}

/// An entry the reader rejects (no permission) never moves the clock,
/// however far ahead it claims to be.
#[test]
fn rejected_entry_does_not_move_the_clock() {
    let creator = pseudo(1);
    let outsider = pseudo(9);
    let (_, clock) =
        merge_with_accepted(&[(creator, vec![meta(1)]), (outsider, vec![meta(u64::MAX)])]);
    assert_eq!(clock, 2);
}

/// An accepted entry (the creator's own) that jumps to the ceiling is
/// applied, but advances the clock by at most the drift bound.
#[test]
fn accepted_far_future_entry_is_bounded() {
    use rekindle_types::lamport::MAX_LAMPORT_DRIFT;
    let (state, clock) = merge_with_accepted(&[(pseudo(1), vec![meta(1), meta(u64::MAX)])]);
    assert_eq!(clock, 2 + MAX_LAMPORT_DRIFT + 1);
    // The entry itself is accepted and orders by its own timestamp.
    assert_eq!(state.metadata.map(|m| m.lamport), Some(u64::MAX));
}
