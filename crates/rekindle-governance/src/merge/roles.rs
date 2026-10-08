//! `merge` roles CRDT apply rules.

use super::{GovernanceEntry, GovernanceState, RoleState};

pub(super) fn apply_roles(entry: &GovernanceEntry, state: &mut GovernanceState) {
    match entry {
        GovernanceEntry::RoleDefinition {
            role_id,
            name,
            permissions,
            position,
            color,
            hoist,
            mentionable,
            self_assignable,
            exclusion_group,
            lamport,
        } => {
            let existing_lamport = state.roles.get(role_id).map_or(0, |r| r.lamport);
            if *lamport > existing_lamport {
                state.roles.insert(
                    *role_id,
                    RoleState {
                        name: name.clone(),
                        permissions: *permissions,
                        position: *position,
                        color: *color,
                        hoist: *hoist,
                        mentionable: *mentionable,
                        self_assignable: *self_assignable,
                        exclusion_group: exclusion_group.clone(),
                        lamport: *lamport,
                    },
                );
            }
        }

        // ── Role assignments: LWW-Flag per (target, role_id) ──
        // Architecture §19.4 — assigning a role with an exclusion_group
        // also removes the member's other assignments in that group
        // (with lower Lamport, since entries are processed in lamport
        // order). This makes "pronouns" / "region" / "team" pickers
        // mutually exclusive without per-group bookkeeping.
        GovernanceEntry::RoleAssignment {
            target, role_id, ..
        } => {
            if let Some(group) = state
                .roles
                .get(role_id)
                .and_then(|role| role.exclusion_group.clone())
            {
                if let Some(roles) = state.role_assignments.get_mut(target) {
                    roles.retain(|other_id| {
                        if other_id == role_id {
                            return true;
                        }
                        state
                            .roles
                            .get(other_id)
                            .and_then(|other| other.exclusion_group.as_ref())
                            != Some(&group)
                    });
                }
            }
            state
                .role_assignments
                .entry(target.clone())
                .or_default()
                .insert(*role_id);
        }

        GovernanceEntry::RoleUnassignment {
            target, role_id, ..
        } => {
            if let Some(roles) = state.role_assignments.get_mut(target) {
                roles.remove(role_id);
            }
        }

        // ── Bans: LWW-Flag per target pseudonym ──
        GovernanceEntry::RoleArchived { role_id, lamport } => {
            // Only archive if lamport > definition lamport
            if let Some(role) = state.roles.get(role_id) {
                if *lamport > role.lamport {
                    state.roles.remove(role_id);
                    // Also remove all assignments for this role
                    for assignments in state.role_assignments.values_mut() {
                        assignments.remove(role_id);
                    }
                }
            }
        }

        // ── Category updated: LWW per category_id ──
        _ => unreachable!("apply_roles: unexpected variant"),
    }
}
