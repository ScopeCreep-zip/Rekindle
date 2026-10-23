//! moderation merge tests.

use super::*;

#[test]
fn ban_unban_lww() {
    let creator = pseudo(1);
    let target = pseudo(2);
    let entries = vec![
        at(
            1,
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
            },
        ),
        at(
            2,
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
            },
        ),
        at(
            3,
            GovernanceEntry::BanEntry {
                target: target.clone(),
                reason: Some("spam".into()),
            },
        ),
        at(
            4,
            GovernanceEntry::UnbanEntry {
                target: target.clone(),
            },
        ),
    ];
    let state = merge_entries(&creator, &[(creator.clone(), entries)]);
    assert!(!state.bans.contains(&target), "unban should reverse ban");
}

#[test]
fn timeout_lww() {
    let creator = pseudo(1);
    let target = pseudo(2);
    let entries = vec![
        at(
            1,
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
            },
        ),
        at(
            2,
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
            },
        ),
        at(
            3,
            GovernanceEntry::TimeoutEntry {
                target: target.clone(),
                duration_seconds: 3600,
                reason: None,
                started_at: 1000,
            },
        ),
    ];
    let state = merge_entries(&creator, &[(creator.clone(), entries)]);
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
        at(
            1,
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
            },
        ),
        at(
            2,
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
            },
        ),
        at(
            3,
            GovernanceEntry::TimeoutEntry {
                target: target.clone(),
                duration_seconds: 3600,
                reason: None,
                started_at: 1000,
            },
        ),
        at(
            4,
            GovernanceEntry::RemoveTimeoutEntry {
                target: target.clone(),
            },
        ),
    ];
    let state = merge_entries(&creator, &[(creator.clone(), entries)]);
    assert!(!state.timeouts.contains_key(&target));
}
