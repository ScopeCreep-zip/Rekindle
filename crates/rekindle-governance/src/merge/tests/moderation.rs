//! moderation merge tests.

use super::*;

#[test]
fn ban_unban_lww() {
    let creator = pseudo(1);
    let target = pseudo(2);
    let entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        GovernanceEntry::RoleDefinition {
            role_id: role_id(0),
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
        GovernanceEntry::BanEntry {
            target: target.clone(),
            reason: Some("spam".into()),
            lamport: 3,
        },
        GovernanceEntry::UnbanEntry {
            target: target.clone(),
            lamport: 4,
        },
    ];
    let state = merge(&[(creator, entries)]);
    assert!(!state.bans.contains(&target), "unban should reverse ban");
}

#[test]
fn timeout_lww() {
    let creator = pseudo(1);
    let target = pseudo(2);
    let entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        GovernanceEntry::RoleDefinition {
            role_id: role_id(0),
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
        GovernanceEntry::TimeoutEntry {
            target: target.clone(),
            duration_seconds: 3600,
            reason: None,
            started_at: 1000,
            lamport: 3,
        },
    ];
    let state = merge(&[(creator, entries)]);
    let timeout = state.timeouts.get(&target).unwrap();
    assert_eq!(timeout.duration_seconds, 3600);
    assert!(!timeout.is_expired(1500));
    assert!(timeout.is_expired(5000));
}

#[test]
fn remove_timeout_clears_active_timeout() {
    let creator = pseudo(1);
    let target = pseudo(2);
    let entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        GovernanceEntry::RoleDefinition {
            role_id: role_id(0),
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
        GovernanceEntry::TimeoutEntry {
            target: target.clone(),
            duration_seconds: 3600,
            reason: None,
            started_at: 1000,
            lamport: 3,
        },
        GovernanceEntry::RemoveTimeoutEntry {
            target: target.clone(),
            lamport: 4,
        },
    ];
    let state = merge(&[(creator, entries)]);
    assert!(!state.timeouts.contains_key(&target));
}
