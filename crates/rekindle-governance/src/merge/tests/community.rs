//! community merge tests.

use super::*;

#[test]
fn mek_max_register() {
    let creator = pseudo(1);
    let entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        GovernanceEntry::MEKGenerationBump {
            generation: 3,
            trigger_departed: PseudonymKey([0xBB; 32]),
            cascade_skipped: vec![],
            lamport: 2,
        },
        GovernanceEntry::MEKGenerationBump {
            generation: 1,
            trigger_departed: PseudonymKey([0xCC; 32]),
            cascade_skipped: vec![],
            lamport: 3,
        },
        GovernanceEntry::MEKGenerationBump {
            generation: 5,
            trigger_departed: PseudonymKey([0xDD; 32]),
            cascade_skipped: vec![],
            lamport: 4,
        },
    ];
    let state = merge(&[(creator, entries)]);
    assert_eq!(state.mek_generation, 5);
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
