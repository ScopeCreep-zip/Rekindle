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
        role_id: RoleId,
    ) -> Option<RoleSnapshotInsert> {
        let state = self.governance_state_impl(community_id)?;
        let role = state.roles.get(&role_id)?;
        Some(RoleSnapshotInsert {
            role_id,
            name: role.name.clone(),
            color: role.color,
            permissions: role.permissions,
            position: role.position,
            hoist: role.hoist,
            mentionable: role.mentionable,
            self_assignable: role.self_assignable,
            exclusion_group: role.exclusion_group.clone(),
        })
    }

    /// `(existing_role_ids, next_position)` for allocating a new role.
    ///
    /// Next position is one past the highest in use below `u32::MAX`, so a
    /// freshly created role sorts to the bottom of the ascending list.
    /// `u32::MAX` is the Owner role's (`origin.rs`): no created role may
    /// reach it, since `validate` admits a role only strictly below its
    /// writer's highest position.
    pub(super) fn role_table_summary_impl(&self, community_id: &str) -> (Vec<RoleId>, u32) {
        let Some(state) = self.governance_state_impl(community_id) else {
            return (Vec::new(), 0);
        };
        let ids: Vec<RoleId> = state.roles.keys().copied().collect();
        let next_position = state
            .roles
            .values()
            .map(|r| r.position)
            .filter(|position| *position < u32::MAX)
            .max()
            .map_or(0, |max| max.saturating_add(1).min(u32::MAX - 1));
        (ids, next_position)
    }
}
