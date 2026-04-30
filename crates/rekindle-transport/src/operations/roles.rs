//! Role management operations — list, create, update, delete, assign, unassign.

use tracing::info;

use crate::error::{TransportError, Result};
use crate::node::TransportNode;
use crate::payload::dht_types::RoleEntry;

/// List all roles in a community.
pub async fn list_roles(node: &TransportNode, governance_key: &str) -> Result<Vec<RoleEntry>> {
    let dht = node.dht()?;
    dht.governance().read_roles(governance_key).await
}

/// Create a new role in a community.
///
/// Reads the current role list, validates no ID collision, appends the new
/// role, and writes back atomically.
pub async fn create_role(
    node: &TransportNode,
    governance_key: &str,
    name: &str,
    permissions: u64,
    color: u32,
    position: i32,
) -> Result<RoleEntry> {
    let dht = node.dht()?;
    let mut roles = dht.governance().read_roles(governance_key).await?;

    let next_id = roles.iter().map(|r| r.id).max().map_or(1, |max| max + 1);

    let role = RoleEntry {
        id: next_id,
        name: name.to_string(),
        color,
        permissions,
        position,
        hoist: false,
        mentionable: true,
        self_assignable: false,
    };

    roles.push(role.clone());
    dht.governance().write_roles(governance_key, &roles).await?;

    info!(role_id = next_id, name, "role created");
    Ok(role)
}

/// Update an existing role's properties.
///
/// Only updates fields that are `Some`. Returns the updated role entry.
pub async fn update_role(
    node: &TransportNode,
    governance_key: &str,
    role_id: u32,
    name: Option<&str>,
    permissions: Option<u64>,
    color: Option<u32>,
) -> Result<RoleEntry> {
    let dht = node.dht()?;
    let mut roles = dht.governance().read_roles(governance_key).await?;

    let role = roles
        .iter_mut()
        .find(|r| r.id == role_id)
        .ok_or_else(|| TransportError::DhtError {
            reason: format!("role {role_id} not found"),
        })?;

    if let Some(n) = name {
        role.name = n.to_string();
    }
    if let Some(p) = permissions {
        role.permissions = p;
    }
    if let Some(c) = color {
        role.color = c;
    }

    let updated = role.clone();
    dht.governance().write_roles(governance_key, &roles).await?;

    info!(role_id, "role updated");
    Ok(updated)
}

/// Delete a role by ID.
pub async fn delete_role(
    node: &TransportNode,
    governance_key: &str,
    role_id: u32,
) -> Result<()> {
    let dht = node.dht()?;
    let mut roles = dht.governance().read_roles(governance_key).await?;

    let before = roles.len();
    roles.retain(|r| r.id != role_id);

    if roles.len() == before {
        return Err(TransportError::DhtError {
            reason: format!("role {role_id} not found"),
        });
    }

    dht.governance().write_roles(governance_key, &roles).await?;
    info!(role_id, "role deleted");
    Ok(())
}

/// Assign a role to a member by updating the member index.
pub async fn assign_role(
    node: &TransportNode,
    registry_key: &str,
    member_pseudonym: &str,
    role_id: u32,
) -> Result<()> {
    let dht = node.dht()?;
    let mut members = dht.registry().read_member_index(registry_key).await?;

    let member = members
        .iter_mut()
        .find(|m| m.pseudonym_key == member_pseudonym)
        .ok_or_else(|| TransportError::DhtError {
            reason: format!("member {member_pseudonym} not found"),
        })?;

    if !member.role_ids.contains(&role_id) {
        member.role_ids.push(role_id);
    }

    dht.registry()
        .write_member_index(registry_key, &members)
        .await?;

    info!(member = member_pseudonym, role_id, "role assigned");
    Ok(())
}

/// Remove a role from a member.
pub async fn unassign_role(
    node: &TransportNode,
    registry_key: &str,
    member_pseudonym: &str,
    role_id: u32,
) -> Result<()> {
    let dht = node.dht()?;
    let mut members = dht.registry().read_member_index(registry_key).await?;

    let member = members
        .iter_mut()
        .find(|m| m.pseudonym_key == member_pseudonym)
        .ok_or_else(|| TransportError::DhtError {
            reason: format!("member {member_pseudonym} not found"),
        })?;

    member.role_ids.retain(|&id| id != role_id);

    dht.registry()
        .write_member_index(registry_key, &members)
        .await?;

    info!(member = member_pseudonym, role_id, "role unassigned");
    Ok(())
}
