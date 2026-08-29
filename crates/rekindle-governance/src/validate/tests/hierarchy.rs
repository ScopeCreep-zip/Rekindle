//! Role-hierarchy (M10.1) and OWNER-protection (M10.2) tests, split out
//! from [`super`] for line-count. Shares fixtures (`pseudo`, `rid`,
//! `state_with_creator_and_roles`) with the parent module via
//! `use super::*`.

use super::*;

// ──────────────────────────────────────────────────────────────────
// M10.1 / M10.2 — role hierarchy + OWNER protection (arch §9.3 line 1946)
// ──────────────────────────────────────────────────────────────────

/// Build a state with a creator, three roles (everyone=0, mod=2, admin=5),
/// and three test members:
/// - pseudo(1): creator (no role assignments needed; creator-bypass kicks in)
/// - pseudo(5): admin (position 5)
/// - pseudo(6): mod (position 2)
/// - pseudo(7): another admin (position 5)
/// - pseudo(99): regular member (no roles → position 0)
fn state_with_three_tier_hierarchy() -> GovernanceState {
    let mut roles = HashMap::new();
    roles.insert(
        rid(0),
        RoleState {
            name: "everyone".into(),
            permissions: VIEW_CHANNELS | SEND_MESSAGES | READ_HISTORY,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 1,
        },
    );
    roles.insert(
        rid(2),
        RoleState {
            name: "mod".into(),
            permissions: BAN_MEMBERS | TIMEOUT_MEMBERS | MANAGE_ROLES,
            position: 2,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 2,
        },
    );
    roles.insert(
        rid(5),
        RoleState {
            name: "admin".into(),
            permissions: ADMINISTRATOR,
            position: 5,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 3,
        },
    );

    let mut assignments = HashMap::new();
    let mut admin_5 = HashSet::new();
    admin_5.insert(rid(5));
    assignments.insert(pseudo(5), admin_5);

    let mut admin_7 = HashSet::new();
    admin_7.insert(rid(5));
    assignments.insert(pseudo(7), admin_7);

    let mut mod_role = HashSet::new();
    mod_role.insert(rid(2));
    assignments.insert(pseudo(6), mod_role);

    GovernanceState {
        creator: Some(pseudo(1)),
        roles,
        role_assignments: assignments,
        ..Default::default()
    }
}

#[test]
fn ban_admin_blocked_by_equal_rank_admin() {
    // M10.1 — admin (pseudo 5, pos 5) tries to ban another admin (pseudo 7, pos 5).
    // Equal rank → reject. This is the exact rogue-admin scenario.
    let state = state_with_three_tier_hierarchy();
    let entry = GovernanceEntry::BanEntry {
        target: pseudo(7),
        reason: None,
        lamport: 100,
    };
    assert!(
        !validate_write(&pseudo(5), &entry, &state),
        "equal-rank admin must not be able to ban another admin"
    );
}

#[test]
fn ban_mod_succeeds_from_admin() {
    // M10.1 — admin (pos 5) banning a mod (pos 2): strictly higher rank → accept.
    let state = state_with_three_tier_hierarchy();
    let entry = GovernanceEntry::BanEntry {
        target: pseudo(6),
        reason: None,
        lamport: 100,
    };
    assert!(validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn ban_admin_blocked_from_mod() {
    // M10.1 — mod (pos 2) attempting to ban admin (pos 5): strictly lower rank → reject.
    // (Even though mod has BAN_MEMBERS perm via the role.)
    let state = state_with_three_tier_hierarchy();
    let entry = GovernanceEntry::BanEntry {
        target: pseudo(5),
        reason: None,
        lamport: 100,
    };
    assert!(!validate_write(&pseudo(6), &entry, &state));
}

#[test]
fn ban_creator_blocked_from_admin() {
    // M10.2 — admin trying to ban the creator must be rejected even
    // though admin has ADMINISTRATOR perm. Creator is OWNER-tier.
    let state = state_with_three_tier_hierarchy();
    let entry = GovernanceEntry::BanEntry {
        target: pseudo(1),
        reason: None,
        lamport: 100,
    };
    assert!(!validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn timeout_admin_blocked_by_equal_rank_admin() {
    // M10.1 — same hierarchy enforcement applies to timeouts.
    let state = state_with_three_tier_hierarchy();
    let entry = GovernanceEntry::TimeoutEntry {
        target: pseudo(7),
        duration_seconds: 3600,
        reason: None,
        started_at: 0,
        lamport: 100,
    };
    assert!(!validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn timeout_creator_blocked() {
    // M10.2 — creator is immune to timeouts.
    let state = state_with_three_tier_hierarchy();
    let entry = GovernanceEntry::TimeoutEntry {
        target: pseudo(1),
        duration_seconds: 3600,
        reason: None,
        started_at: 0,
        lamport: 100,
    };
    assert!(!validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn unassign_admin_role_blocked_by_equal_rank_admin() {
    // M10.1 — admin (pos 5) attempting to revoke another admin's
    // (pos 5) admin role: target_max == writer_max → reject.
    let state = state_with_three_tier_hierarchy();
    let entry = GovernanceEntry::RoleUnassignment {
        target: pseudo(7),
        role_id: rid(5),
        lamport: 100,
    };
    assert!(!validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn unassign_creator_role_blocked_from_anyone_else() {
    // M10.2 — even if creator had a role assignment, no other peer
    // can unassign roles from them. (Synthesized: give creator the
    // admin role and then try to revoke from a peer that climbed
    // somehow to the same rank.)
    let mut state = state_with_three_tier_hierarchy();
    let mut creator_roles = HashSet::new();
    creator_roles.insert(rid(5));
    state.role_assignments.insert(pseudo(1), creator_roles);

    let entry = GovernanceEntry::RoleUnassignment {
        target: pseudo(1),
        role_id: rid(5),
        lamport: 100,
    };
    // Even though pseudo(7) is also rank 5, they cannot touch creator.
    assert!(!validate_write(&pseudo(7), &entry, &state));
}

#[test]
fn unassign_self_always_allowed() {
    // M10.1 — a member can step down from their own role regardless
    // of rank. (Admin gives up admin → accepted.)
    let state = state_with_three_tier_hierarchy();
    let entry = GovernanceEntry::RoleUnassignment {
        target: pseudo(5),
        role_id: rid(5),
        lamport: 100,
    };
    assert!(validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn unassign_self_creator_allowed() {
    // M10.2 — creator unassigning their own role is allowed (the
    // OWNER-block only blocks OTHER peers).
    let mut state = state_with_three_tier_hierarchy();
    let mut creator_roles = HashSet::new();
    creator_roles.insert(rid(5));
    state.role_assignments.insert(pseudo(1), creator_roles);

    let entry = GovernanceEntry::RoleUnassignment {
        target: pseudo(1),
        role_id: rid(5),
        lamport: 100,
    };
    assert!(validate_write(&pseudo(1), &entry, &state));
}

#[test]
fn role_assignment_blocked_at_or_above_writer_rank() {
    // M10.1 — mod (pos 2) trying to grant the admin role (pos 5) to
    // pseudo(99): role_position >= writer_max → reject.
    let state = state_with_three_tier_hierarchy();
    let entry = GovernanceEntry::RoleAssignment {
        target: pseudo(99),
        role_id: rid(5),
        lamport: 100,
    };
    assert!(!validate_write(&pseudo(6), &entry, &state));
}

#[test]
fn role_assignment_below_rank_succeeds() {
    // M10.1 — admin (pos 5) granting mod role (pos 2) to a member:
    // role_position < writer_max → accept.
    let state = state_with_three_tier_hierarchy();
    let entry = GovernanceEntry::RoleAssignment {
        target: pseudo(99),
        role_id: rid(2),
        lamport: 100,
    };
    assert!(validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn role_definition_blocked_at_or_above_writer_rank() {
    // M10.1 — admin (pos 5) trying to mint a new role at position 5
    // (which they could then grant themselves to climb): reject.
    let state = state_with_three_tier_hierarchy();
    let entry = GovernanceEntry::RoleDefinition {
        role_id: rid(99),
        name: "shadow-admin".into(),
        permissions: ADMINISTRATOR,
        position: 5,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
        lamport: 100,
    };
    assert!(!validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn role_definition_below_rank_succeeds() {
    // M10.1 — admin (pos 5) mints a role at position 3 (below them):
    // accept.
    let state = state_with_three_tier_hierarchy();
    let entry = GovernanceEntry::RoleDefinition {
        role_id: rid(3),
        name: "trial-mod".into(),
        permissions: TIMEOUT_MEMBERS,
        position: 3,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
        lamport: 100,
    };
    assert!(validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn creator_can_act_at_any_rank() {
    // M10.2 — creator-bypass (validate.rs:61) means creator can ban
    // any peer, even another admin at equal rank.
    let state = state_with_three_tier_hierarchy();
    let entry = GovernanceEntry::BanEntry {
        target: pseudo(5),
        reason: None,
        lamport: 100,
    };
    assert!(validate_write(&pseudo(1), &entry, &state));
}

#[test]
fn welcome_screen_rejects_too_many_channels() {
    use rekindle_types::governance::WelcomeChannel;
    let state = state_with_creator_and_roles();
    let mut channels = Vec::new();
    for i in 0..6_u8 {
        channels.push(WelcomeChannel {
            channel_id: rekindle_types::id::ChannelId([i; 16]),
            description: "d".into(),
            emoji: None,
        });
    }
    let entry = GovernanceEntry::WelcomeScreen {
        description: "d".into(),
        channels,
        lamport: 11,
    };
    assert!(!validate_write(&pseudo(5), &entry, &state));
}
