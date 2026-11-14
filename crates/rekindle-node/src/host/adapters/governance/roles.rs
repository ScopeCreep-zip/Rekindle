//! Role paths.
//!
//! **The Host keeps no role mirror of its own.** Role definitions and
//! assignments live in the merged `GovernanceState`, which is
//! authoritative; the community store's role list, our role ids and the
//! `community_roles` rows are its projection (`projection.rs`), laid down
//! by the merge `write_entry` runs. A member's roles, and ours, are read
//! from the merged role assignments (plan E3.2, D15); nothing caches them,
//! so an assignment, an unassignment or a role deletion has nothing left
//! to store here.

use rekindle_governance_runtime::roles::RoleSnapshotInsert;

use rekindle_types::id::RoleId;

use super::HostGovernance;

impl HostGovernance {
    /// A role's current definition, read from the merged CRDT state.
    pub(super) fn role_current_definition_impl(
        &self,
        community_id: &str,
        role_id: u32,
    ) -> Option<RoleSnapshotInsert> {
        let state = self.governance_state_impl(community_id)?;
        let role = state.roles.get(&RoleId::from_legacy_u32(role_id))?;
        Some(RoleSnapshotInsert {
            role_id,
            name: role.name.clone(),
            color: role.color,
            permissions: role.permissions,
            position: i32::try_from(role.position).unwrap_or(i32::MAX),
            hoist: role.hoist,
            mentionable: role.mentionable,
            self_assignable: role.self_assignable,
            exclusion_group: role.exclusion_group.clone(),
        })
    }

    /// `(existing_role_ids, next_position)` for allocating a new role.
    ///
    /// Next position is one past the highest in use, so a freshly
    /// created role sorts to the bottom — matching the desktop.
    pub(super) fn role_table_summary_impl(&self, community_id: &str) -> (Vec<u32>, i32) {
        let Some(state) = self.governance_state_impl(community_id) else {
            return (Vec::new(), 0);
        };
        let ids: Vec<u32> = state
            .roles
            .keys()
            .copied()
            .map(RoleId::to_legacy_u32)
            .collect();
        let next_position = state
            .roles
            .values()
            .map(|r| i32::try_from(r.position).unwrap_or(i32::MAX))
            .max()
            .map_or(0, |max| max.saturating_add(1));
        (ids, next_position)
    }
}
