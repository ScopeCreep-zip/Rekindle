//! The role table behind the role commands (Phase 23.D.15): one
//! responsibility of the runtime's deps, reached through
//! [`GovernanceRuntimeDeps::roles`](super::GovernanceRuntimeDeps::roles).

use async_trait::async_trait;

use crate::error::GovernanceRuntimeError;

/// Reads and writes of a community's role table.
#[async_trait]
pub trait RoleTable: Send + Sync {
    /// Snapshot of an existing role's current definition. Used by
    /// `edit_role` to compute the merged-update RoleDefinition.
    fn role_current_definition(
        &self,
        community_id: &str,
        role_id: rekindle_types::id::RoleId,
    ) -> Option<crate::roles::RoleSnapshotInsert>;

    /// Return `(existing_ids, next_position)` for `create_role` —
    /// used to allocate a unique random role id and assign the next
    /// position slot.
    fn role_table_summary(&self, community_id: &str) -> (Vec<rekindle_types::id::RoleId>, u32);

    /// Insert a new role into AppState + DB. The `RoleSnapshotInsert`
    /// is fully populated; adapter mirrors into `community.roles` (sort
    /// by position) + `community_roles` SQLite row.
    async fn apply_role_create(
        &self,
        community_id: &str,
        snapshot: crate::roles::RoleSnapshotInsert,
    ) -> Result<(), GovernanceRuntimeError>;

    /// Apply a partial-update patch to an existing role. Adapter
    /// mutates AppState `community.roles[role_id]` + emits the matching
    /// `community_roles` UPDATE.
    async fn apply_role_edit(
        &self,
        community_id: &str,
        role_id: rekindle_types::id::RoleId,
        patch: crate::roles::RoleSnapshotPatch,
    ) -> Result<(), GovernanceRuntimeError>;
}
