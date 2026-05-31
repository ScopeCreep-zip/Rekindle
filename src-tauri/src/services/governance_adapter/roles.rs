//! Phase 23.D.15 — role-mutation deps method impls extracted from
//! `deps_impl.rs` to keep it under the 500-LoC cap. Each impl is the
//! AppState mutation + DB row write the crate-side
//! `rekindle_governance_runtime::roles` orchestrator delegates to.

use rekindle_governance_runtime::error::GovernanceRuntimeError;
use rekindle_governance_runtime::roles::{ExclusionGroupEdit, RoleSnapshotInsert, RoleSnapshotPatch};

use crate::db_helpers::db_call;
use crate::state::RoleDefinition;
use crate::state_helpers;

use super::GovernanceAdapter;

pub(super) fn role_current_definition_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    role_id: u32,
) -> Option<RoleSnapshotInsert> {
    let communities = adapter.state.communities.read();
    let community = communities.get(community_id)?;
    let r = community.roles.iter().find(|r| r.id == role_id)?;
    Some(RoleSnapshotInsert {
        role_id: r.id,
        name: r.name.clone(),
        color: r.color,
        permissions: r.permissions,
        position: r.position,
        hoist: r.hoist,
        mentionable: r.mentionable,
        self_assignable: r.self_assignable,
        exclusion_group: r.exclusion_group.clone(),
    })
}

pub(super) fn role_table_summary_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
) -> (Vec<u32>, i32) {
    let communities = adapter.state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return (Vec::new(), 0);
    };
    let existing_ids: Vec<u32> = community.roles.iter().map(|r| r.id).collect();
    let next_position = community
        .roles
        .iter()
        .map(|r| r.position)
        .max()
        .unwrap_or(-1)
        .saturating_add(1);
    (existing_ids, next_position)
}

pub(super) async fn apply_role_assignment_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    pseudonym_key: &str,
    role_id: u32,
    is_self: bool,
) -> Result<(), GovernanceRuntimeError> {
    let owner_key = state_helpers::current_owner_key(&adapter.state)
        .map_err(GovernanceRuntimeError::Adapter)?;
    if is_self {
        let mut communities = adapter.state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            if !c.my_role_ids.contains(&role_id) {
                c.my_role_ids.push(role_id);
            }
        }
    }
    let cid = community_id.to_string();
    let pk = pseudonym_key.to_string();
    db_call(&adapter.pool, move |conn| {
        let current: String = conn
            .query_row(
                "SELECT role_ids FROM community_members WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                rusqlite::params![owner_key, cid, pk],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "[0,1]".to_string());
        let mut ids: Vec<u32> = serde_json::from_str(&current).unwrap_or_default();
        if !ids.contains(&role_id) {
            ids.push(role_id);
        }
        let new_json = serde_json::to_string(&ids).unwrap_or_default();
        conn.execute(
            "UPDATE community_members SET role_ids = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![new_json, owner_key, cid, pk],
        )?;
        Ok(())
    })
    .await
    .map_err(GovernanceRuntimeError::Adapter)
}

pub(super) async fn apply_role_unassignment_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    pseudonym_key: &str,
    role_id: u32,
    is_self: bool,
) -> Result<(), GovernanceRuntimeError> {
    let owner_key = state_helpers::current_owner_key(&adapter.state)
        .map_err(GovernanceRuntimeError::Adapter)?;
    if is_self {
        let mut communities = adapter.state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.my_role_ids.retain(|&id| id != role_id);
        }
    }
    let cid = community_id.to_string();
    let pk = pseudonym_key.to_string();
    db_call(&adapter.pool, move |conn| {
        let current: String = conn
            .query_row(
                "SELECT role_ids FROM community_members WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                rusqlite::params![owner_key, cid, pk],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "[0,1]".to_string());
        let mut ids: Vec<u32> = serde_json::from_str(&current).unwrap_or_default();
        ids.retain(|&id| id != role_id);
        let new_json = serde_json::to_string(&ids).unwrap_or_default();
        conn.execute(
            "UPDATE community_members SET role_ids = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![new_json, owner_key, cid, pk],
        )?;
        Ok(())
    })
    .await
    .map_err(GovernanceRuntimeError::Adapter)
}

pub(super) async fn apply_role_create_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    snapshot: RoleSnapshotInsert,
) -> Result<(), GovernanceRuntimeError> {
    let owner_key = state_helpers::current_owner_key(&adapter.state)
        .map_err(GovernanceRuntimeError::Adapter)?;

    let role_def = RoleDefinition {
        id: snapshot.role_id,
        name: snapshot.name.clone(),
        color: snapshot.color,
        permissions: snapshot.permissions,
        position: snapshot.position,
        hoist: snapshot.hoist,
        mentionable: snapshot.mentionable,
        self_assignable: snapshot.self_assignable,
        exclusion_group: snapshot.exclusion_group.clone(),
    };
    {
        let mut communities = adapter.state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.roles.push(role_def);
            c.roles.sort_by_key(|role| role.position);
        }
    }

    let cid = community_id.to_string();
    db_call(&adapter.pool, move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable, self_assignable, exclusion_group) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                owner_key,
                cid,
                snapshot.role_id,
                snapshot.name,
                snapshot.color,
                snapshot.permissions.cast_signed(),
                snapshot.position,
                snapshot.hoist,
                snapshot.mentionable,
                snapshot.self_assignable,
                snapshot.exclusion_group,
            ],
        )?;
        Ok(())
    })
    .await
    .map_err(GovernanceRuntimeError::Adapter)
}

pub(super) async fn apply_role_edit_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    role_id: u32,
    patch: RoleSnapshotPatch,
) -> Result<(), GovernanceRuntimeError> {
    let owner_key = state_helpers::current_owner_key(&adapter.state)
        .map_err(GovernanceRuntimeError::Adapter)?;
    {
        let mut communities = adapter.state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            if let Some(r) = c.roles.iter_mut().find(|r| r.id == role_id) {
                if let Some(ref n) = patch.name {
                    r.name.clone_from(n);
                }
                if let Some(col) = patch.color {
                    r.color = col;
                }
                if let Some(p) = patch.permissions {
                    r.permissions = p;
                }
                if let Some(pos) = patch.position {
                    r.position = pos;
                }
                if let Some(h) = patch.hoist {
                    r.hoist = h;
                }
                if let Some(m) = patch.mentionable {
                    r.mentionable = m;
                }
                if let Some(sa) = patch.self_assignable {
                    r.self_assignable = sa;
                }
                match &patch.exclusion_group {
                    ExclusionGroupEdit::Unchanged => {}
                    ExclusionGroupEdit::Clear => r.exclusion_group = None,
                    ExclusionGroupEdit::Set(s) => r.exclusion_group = Some(s.clone()),
                }
            }
        }
    }

    let cid = community_id.to_string();
    db_call(&adapter.pool, move |conn| {
        let mut sets = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(n) = patch.name {
            sets.push("name = ?");
            params.push(Box::new(n));
        }
        if let Some(col) = patch.color {
            sets.push("color = ?");
            params.push(Box::new(col));
        }
        if let Some(p) = patch.permissions {
            sets.push("permissions = ?");
            params.push(Box::new(p.cast_signed()));
        }
        if let Some(pos) = patch.position {
            sets.push("position = ?");
            params.push(Box::new(pos));
        }
        if let Some(h) = patch.hoist {
            sets.push("hoist = ?");
            params.push(Box::new(h));
        }
        if let Some(m) = patch.mentionable {
            sets.push("mentionable = ?");
            params.push(Box::new(m));
        }
        if let Some(sa) = patch.self_assignable {
            sets.push("self_assignable = ?");
            params.push(Box::new(sa));
        }
        match patch.exclusion_group {
            ExclusionGroupEdit::Unchanged => {}
            ExclusionGroupEdit::Clear => {
                sets.push("exclusion_group = ?");
                params.push(Box::new(None::<String>));
            }
            ExclusionGroupEdit::Set(s) => {
                sets.push("exclusion_group = ?");
                params.push(Box::new(Some(s)));
            }
        }
        if !sets.is_empty() {
            let sql = format!(
                "UPDATE community_roles SET {} WHERE owner_key = ? AND community_id = ? AND role_id = ?",
                sets.join(", ")
            );
            params.push(Box::new(owner_key));
            params.push(Box::new(cid));
            params.push(Box::new(role_id));
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(std::convert::AsRef::as_ref).collect();
            conn.execute(&sql, param_refs.as_slice())?;
        }
        Ok(())
    })
    .await
    .map_err(GovernanceRuntimeError::Adapter)
}

pub(super) async fn apply_role_delete_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    role_id: u32,
) -> Result<(), GovernanceRuntimeError> {
    let owner_key = state_helpers::current_owner_key(&adapter.state)
        .map_err(GovernanceRuntimeError::Adapter)?;

    {
        let mut communities = adapter.state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.roles.retain(|r| r.id != role_id);
            c.my_role_ids.retain(|&id| id != role_id);
        }
    }

    let cid = community_id.to_string();
    db_call(&adapter.pool, move |conn| {
        conn.execute(
            "DELETE FROM community_roles WHERE owner_key = ? AND community_id = ? AND role_id = ?",
            rusqlite::params![owner_key, cid, role_id],
        )?;
        let mut stmt = conn.prepare(
            "SELECT pseudonym_key, role_ids FROM community_members WHERE owner_key = ? AND community_id = ?",
        )?;
        let members: Vec<(String, String)> = stmt
            .query_map(rusqlite::params![owner_key, cid], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(std::result::Result::ok)
            .collect();
        drop(stmt);
        for (pk, json) in &members {
            let mut ids: Vec<u32> = serde_json::from_str(json).unwrap_or_default();
            if ids.contains(&role_id) {
                ids.retain(|&id| id != role_id);
                let new_json = serde_json::to_string(&ids).unwrap_or_default();
                conn.execute(
                    "UPDATE community_members SET role_ids = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                    rusqlite::params![new_json, owner_key, cid, pk],
                )?;
            }
        }
        let my_ids_json: String = conn
            .query_row(
                "SELECT my_role_ids FROM communities WHERE owner_key = ? AND id = ?",
                rusqlite::params![owner_key, cid],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "[0,1]".to_string());
        let mut my_ids: Vec<u32> = serde_json::from_str(&my_ids_json).unwrap_or_default();
        if my_ids.contains(&role_id) {
            my_ids.retain(|&id| id != role_id);
            let new_json = serde_json::to_string(&my_ids).unwrap_or_default();
            conn.execute(
                "UPDATE communities SET my_role_ids = ? WHERE owner_key = ? AND id = ?",
                rusqlite::params![new_json, owner_key, cid],
            )?;
        }
        Ok(())
    })
    .await
    .map_err(GovernanceRuntimeError::Adapter)
}
