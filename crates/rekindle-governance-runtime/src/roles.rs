//! Phase 23.D.15 — role-mutation orchestrators ported from
//! `src-tauri/services/community_role_runtime.rs`. Each combines a
//! governance entry write (via `apply::write_entry`) with the
//! AppState + SQLite mirror that the deps adapter holds.
//!
//! Crate side: governance entry construction + lamport coordination
//! + random unique role id allocation. Role sets are the merged role assignments.
//! Deps side: AppState mutation + DB persists (each Tier-9 adapter
//! decides how to map the abstract `RoleStateChange` to its concrete
//! `state.communities` write + DB row update).

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{PseudonymKey, RoleId};

use crate::apply;
use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;

/// Role-row fields the adapter mirrors into AppState + DB. The
/// crate-side orchestrator builds this and hands it to
/// `RoleTable::apply_role_create`.
#[derive(Debug, Clone)]
pub struct RoleSnapshotInsert {
    pub role_id: RoleId,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    pub position: u32,
    pub hoist: bool,
    pub mentionable: bool,
    pub self_assignable: bool,
    pub exclusion_group: Option<String>,
}

/// The exclusion-group edit `RoleUpdate` carries (one enum, declared
/// with the request in `rekindle-types`).
pub use rekindle_types::governance::ExclusionGroupEdit;

/// Partial-update payload for `edit_role`. `None` fields are unchanged
/// — except `exclusion_group` which uses the three-state
/// [`ExclusionGroupEdit`] enum because `Option<Option<String>>` is
/// architecturally banned by the `clippy::option_option` lint.
#[derive(Debug, Clone, Default)]
pub struct RoleSnapshotPatch {
    pub name: Option<String>,
    pub color: Option<u32>,
    pub permissions: Option<u64>,
    pub position: Option<u32>,
    pub hoist: Option<bool>,
    pub mentionable: Option<bool>,
    pub self_assignable: Option<bool>,
    pub exclusion_group: ExclusionGroupEdit,
}

pub async fn assign_role<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_key: &str,
    role_id: RoleId,
) -> Result<(), GovernanceRuntimeError> {
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleAssignment {
            target: PseudonymKey::from_hex_lossy(pseudonym_key),
            role_id,
        },
    )
    .await?;
    // Every reader derives the member's roles, and ours, from the merged
    // role assignments (plan E3.2, D15): nothing else to store.
    Ok(())
}

pub async fn unassign_role<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_key: &str,
    role_id: RoleId,
) -> Result<(), GovernanceRuntimeError> {
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleUnassignment {
            target: PseudonymKey::from_hex_lossy(pseudonym_key),
            role_id,
        },
    )
    .await?;
    // Every reader derives the member's roles, and ours, from the merged
    // role assignments (plan E3.2, D15): nothing else to store.
    Ok(())
}

/// Allocate a fresh 16-byte role id that's not already in use and is
/// not `@everyone`. Caller passes the existing id set (read from
/// governance state); we try up to 64 random ids before giving up.
fn allocate_unique_role_id(
    existing_ids: &[RoleId],
    rng: &dyn rekindle_utils::RandomSource,
) -> Option<RoleId> {
    for _ in 0..64 {
        let candidate = RoleId(rekindle_utils::random::id_bytes_16_from(rng));
        if candidate != RoleId::EVERYONE && !existing_ids.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

pub async fn create_role<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    name: String,
    color: u32,
    permissions: u64,
    hoist: bool,
    mentionable: bool,
    self_assignable: bool,
    exclusion_group: Option<String>,
) -> Result<RoleId, GovernanceRuntimeError> {
    let (existing_ids, next_position) = deps.roles().role_table_summary(community_id);
    let role_id = allocate_unique_role_id(&existing_ids, deps.rng().as_ref()).ok_or_else(|| {
        GovernanceRuntimeError::Adapter("failed to allocate unique role id".into())
    })?;

    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleDefinition {
            role_id,
            name: name.clone(),
            permissions,
            position: next_position,
            color,
            hoist,
            mentionable,
            self_assignable,
            exclusion_group: exclusion_group.clone(),
        },
    )
    .await?;

    let snapshot = RoleSnapshotInsert {
        role_id,
        name,
        color,
        permissions,
        position: next_position,
        hoist,
        mentionable,
        self_assignable,
        exclusion_group,
    };
    deps.roles()
        .apply_role_create(community_id, snapshot)
        .await?;
    Ok(role_id)
}

pub async fn edit_role<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    role_id: RoleId,
    patch: RoleSnapshotPatch,
) -> Result<(), GovernanceRuntimeError> {
    let current = deps
        .roles()
        .role_current_definition(community_id, role_id)
        .ok_or_else(|| {
            GovernanceRuntimeError::Adapter(format!("role not found: {}", role_id.to_hex()))
        })?;

    let next_exclusion = match &patch.exclusion_group {
        ExclusionGroupEdit::Unchanged => current.exclusion_group,
        ExclusionGroupEdit::Clear => None,
        ExclusionGroupEdit::Set(s) => Some(s.clone()),
    };

    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleDefinition {
            role_id,
            name: patch.name.clone().unwrap_or(current.name),
            permissions: patch.permissions.unwrap_or(current.permissions),
            position: patch.position.unwrap_or(current.position),
            color: patch.color.unwrap_or(current.color),
            hoist: patch.hoist.unwrap_or(current.hoist),
            mentionable: patch.mentionable.unwrap_or(current.mentionable),
            self_assignable: patch.self_assignable.unwrap_or(current.self_assignable),
            exclusion_group: next_exclusion,
        },
    )
    .await?;

    deps.roles()
        .apply_role_edit(community_id, role_id, patch)
        .await
}

pub async fn delete_role<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    role_id: RoleId,
) -> Result<(), GovernanceRuntimeError> {
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleArchived { role_id },
    )
    .await?;
    // The merge drops the deleted role from every assignment.
    Ok(())
}

#[cfg(test)]
#[path = "roles_tests.rs"]
mod tests;
