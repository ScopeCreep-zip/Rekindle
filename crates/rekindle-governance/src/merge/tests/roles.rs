//! roles merge tests.

use super::*;

#[test]
fn role_lww_highest_lamport_wins() {
    let creator = pseudo(1);
    let rid = role_id(1);
    let entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        GovernanceEntry::RoleDefinition {
            role_id: rid,
            name: "old_name".into(),
            permissions: 0,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 2,
        },
        GovernanceEntry::RoleDefinition {
            role_id: rid,
            name: "new_name".into(),
            permissions: 0xFF,
            position: 1,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 3,
        },
    ];
    let state = merge(&[(creator, entries)]);
    let role = state.roles.get(&rid).unwrap();
    assert_eq!(role.name, "new_name");
    assert_eq!(role.permissions, 0xFF);
}

#[test]
fn role_assignment_and_unassignment() {
    let creator = pseudo(1);
    let member = pseudo(2);
    let rid = role_id(5);

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
        GovernanceEntry::RoleAssignment {
            target: member.clone(),
            role_id: rid,
            lamport: 3,
        },
        GovernanceEntry::RoleUnassignment {
            target: member.clone(),
            role_id: rid,
            lamport: 4,
        },
    ];
    let state = merge(&[(creator, entries)]);
    let roles = state.role_assignments.get(&member);
    assert!(
        roles.is_none() || roles.unwrap().is_empty(),
        "role should be unassigned"
    );
}

#[test]
fn exclusion_group_unassigns_prior_role_in_same_group() {
    // Architecture §19.4 — pronouns: assigning "he/him" must remove
    // "she/her" automatically because both share the "pronouns" group.
    let creator = pseudo(7);
    let member = pseudo(8);
    let role_he = role_id(11);
    let role_she = role_id(12);
    let role_unrelated = role_id(13);
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
        GovernanceEntry::RoleDefinition {
            role_id: role_he,
            name: "he/him".into(),
            permissions: 0,
            position: 1,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: true,
            exclusion_group: Some("pronouns".into()),
            lamport: 3,
        },
        GovernanceEntry::RoleDefinition {
            role_id: role_she,
            name: "she/her".into(),
            permissions: 0,
            position: 1,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: true,
            exclusion_group: Some("pronouns".into()),
            lamport: 4,
        },
        GovernanceEntry::RoleDefinition {
            role_id: role_unrelated,
            name: "early-bird".into(),
            permissions: 0,
            position: 1,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: true,
            exclusion_group: None,
            lamport: 5,
        },
        GovernanceEntry::RoleAssignment {
            target: member.clone(),
            role_id: role_she,
            lamport: 6,
        },
        GovernanceEntry::RoleAssignment {
            target: member.clone(),
            role_id: role_unrelated,
            lamport: 7,
        },
        // Switching pronouns: he/him must replace she/her, but
        // early-bird (no exclusion group) stays assigned.
        GovernanceEntry::RoleAssignment {
            target: member.clone(),
            role_id: role_he,
            lamport: 8,
        },
    ];
    let state = merge(&[(creator, entries)]);
    let assignments = state
        .role_assignments
        .get(&member)
        .expect("member must have assignments");
    assert!(assignments.contains(&role_he), "he/him must be active");
    assert!(
        !assignments.contains(&role_she),
        "she/her must have been auto-unassigned"
    );
    assert!(
        assignments.contains(&role_unrelated),
        "non-grouped roles are unaffected"
    );
}
