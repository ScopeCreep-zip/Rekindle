//! Role paths.
//!
//! **The daemon keeps no role table, deliberately.** The Tauri host
//! mirrors roles into `AppState.community.roles` and a `community_roles`
//! SQLite table because its UI reads them synchronously on every render.
//! The daemon has no such reader, and adding a mirror would create a
//! second source of truth against the CRDT — the thing v2.0 exists to
//! avoid. Role definitions live in the merged `GovernanceState`, which
//! is authoritative, so the reads below go there.
//!
//! The `apply_role_*` methods are consequently narrow: the real change
//! is the governance entry `rekindle-governance-runtime` has already
//! written and merged. All that remains locally is our *own* role list
//! in `session.json`, which the daemon needs to answer permission checks
//! without re-merging.

use rekindle_governance_runtime::roles::RoleSnapshotInsert;

use rekindle_types::id::RoleId;

use super::DaemonGovernanceAdapter;

impl DaemonGovernanceAdapter<'_> {
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

    /// Add `role_id` to our own role list when the target is us.
    ///
    /// A no-op for other members: their roles come from the CRDT on the
    /// next merge, and mirroring them here would be the second source of
    /// truth this module's header rules out.
    pub(super) fn apply_role_assignment_impl(
        &self,
        community_id: &str,
        role_id: u32,
        is_self: bool,
    ) {
        if !is_self {
            return;
        }
        self.mutate_my_roles(community_id, |roles| {
            if !roles.contains(&role_id) {
                roles.push(role_id);
            }
        });
    }

    pub(super) fn apply_role_unassignment_impl(
        &self,
        community_id: &str,
        role_id: u32,
        is_self: bool,
    ) {
        if !is_self {
            return;
        }
        self.mutate_my_roles(community_id, |roles| roles.retain(|r| *r != role_id));
    }

    /// Deleting a role must still drop it from our own list — otherwise
    /// we would keep claiming a role the CRDT no longer defines, and
    /// permission checks would compute against a stale bit.
    pub(super) fn apply_role_delete_impl(&self, community_id: &str, role_id: u32) {
        self.mutate_my_roles(community_id, |roles| roles.retain(|r| *r != role_id));
    }

    /// Apply `f` to our persisted role list and save if it changed.
    fn mutate_my_roles(&self, community_id: &str, f: impl FnOnce(&mut Vec<u32>)) {
        let changed = {
            let mut guard = self.ctx.session.write();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let Some(membership) = session.communities.get_mut(community_id) else {
                return;
            };
            let before = membership.role_ids.clone();
            f(&mut membership.role_ids);
            membership.role_ids != before
        };
        if changed {
            self.persist_session();
        }
    }
}
