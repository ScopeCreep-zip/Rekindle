//! Role paths.
//!
//! **The Host keeps no role-definition mirror of its own.** Role
//! definitions live in the merged `GovernanceState`, which is
//! authoritative; the community store's role list and the
//! `community_roles` rows are its projection (`projection.rs`), laid down
//! by the merge `write_entry` runs before any `apply_role_*` call, so the
//! reads below go to the merged state.
//!
//! The `apply_role_*` methods are consequently narrow: the real change is
//! the governance entry `rekindle-governance-runtime` has already written
//! and merged. What remains locally is our *own* role list
//! (`communities.my_role_ids`, which permission checks read without
//! re-merging) and the member's stored role set
//! (`community_members.role_ids`, which the member list reads).

use rekindle_governance_runtime::roles::RoleSnapshotInsert;
use rekindle_governance_runtime::GovernanceRuntimeError;

use rekindle_types::id::RoleId;

use super::HostGovernance;

/// A change to the stored member role sets (`community_members.role_ids`).
enum MemberRoles {
    /// The member (pseudonym hex) gains the role.
    Add(String, u32),
    /// The member loses the role.
    Remove(String, u32),
    /// A deleted role leaves every member.
    RemoveEverywhere(u32),
}

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

    /// Give the member `role_id`: our own role list when the target is us,
    /// and the member's stored role set either way.
    pub(super) async fn apply_role_assignment_impl(
        &self,
        community_id: &str,
        pseudonym_key: &str,
        role_id: u32,
        is_self: bool,
    ) -> Result<(), GovernanceRuntimeError> {
        if is_self {
            self.mutate_my_roles(community_id, |roles| {
                if !roles.contains(&role_id) {
                    roles.push(role_id);
                }
            });
        }
        let write = MemberRoles::Add(pseudonym_key.to_string(), role_id);
        self.store_member_roles(community_id, write).await
    }

    pub(super) async fn apply_role_unassignment_impl(
        &self,
        community_id: &str,
        pseudonym_key: &str,
        role_id: u32,
        is_self: bool,
    ) -> Result<(), GovernanceRuntimeError> {
        if is_self {
            self.mutate_my_roles(community_id, |roles| roles.retain(|r| *r != role_id));
        }
        let write = MemberRoles::Remove(pseudonym_key.to_string(), role_id);
        self.store_member_roles(community_id, write).await
    }

    /// Deleting a role must still drop it from our own list — otherwise
    /// we would keep claiming a role the CRDT no longer defines, and
    /// permission checks would compute against a stale bit — and from
    /// every member's stored set.
    pub(super) async fn apply_role_delete_impl(
        &self,
        community_id: &str,
        role_id: u32,
    ) -> Result<(), GovernanceRuntimeError> {
        self.mutate_my_roles(community_id, |roles| roles.retain(|r| *r != role_id));
        self.store_member_roles(community_id, MemberRoles::RemoveEverywhere(role_id))
            .await
    }

    /// Apply `write` to the community's stored member role sets; an error
    /// while locked or when the write fails.
    async fn store_member_roles(
        &self,
        community_id: &str,
        write: MemberRoles,
    ) -> Result<(), GovernanceRuntimeError> {
        let not_logged_in = || GovernanceRuntimeError::Adapter(rekindle_db::NotLoggedIn.into());
        let owner = self
            .ctx
            .session
            .read()
            .as_ref()
            .map(|s| s.identity.public_key_hex.clone())
            .ok_or_else(not_logged_in)?;
        let db = self.ctx.db.current().map_err(|_| not_logged_in())?;
        let community = community_id.to_string();
        db.call(move |conn| {
            use rekindle_db::repo::members::{add_role, remove_role, remove_role_everywhere};
            match write {
                MemberRoles::Add(member, role) => add_role(conn, &owner, &community, &member, role),
                MemberRoles::Remove(member, role) => {
                    remove_role(conn, &owner, &community, &member, role)
                }
                MemberRoles::RemoveEverywhere(role) => {
                    remove_role_everywhere(conn, &owner, &community, role)
                }
            }
        })
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("member roles not stored: {e}")))
    }

    /// Apply `f` to our stored role list.
    fn mutate_my_roles(&self, community_id: &str, f: impl FnOnce(&mut Vec<u32>)) {
        crate::host::session::update_roles(
            &self.ctx.session,
            &self.ctx.db,
            &self.ctx.communities,
            community_id,
            f,
        );
    }
}
