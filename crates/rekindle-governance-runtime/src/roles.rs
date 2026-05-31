//! Phase 23.D.15 — role-mutation orchestrators ported from
//! `src-tauri/services/community_role_runtime.rs`. Each combines a
//! governance entry write (via `apply::write_entry`) with the
//! AppState + SQLite mirror that the deps adapter holds.
//!
//! Crate side: governance entry construction + lamport coordination
//! + random unique role id allocation + my_role_ids invariant logic.
//! Deps side: AppState mutation + DB persists (each Tier-9 adapter
//! decides how to map the abstract `RoleStateChange` to its concrete
//! `state.communities` write + DB row update).

use rand::RngCore;
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
    pub position: i32,
    pub hoist: bool,
    pub mentionable: bool,
    pub self_assignable: bool,
    pub exclusion_group: Option<String>,
}

/// Three-state edit for the exclusion-group slug. `Unchanged` is the
/// default (caller doesn't want to touch the field); `Clear` removes
/// the existing exclusion group; `Set` overwrites with a new slug.
#[derive(Debug, Clone, Default)]
pub enum ExclusionGroupEdit {
    #[default]
    Unchanged,
    Clear,
    Set(String),
}

/// Partial-update payload for `edit_role`. `None` fields are unchanged
/// — except `exclusion_group` which uses the three-state
/// [`ExclusionGroupEdit`] enum because `Option<Option<String>>` is
/// architecturally banned by the `clippy::option_option` lint.
#[derive(Debug, Clone, Default)]
pub struct RoleSnapshotPatch {
    pub name: Option<String>,
    pub color: Option<u32>,
    pub permissions: Option<u64>,
    pub position: Option<i32>,
    pub hoist: Option<bool>,
    pub mentionable: Option<bool>,
    pub self_assignable: Option<bool>,
    pub exclusion_group: ExclusionGroupEdit,
}

fn u32_to_role_id(role_id: u32) -> RoleId {
    let mut buf = [0u8; 16];
    buf[..4].copy_from_slice(&role_id.to_le_bytes());
    RoleId(buf)
}

fn hex_to_pseudo_32(hex_str: &str) -> [u8; 32] {
    hex::decode(hex_str)
        .ok()
        .and_then(|b| b.try_into().ok())
        .unwrap_or([0u8; 32])
}

pub async fn assign_role<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_key: &str,
    role_id: u32,
) -> Result<(), GovernanceRuntimeError> {
    let lamport = deps.increment_lamport(community_id);
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleAssignment {
            target: PseudonymKey(hex_to_pseudo_32(pseudonym_key)),
            role_id: u32_to_role_id(role_id),
            lamport,
        },
    )
    .await?;

    let is_self = deps
        .community_membership(community_id)
        .and_then(|m| m.my_pseudonym_hex)
        .as_deref()
        == Some(pseudonym_key);
    deps.apply_role_assignment(community_id, pseudonym_key, role_id, is_self)
        .await
}

pub async fn unassign_role<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_key: &str,
    role_id: u32,
) -> Result<(), GovernanceRuntimeError> {
    let lamport = deps.increment_lamport(community_id);
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleUnassignment {
            target: PseudonymKey(hex_to_pseudo_32(pseudonym_key)),
            role_id: u32_to_role_id(role_id),
            lamport,
        },
    )
    .await?;

    let is_self = deps
        .community_membership(community_id)
        .and_then(|m| m.my_pseudonym_hex)
        .as_deref()
        == Some(pseudonym_key);
    deps.apply_role_unassignment(community_id, pseudonym_key, role_id, is_self)
        .await
}

/// Allocate a fresh u32 role id that's not already in use. Caller
/// passes the existing id set (read from governance state); we try up
/// to 64 random ids before giving up.
fn allocate_unique_role_id(existing_ids: &[u32]) -> Option<u32> {
    for _ in 0..64 {
        let candidate = rand::rngs::OsRng.next_u32().saturating_add(100);
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
    let role_id = allocate_unique_role_id(&existing_ids).ok_or_else(|| {
        GovernanceRuntimeError::Adapter("failed to allocate unique role id".into())
    })?;

    let lamport = deps.increment_lamport(community_id);
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleDefinition {
            role_id: u32_to_role_id(role_id),
            name: name.clone(),
            permissions,
            position: u32::try_from(next_position).unwrap_or(0),
            color,
            hoist,
            mentionable,
            self_assignable,
            exclusion_group: exclusion_group.clone(),
            lamport,
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

    let lamport = deps.increment_lamport(community_id);
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleDefinition {
            role_id: u32_to_role_id(role_id),
            name: patch.name.clone().unwrap_or(current.name),
            permissions: patch.permissions.unwrap_or(current.permissions),
            position: u32::try_from(patch.position.unwrap_or(current.position)).unwrap_or(0),
            color: patch.color.unwrap_or(current.color),
            hoist: patch.hoist.unwrap_or(current.hoist),
            mentionable: patch.mentionable.unwrap_or(current.mentionable),
            self_assignable: patch.self_assignable.unwrap_or(current.self_assignable),
            exclusion_group: next_exclusion,
            lamport,
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
    let lamport = deps.increment_lamport(community_id);
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RoleArchived {
            role_id: u32_to_role_id(role_id),
            lamport,
        },
    )
    .await?;

    deps.apply_role_delete(community_id, role_id).await
}

/// Resolve the local member's pseudonym key for `community_id`, but
/// only if `role_id` exists and is marked `self_assignable`. Used by
/// the self-assign / self-unassign command paths.
pub fn resolve_self_assignable_pseudonym<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    role_id: u32,
) -> Result<String, GovernanceRuntimeError> {
    let current = deps
        .role_current_definition(community_id, role_id)
        .ok_or_else(|| GovernanceRuntimeError::Adapter(format!("role not found: {role_id}")))?;
    if !current.self_assignable {
        return Err(GovernanceRuntimeError::Adapter(
            "role is not self-assignable".into(),
        ));
    }
    deps.community_membership(community_id)
        .and_then(|m| m.my_pseudonym_hex)
        .ok_or_else(|| {
            GovernanceRuntimeError::Adapter("no pseudonym key for this community".into())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_unique_role_id_returns_unused() {
        let existing = vec![100, 200, 300];
        let id = allocate_unique_role_id(&existing).expect("found");
        assert!(!existing.contains(&id));
    }

    #[test]
    fn allocate_unique_role_id_minimum_threshold() {
        let id = allocate_unique_role_id(&[]).expect("found");
        assert!(
            id >= 100,
            "id should be >= 100 to avoid built-in reserved range"
        );
    }

    #[test]
    fn u32_to_role_id_round_trips_low_bytes() {
        let r = u32_to_role_id(0x0102_0304);
        assert_eq!(r.0[0..4], [0x04, 0x03, 0x02, 0x01]);
        assert_eq!(r.0[4..], [0u8; 12]);
    }

    #[test]
    fn hex_to_pseudo_zero_on_bad_hex() {
        assert_eq!(hex_to_pseudo_32("not hex"), [0u8; 32]);
    }
}
