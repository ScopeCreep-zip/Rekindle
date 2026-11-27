use super::*;
use crate::state::{GovernanceState, RoleState, TimeoutState};
use rekindle_types::permissions::{BYPASS_SLOWMODE, DEFAULT_EVERYONE};
use std::collections::{HashMap, HashSet};

fn pseudo(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

fn rid(b: u8) -> RoleId {
    RoleId([b; 16])
}

fn base_state() -> GovernanceState {
    let everyone_role = RoleState {
        name: "everyone".into(),
        permissions: DEFAULT_EVERYONE,
        position: 0,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
        stamp: crate::fixture::stamp(1),
    };
    let mut roles = HashMap::new();
    roles.insert(rid(0), everyone_role);

    GovernanceState {
        creators: std::collections::BTreeSet::from([pseudo(1)]),
        roles,
        ..Default::default()
    }
}

#[test]
fn creator_has_all() {
    let state = base_state();
    let perms = compute_permissions(&pseudo(1), None, &state, 0);
    assert_eq!(perms, ALL);
}

#[test]
fn everyone_gets_defaults() {
    let state = base_state();
    let perms = compute_permissions(&pseudo(99), None, &state, 0);
    assert!(perms & SEND_MESSAGES != 0);
    assert!(perms & VIEW_CHANNELS != 0);
    assert!(perms & READ_HISTORY != 0);
    assert_eq!(perms & BAN_MEMBERS, 0);
    assert_eq!(perms & ADMINISTRATOR, 0);
}

#[test]
fn everyone_can_connect_to_voice() {
    // Regression: a non-creator member must receive CONNECT (and
    // USE_VOICE_ACTIVITY) from DEFAULT_EVERYONE so the canonical
    // voice-join gate (require_permission → has_all_capabilities) passes.
    // A permission bit-layout mismatch previously denied every non-owner
    // member voice access.
    let state = base_state();
    let perms = compute_permissions(&pseudo(99), None, &state, 0);
    assert!(has_all_capabilities(perms, CONNECT), "member lacks CONNECT");
    assert!(has_all_capabilities(perms, USE_VOICE_ACTIVITY));
}

#[test]
fn administrator_bypasses_all() {
    let mut state = base_state();
    let admin_role = RoleState {
        name: "admin".into(),
        permissions: ADMINISTRATOR,
        position: 1,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
        stamp: crate::fixture::stamp(2),
    };
    state.roles.insert(rid(1), admin_role);
    let mut assignments = HashSet::new();
    assignments.insert(rid(1));
    state.role_assignments.insert(pseudo(5), assignments);

    let perms = compute_permissions(&pseudo(5), None, &state, 0);
    assert_eq!(perms, ALL);
}

#[test]
fn timeout_strips_permissions() {
    let mut state = base_state();
    state.timeouts.insert(
        pseudo(10),
        TimeoutState {
            duration_seconds: 3600,
            started_at: 1000,
            reason: None,
            stamp: crate::fixture::stamp(5),
        },
    );

    // Not expired (now=1500, expires at 4600)
    let perms = compute_permissions(&pseudo(10), None, &state, 1500);
    assert!(perms & VIEW_CHANNELS != 0);
    assert!(perms & READ_HISTORY != 0);
    assert_eq!(perms & SEND_MESSAGES, 0);
    assert_eq!(perms & BAN_MEMBERS, 0);

    // Expired (now=5000)
    let perms = compute_permissions(&pseudo(10), None, &state, 5000);
    assert!(
        perms & SEND_MESSAGES != 0,
        "timeout expired, perms restored"
    );
}

#[test]
fn no_view_channels_strips_everything() {
    let mut state = base_state();
    // Override @everyone to have no VIEW_CHANNELS
    state.roles.insert(
        rid(0),
        RoleState {
            name: "everyone".into(),
            permissions: SEND_MESSAGES, // has send but NOT view
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            stamp: crate::fixture::stamp(1),
        },
    );

    let perms = compute_permissions(&pseudo(99), None, &state, 0);
    assert_eq!(perms, 0, "no VIEW_CHANNELS → everything stripped");
}

#[test]
fn implicit_deps_no_send_strips_mention() {
    let mut state = base_state();
    state.roles.insert(
        rid(0),
        RoleState {
            name: "everyone".into(),
            permissions: VIEW_CHANNELS | READ_HISTORY | MENTION_EVERYONE, // mention but no send
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            stamp: crate::fixture::stamp(1),
        },
    );

    let perms = compute_permissions(&pseudo(99), None, &state, 0);
    assert_eq!(perms & MENTION_EVERYONE, 0, "no SEND → MENTION stripped");
}

#[test]
fn has_capability_single_bit() {
    assert!(has_capability(
        BYPASS_SLOWMODE | SEND_MESSAGES,
        BYPASS_SLOWMODE
    ));
    assert!(!has_capability(SEND_MESSAGES, BYPASS_SLOWMODE));
    assert!(!has_capability(0, MANAGE_COMMUNITY));
}

#[test]
fn has_all_capabilities_requires_every_bit() {
    let mask = SEND_MESSAGES | EMBED_LINKS;
    assert!(has_all_capabilities(
        SEND_MESSAGES | EMBED_LINKS | VIEW_CHANNELS,
        mask
    ));
    assert!(!has_all_capabilities(SEND_MESSAGES, mask));
    assert!(has_all_capabilities(ALL, mask));
    // Empty mask is vacuously satisfied.
    assert!(has_all_capabilities(0, 0));
}
