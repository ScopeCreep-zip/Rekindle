//! community merge tests.

use super::*;

fn bump(generation: u64, lamport: u64) -> GovernanceEntry {
    GovernanceEntry::MEKGenerationBump {
        generation,
        trigger_departed: PseudonymKey([0xBB; 32]),
        cascade_skipped: vec![],
        lamport,
    }
}

/// Generations advance one at a time (plan D20): each bump must name
/// `current + 1`. A jump — `u64::MAX` included — and a replay are refused
/// even from the creator.
#[test]
fn mek_generation_advances_by_one() {
    let creator = pseudo(1);
    let entries = vec![
        bump(1, 1),
        bump(2, 2),
        bump(5, 3),
        bump(2, 4),
        bump(u64::MAX, 5),
        bump(3, 6),
    ];
    let state = merge(&[(creator, entries)]);
    assert_eq!(state.mek_generation, 3);
}

/// A member without KICK, BAN or MANAGE_COMMUNITY cannot bump.
#[test]
fn mek_bump_needs_a_rotation_permission() {
    let creator = pseudo(1);
    let member = pseudo(2);
    let state = merge(&[(creator, vec![meta_entry(1)]), (member, vec![bump(1, 2)])]);
    assert_eq!(state.mek_generation, 0);
}

fn meta_entry(lamport: u64) -> GovernanceEntry {
    GovernanceEntry::CommunityMeta {
        name: Some("C".into()),
        description: None,
        icon_hash: None,
        banner_hash: None,
        lamport,
    }
}

#[test]
fn community_policy_lww_keeps_highest_lamport() {
    let creator = pseudo(5);
    let entries = vec![
        GovernanceEntry::CommunityPolicy {
            policy_text: Some("v1".into()),
            max_joins_per_interval: 10,
            join_interval_seconds: 300,
            lamport: 1,
        },
        GovernanceEntry::CommunityPolicy {
            policy_text: Some("v2".into()),
            max_joins_per_interval: 30,
            join_interval_seconds: 900,
            lamport: 5,
        },
        GovernanceEntry::CommunityPolicy {
            policy_text: Some("stale".into()),
            max_joins_per_interval: 1,
            join_interval_seconds: 60,
            lamport: 3,
        },
    ];
    let state = merge(&[(creator, entries)]);
    let policy = state.community_policy.expect("policy must be set");
    assert_eq!(policy.lamport, 5);
    assert_eq!(policy.policy_text.as_deref(), Some("v2"));
    assert_eq!(policy.max_joins_per_interval, 30);
    assert_eq!(policy.join_interval_seconds, 900);
}
