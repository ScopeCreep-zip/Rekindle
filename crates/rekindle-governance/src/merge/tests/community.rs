//! community merge tests.

use super::*;
use crate::state::GENESIS_MEK_GENERATION;

fn bump(generation: u64, lamport: u64) -> Stamped {
    at(
        lamport,
        GovernanceEntry::MEKGenerationBump {
            generation,
            trigger_departed: PseudonymKey([0xBB; 32]),
            cascade_skipped: vec![],
        },
    )
}

/// Generations advance one at a time (plan D20) from the community key
/// minted at origin: each bump must name `current + 1`, so the genesis
/// generation itself is a replay. A jump — `u64::MAX` included — and a
/// replay are refused even from the creator.
#[test]
fn mek_generation_advances_by_one() {
    let creator = pseudo(1);
    let entries = vec![
        bump(GENESIS_MEK_GENERATION, 1),
        bump(2, 2),
        bump(3, 3),
        bump(6, 4),
        bump(3, 5),
        bump(u64::MAX, 6),
        bump(4, 7),
    ];
    let state = merge_entries(&creator, &[(creator.clone(), entries)]);
    assert_eq!(state.mek_generation, 4);
}

/// A merge with no bump is at the origin key's generation, which the
/// first rotation's bump (generation 2) names as `current + 1`.
#[test]
fn a_merge_starts_at_the_genesis_key_generation() {
    let creator = pseudo(1);
    let none = merge_entries(&creator, &[(creator.clone(), vec![meta_entry(1)])]);
    assert_eq!(none.mek_generation, GENESIS_MEK_GENERATION);
    let first = merge_entries(&creator, &[(creator.clone(), vec![bump(2, 1)])]);
    assert_eq!(first.mek_generation, 2);
}

/// A member without KICK, BAN or MANAGE_COMMUNITY cannot bump.
#[test]
fn mek_bump_needs_a_rotation_permission() {
    let creator = pseudo(1);
    let member = pseudo(2);
    let state = merge_entries(
        &creator,
        &[
            (creator.clone(), vec![meta_entry(1)]),
            (member, vec![bump(GENESIS_MEK_GENERATION + 1, 2)]),
        ],
    );
    assert_eq!(state.mek_generation, GENESIS_MEK_GENERATION);
}

fn meta_entry(lamport: u64) -> Stamped {
    at(
        lamport,
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        },
    )
}

#[test]
fn community_policy_lww_keeps_highest_lamport() {
    let creator = pseudo(5);
    let entries = vec![
        at(
            1,
            GovernanceEntry::CommunityPolicy {
                policy_text: Some("v1".into()),
                max_joins_per_interval: 10,
                join_interval_seconds: 300,
            },
        ),
        at(
            5,
            GovernanceEntry::CommunityPolicy {
                policy_text: Some("v2".into()),
                max_joins_per_interval: 30,
                join_interval_seconds: 900,
            },
        ),
        at(
            3,
            GovernanceEntry::CommunityPolicy {
                policy_text: Some("stale".into()),
                max_joins_per_interval: 1,
                join_interval_seconds: 60,
            },
        ),
    ];
    let state = merge_entries(&creator, &[(creator.clone(), entries)]);
    let policy = state.community_policy.expect("policy must be set");
    assert_eq!(policy.stamp.lamport, 5);
    assert_eq!(policy.policy_text.as_deref(), Some("v2"));
    assert_eq!(policy.max_joins_per_interval, 30);
    assert_eq!(policy.join_interval_seconds, 900);
}
