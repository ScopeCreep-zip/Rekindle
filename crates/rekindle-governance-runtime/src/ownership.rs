//! Handing administrative control to another member.
//!
//! ## Why this is not "transfer ownership"
//!
//! Under flat governance there is no owner to move.
//! [`GovernanceState::creator`](rekindle_governance::state::GovernanceState)
//! is set from the genesis entries at Veilid seq 1 and
//! `compute_permissions` returns `ALL` for it unconditionally. Nothing
//! in the CRDT can change it, deliberately — the same reasoning that
//! fixes `AdmissionPolicy` at genesis: a mutable privileged flag is
//! flippable by anyone who holds `MANAGE_COMMUNITY`.
//!
//! Matrix reached the identical conclusion independently in room
//! version 12 (MSC4289, 2025): creators hold an immutable power level,
//! "cannot be demoted", and the creator set is fixed by the room's
//! earliest event. Their stated reasons are ours — it blocks privilege
//! escalation through backdated events, and it stops an admin locking
//! themselves out of their own room by self-demoting. A creator who can
//! always fix the room is a feature.
//!
//! So the operation a caller actually wants is: **give this member
//! administrative power**, which under v2.0 means assigning them a role
//! that carries `ADMINISTRATOR`. That is an ordinary `RoleAssignment`
//! entry every peer merges and validates.
//!
//! ## What it replaced
//!
//! The daemon's `handle_transfer_ownership` rewrote `owner_pseudonym`
//! and `operator_pseudonyms` in the v1.0 governance-manifest metadata
//! subkey. Nothing reads those under v2.0 — permissions come from the
//! merged CRDT — so the call reported success and granted the new owner
//! precisely nothing.

use rekindle_governance::state::GovernanceState;
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::permissions::ADMINISTRATOR;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::roles;

/// What happened, so the caller can tell the user the truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantOutcome {
    /// The role the target now holds.
    pub role_id: u32,
    /// Whether we also gave up our own copy of that role.
    pub relinquished: bool,
    /// Set when we are the community's creator.
    ///
    /// The creator's `ALL` is genesis-fixed and cannot be given up, so a
    /// "transfer" by the creator is a grant and nothing else. Reported
    /// rather than silently ignored — a caller who thinks they have
    /// stepped down and has not is exactly the confusion Matrix's
    /// immutable-creator rule exists to prevent.
    pub still_creator: bool,
}

/// The lowest-numbered role carrying `ADMINISTRATOR`.
///
/// Deterministic by role id so two admins granting concurrently pick
/// the same role and the assignments merge rather than diverge.
fn administrator_role(state: &GovernanceState) -> Option<RoleId> {
    state
        .roles
        .iter()
        .filter(|(_, role)| role.permissions & ADMINISTRATOR != 0)
        .map(|(id, _)| *id)
        .min_by_key(|id| id.to_legacy_u32())
}

/// Grant administrative control to `target`, and step down if we can.
///
/// Refuses when no role carries `ADMINISTRATOR`: creating one silently
/// would be inventing a permission structure the community never chose,
/// and the caller can make one with the ordinary role commands.
pub async fn grant_administration<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    target_pseudonym_hex: &str,
) -> Result<GrantOutcome, GovernanceRuntimeError> {
    let state = deps
        .governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_string()))?;

    deps.require_permission(community_id, ADMINISTRATOR)?;

    let role_id = administrator_role(&state).ok_or_else(|| {
        GovernanceRuntimeError::Adapter(
            "no role in this community carries ADMINISTRATOR — create one first".to_string(),
        )
    })?;
    let role_legacy = role_id.to_legacy_u32();

    roles::assign_role(deps, community_id, target_pseudonym_hex, role_legacy).await?;

    // Step down only if we hold the role by assignment. A creator's
    // power is not held that way and cannot be dropped.
    let me = deps
        .community_membership(community_id)
        .and_then(|m| m.my_pseudonym_hex)
        .map(|hex| PseudonymKey::from_hex_lossy(&hex));
    let still_creator = me
        .as_ref()
        .is_some_and(|me| state.creator.as_ref() == Some(me));
    let holds_role = me
        .as_ref()
        .and_then(|me| state.role_assignments.get(me))
        .is_some_and(|roles| roles.contains(&role_id));

    let relinquished = if holds_role && !still_creator {
        let my_hex = deps
            .community_membership(community_id)
            .and_then(|m| m.my_pseudonym_hex)
            .unwrap_or_default();
        roles::unassign_role(deps, community_id, &my_hex, role_legacy).await?;
        true
    } else {
        false
    };

    Ok(GrantOutcome {
        role_id: role_legacy,
        relinquished,
        still_creator,
    })
}

#[cfg(test)]
mod tests {
    use super::administrator_role;
    use rekindle_governance::state::{GovernanceState, RoleState};
    use rekindle_types::id::RoleId;
    use rekindle_types::permissions::{ADMINISTRATOR, KICK_MEMBERS, SEND_MESSAGES};

    fn role(id: u32, permissions: u64) -> (RoleId, RoleState) {
        (
            RoleId::from_legacy_u32(id),
            RoleState {
                name: format!("role-{id}"),
                permissions,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
                lamport: 1,
            },
        )
    }

    #[test]
    fn picks_a_role_carrying_administrator() {
        let mut state = GovernanceState::default();
        let (id, r) = role(7, ADMINISTRATOR);
        state.roles.insert(id, r);
        let (id2, r2) = role(3, SEND_MESSAGES);
        state.roles.insert(id2, r2);
        assert_eq!(administrator_role(&state), Some(RoleId::from_legacy_u32(7)));
    }

    /// Deterministic across peers: two admins granting concurrently must
    /// choose the same role or the assignments name different roles and
    /// only one of them confers anything.
    #[test]
    fn picks_the_lowest_id_when_several_qualify() {
        let mut state = GovernanceState::default();
        for id in [9u32, 2, 5] {
            let (rid, r) = role(id, ADMINISTRATOR);
            state.roles.insert(rid, r);
        }
        assert_eq!(administrator_role(&state), Some(RoleId::from_legacy_u32(2)));
    }

    /// Refused rather than invented. Creating an admin role silently
    /// would hand out a permission structure the community never chose.
    #[test]
    fn none_when_no_role_carries_administrator() {
        let mut state = GovernanceState::default();
        let (id, r) = role(4, KICK_MEMBERS | SEND_MESSAGES);
        state.roles.insert(id, r);
        assert_eq!(administrator_role(&state), None);
    }
}
