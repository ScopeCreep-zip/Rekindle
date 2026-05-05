use tauri::State;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::{hex_to_pseudo_32, require_permission, u32_to_role_id};
use super::types::CommunityRoleDto;

async fn assign_role_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    pseudonym_key: &str,
    role_id: u32,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;

    let lamport = state_helpers::increment_lamport(state, community_id);
    crate::services::community::write_entry(
        state,
        community_id,
        rekindle_types::governance::GovernanceEntry::RoleAssignment {
            target: rekindle_types::id::PseudonymKey(hex_to_pseudo_32(pseudonym_key)),
            role_id: u32_to_role_id(role_id),
            lamport,
        },
    )
    .await?;

    let is_self = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .is_some_and(|c| c.my_pseudonym_key.as_deref() == Some(pseudonym_key))
    };
    if is_self {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            if !c.my_role_ids.contains(&role_id) {
                c.my_role_ids.push(role_id);
            }
        }
    }

    let cid = community_id.to_string();
    let pk = pseudonym_key.to_string();
    db_call(pool, move |conn| {
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
    .await?;
    Ok(())
}

async fn unassign_role_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    pseudonym_key: &str,
    role_id: u32,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;

    let lamport = state_helpers::increment_lamport(state, community_id);
    crate::services::community::write_entry(
        state,
        community_id,
        rekindle_types::governance::GovernanceEntry::RoleUnassignment {
            target: rekindle_types::id::PseudonymKey(hex_to_pseudo_32(pseudonym_key)),
            role_id: u32_to_role_id(role_id),
            lamport,
        },
    )
    .await?;

    let is_self = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .is_some_and(|c| c.my_pseudonym_key.as_deref() == Some(pseudonym_key))
    };
    if is_self {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.my_role_ids.retain(|&id| id != role_id);
        }
    }

    let cid = community_id.to_string();
    let pk = pseudonym_key.to_string();
    db_call(pool, move |conn| {
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
    .await?;
    Ok(())
}

/// Get all role definitions for a community from the merged governance state.
#[tauri::command]
pub async fn get_roles(
    community_id: String,
    state: State<'_, SharedState>,
    _pool: State<'_, DbPool>,
) -> Result<Vec<CommunityRoleDto>, String> {
    let communities = state.communities.read();
    let community = communities
        .get(&community_id)
        .ok_or("community not found")?;
    Ok(community.roles.iter().map(CommunityRoleDto::from).collect())
}

/// Create a new role in a community.
///
/// `permissions` is accepted as a string to avoid JavaScript `Number` precision loss
/// on u64 values above `2^53 - 1`.
#[tauri::command]
pub async fn create_role(
    community_id: String,
    name: String,
    color: u32,
    permissions: String,
    hoist: bool,
    mentionable: bool,
    self_assignable: bool,
    exclusion_group: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<u32, String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_ROLES)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let permissions_u64: u64 = permissions
        .parse()
        .map_err(|e| format!("invalid permissions: {e}"))?;
    let exclusion_group = normalize_exclusion_group(exclusion_group)?;

    let (role_id, position) = {
        use rand::RngCore;

        let communities = state.communities.read();
        let community = communities
            .get(&community_id)
            .ok_or("community not found")?;
        let next_position = community
            .roles
            .iter()
            .map(|role| role.position)
            .max()
            .unwrap_or(-1)
            .saturating_add(1);

        let mut candidate = 0u32;
        let mut found = false;
        for _ in 0..64 {
            candidate = rand::rngs::OsRng.next_u32().saturating_add(100);
            if !community.roles.iter().any(|role| role.id == candidate) {
                found = true;
                break;
            }
        }

        if !found {
            return Err("failed to allocate unique role id".into());
        }

        (candidate, next_position)
    };

    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::RoleDefinition {
            role_id: u32_to_role_id(role_id),
            name: name.clone(),
            permissions: permissions_u64,
            position: u32::try_from(position).unwrap_or(0),
            color,
            hoist,
            mentionable,
            self_assignable,
            exclusion_group: exclusion_group.clone(),
            lamport,
        },
    )
    .await?;

    let role_def = crate::state::RoleDefinition {
        id: role_id,
        name: name.clone(),
        color,
        permissions: permissions_u64,
        position,
        hoist,
        mentionable,
        self_assignable,
        exclusion_group: exclusion_group.clone(),
    };
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(&community_id) {
            c.roles.push(role_def);
            c.roles.sort_by_key(|role| role.position);
        }
    }

    let cid = community_id.clone();
    let exclusion_group_db = exclusion_group;
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable, self_assignable, exclusion_group) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![owner_key, cid, role_id, name, color, permissions_u64.cast_signed(), position, hoist, mentionable, self_assignable, exclusion_group_db],
        )?;
        Ok(())
    }).await?;
    Ok(role_id)
}

/// Architecture §19.4 — explicit edit verb so callers can either set a
/// new exclusion-group slug or clear it. Omitting the field entirely on
/// `edit_role` is "no change". Tagged with serde so the frontend can
/// pass `{ "kind": "set", "value": "pronouns" }` or
/// `{ "kind": "clear" }`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum ExclusionGroupEdit {
    Set(String),
    Clear,
}

/// Architecture §19.4 — exclusion-group slugs are case-insensitive
/// short tags. Normalise to lowercase and reject overlong / control
/// chars; treat `Some("")` as `None`.
fn normalize_exclusion_group(raw: Option<String>) -> Result<Option<String>, String> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            if trimmed.chars().count() > 32 {
                return Err("exclusion_group must be ≤32 characters".into());
            }
            if !trimmed
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
            {
                return Err(
                    "exclusion_group may only contain letters, numbers, '_' or '-'".into()
                );
            }
            Ok(Some(trimmed.to_ascii_lowercase()))
        }
    }
}

/// Edit an existing role in a community.
///
/// `permissions` is accepted as a string to avoid JavaScript `Number` precision loss
/// on u64 values above `2^53 - 1`.
#[tauri::command]
pub async fn edit_role(
    community_id: String,
    role_id: u32,
    name: Option<String>,
    color: Option<u32>,
    permissions: Option<String>,
    position: Option<i32>,
    hoist: Option<bool>,
    mentionable: Option<bool>,
    self_assignable: Option<bool>,
    // Architecture §19.4 — see `ExclusionGroupEdit`.
    exclusion_group: Option<ExclusionGroupEdit>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_ROLES)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let permissions_u64: Option<u64> = permissions
        .map(|s| s.parse::<u64>())
        .transpose()
        .map_err(|e| format!("invalid permissions: {e}"))?;

    let normalised_exclusion: Option<Option<String>> = match exclusion_group {
        None => None,
        Some(ExclusionGroupEdit::Clear) => Some(None),
        Some(ExclusionGroupEdit::Set(value)) => Some(normalize_exclusion_group(Some(value))?),
    };

    let (
        cur_name,
        cur_color,
        cur_perms,
        cur_pos,
        cur_hoist,
        cur_ment,
        cur_self_assignable,
        cur_exclusion,
    ) = {
        let communities = state.communities.read();
        let c = communities
            .get(&community_id)
            .ok_or("community not found")?;
        let r = c
            .roles
            .iter()
            .find(|r| r.id == role_id)
            .ok_or("role not found")?;
        (
            r.name.clone(),
            r.color,
            r.permissions,
            r.position,
            r.hoist,
            r.mentionable,
            r.self_assignable,
            r.exclusion_group.clone(),
        )
    };
    let next_exclusion = normalised_exclusion.clone().unwrap_or(cur_exclusion);
    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::RoleDefinition {
            role_id: u32_to_role_id(role_id),
            name: name.clone().unwrap_or(cur_name),
            permissions: permissions_u64.unwrap_or(cur_perms),
            position: u32::try_from(position.unwrap_or(cur_pos)).unwrap_or(0),
            color: color.unwrap_or(cur_color),
            hoist: hoist.unwrap_or(cur_hoist),
            mentionable: mentionable.unwrap_or(cur_ment),
            self_assignable: self_assignable.unwrap_or(cur_self_assignable),
            exclusion_group: next_exclusion.clone(),
            lamport,
        },
    )
    .await?;

    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(&community_id) {
            if let Some(r) = c.roles.iter_mut().find(|r| r.id == role_id) {
                if let Some(ref n) = name {
                    r.name.clone_from(n);
                }
                if let Some(col) = color {
                    r.color = col;
                }
                if let Some(p) = permissions_u64 {
                    r.permissions = p;
                }
                if let Some(pos) = position {
                    r.position = pos;
                }
                if let Some(h) = hoist {
                    r.hoist = h;
                }
                if let Some(m) = mentionable {
                    r.mentionable = m;
                }
                if let Some(sa) = self_assignable {
                    r.self_assignable = sa;
                }
                if let Some(ref next) = normalised_exclusion {
                    r.exclusion_group.clone_from(next);
                }
            }
        }
    }

    let cid = community_id.clone();
    db_call(pool.inner(), move |conn| {
        let mut sets = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(n) = name {
            sets.push("name = ?");
            params.push(Box::new(n));
        }
        if let Some(col) = color {
            sets.push("color = ?");
            params.push(Box::new(col));
        }
        if let Some(p) = permissions_u64 {
            sets.push("permissions = ?");
            params.push(Box::new(p.cast_signed()));
        }
        if let Some(pos) = position {
            sets.push("position = ?");
            params.push(Box::new(pos));
        }
        if let Some(h) = hoist {
            sets.push("hoist = ?");
            params.push(Box::new(h));
        }
        if let Some(m) = mentionable {
            sets.push("mentionable = ?");
            params.push(Box::new(m));
        }
        if let Some(sa) = self_assignable {
            sets.push("self_assignable = ?");
            params.push(Box::new(sa));
        }
        if let Some(eg) = normalised_exclusion {
            sets.push("exclusion_group = ?");
            params.push(Box::new(eg));
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
    .await?;
    Ok(())
}

/// Delete a role from a community.
#[tauri::command]
pub async fn delete_role(
    community_id: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_ROLES)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    let lamport = state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::RoleArchived {
            role_id: u32_to_role_id(role_id),
            lamport,
        },
    )
    .await?;

    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(&community_id) {
            c.roles.retain(|r| r.id != role_id);
            c.my_role_ids.retain(|&id| id != role_id);
        }
    }

    let cid = community_id.clone();
    db_call(pool.inner(), move |conn| {
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
        let rid = role_id;
        for (pk, json) in &members {
            let mut ids: Vec<u32> = serde_json::from_str(json).unwrap_or_default();
            if ids.contains(&rid) {
                ids.retain(|&id| id != rid);
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
        if my_ids.contains(&rid) {
            my_ids.retain(|&id| id != rid);
            let new_json = serde_json::to_string(&my_ids).unwrap_or_default();
            conn.execute(
                "UPDATE communities SET my_role_ids = ? WHERE owner_key = ? AND id = ?",
                rusqlite::params![new_json, owner_key, cid],
            )?;
        }
        Ok(())
    })
    .await?;
    Ok(())
}

/// Assign a role to a member (additive — does not remove other roles).
#[tauri::command]
pub async fn assign_role(
    community_id: String,
    pseudonym_key: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_ROLES)?;
    assign_role_inner(state.inner(), pool.inner(), &community_id, &pseudonym_key, role_id).await
}

/// Remove a role from a member.
#[tauri::command]
pub async fn unassign_role(
    community_id: String,
    pseudonym_key: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_ROLES)?;
    unassign_role_inner(state.inner(), pool.inner(), &community_id, &pseudonym_key, role_id).await
}

/// Assign a self-assignable role to the current member.
#[tauri::command]
pub async fn self_assign_role(
    community_id: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let pseudonym_key = {
        let communities = state.communities.read();
        let community = communities
            .get(&community_id)
            .ok_or("community not found")?;
        let role = community
            .roles
            .iter()
            .find(|role| role.id == role_id)
            .ok_or("role not found")?;
        if !role.self_assignable {
            return Err("role is not self-assignable".into());
        }
        community
            .my_pseudonym_key
            .clone()
            .ok_or("no pseudonym key for this community")?
    };

    assign_role_inner(state.inner(), pool.inner(), &community_id, &pseudonym_key, role_id).await
}

/// Remove a self-assignable role from the current member.
#[tauri::command]
pub async fn self_unassign_role(
    community_id: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let pseudonym_key = {
        let communities = state.communities.read();
        let community = communities
            .get(&community_id)
            .ok_or("community not found")?;
        let role = community
            .roles
            .iter()
            .find(|role| role.id == role_id)
            .ok_or("role not found")?;
        if !role.self_assignable {
            return Err("role is not self-assignable".into());
        }
        community
            .my_pseudonym_key
            .clone()
            .ok_or("no pseudonym key for this community")?
    };

    unassign_role_inner(state.inner(), pool.inner(), &community_id, &pseudonym_key, role_id).await
}
