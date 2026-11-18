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
/// `deps.apply_role_create`.
#[derive(Debug, Clone)]
pub struct RoleSnapshotInsert {
    pub role_id: u32,
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
    role_id: u32,
) -> Result<(), GovernanceRuntimeError> {
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleAssignment {
            target: PseudonymKey::from_hex_lossy(pseudonym_key),
            role_id: RoleId::from_legacy_u32(role_id),
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
    role_id: u32,
) -> Result<(), GovernanceRuntimeError> {
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleUnassignment {
            target: PseudonymKey::from_hex_lossy(pseudonym_key),
            role_id: RoleId::from_legacy_u32(role_id),
        },
    )
    .await?;
    // Every reader derives the member's roles, and ours, from the merged
    // role assignments (plan E3.2, D15): nothing else to store.
    Ok(())
}

/// Allocate a fresh u32 role id that's not already in use. Caller
/// passes the existing id set (read from governance state); we try up
/// to 64 random ids before giving up.
fn allocate_unique_role_id(
    existing_ids: &[u32],
    rng: &dyn rekindle_utils::RandomSource,
) -> Option<u32> {
    for _ in 0..64 {
        let candidate = rng.next_u32().saturating_add(100);
        if !existing_ids.contains(&candidate) {
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
) -> Result<u32, GovernanceRuntimeError> {
    let (existing_ids, next_position) = deps.role_table_summary(community_id);
    let role_id = allocate_unique_role_id(&existing_ids, deps.rng().as_ref()).ok_or_else(|| {
        GovernanceRuntimeError::Adapter("failed to allocate unique role id".into())
    })?;

    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleDefinition {
            role_id: RoleId::from_legacy_u32(role_id),
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
    deps.apply_role_create(community_id, snapshot).await?;
    Ok(role_id)
}

pub async fn edit_role<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    role_id: u32,
    patch: RoleSnapshotPatch,
) -> Result<(), GovernanceRuntimeError> {
    let current = deps
        .role_current_definition(community_id, role_id)
        .ok_or_else(|| GovernanceRuntimeError::Adapter(format!("role not found: {role_id}")))?;

    let next_exclusion = match &patch.exclusion_group {
        ExclusionGroupEdit::Unchanged => current.exclusion_group,
        ExclusionGroupEdit::Clear => None,
        ExclusionGroupEdit::Set(s) => Some(s.clone()),
    };

    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleDefinition {
            role_id: RoleId::from_legacy_u32(role_id),
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

    deps.apply_role_edit(community_id, role_id, patch).await
}

pub async fn delete_role<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    role_id: u32,
) -> Result<(), GovernanceRuntimeError> {
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleArchived {
            role_id: RoleId::from_legacy_u32(role_id),
        },
    )
    .await?;
    // The merge drops the deleted role from every assignment.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_unique_role_id_returns_unused() {
        let existing = vec![100, 200, 300];
        let id = allocate_unique_role_id(&existing, &rekindle_utils::OsRandom).expect("found");
        assert!(!existing.contains(&id));
    }

    #[test]
    fn allocate_unique_role_id_minimum_threshold() {
        let id = allocate_unique_role_id(&[], &rekindle_utils::OsRandom).expect("found");
        assert!(
            id >= 100,
            "id should be >= 100 to avoid built-in reserved range"
        );
    }

    #[test]
    fn u32_to_role_id_round_trips_low_bytes() {
        let r = RoleId::from_legacy_u32(0x0102_0304);
        assert_eq!(r.0[0..4], [0x04, 0x03, 0x02, 0x01]);
        assert_eq!(r.0[4..], [0u8; 12]);
    }

    #[test]
    fn hex_to_pseudo_zero_on_bad_hex() {
        assert_eq!(PseudonymKey::from_hex_lossy("not hex").0, [0u8; 32]);
    }
}
