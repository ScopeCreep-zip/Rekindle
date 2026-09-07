//! Role definition operations (create / update / delete)
//!
//! Assign / unassign are **not** here: granting a role once meant
//! rewriting the target's row in the shared registry member index, which
//! `o_cnt: 0` gives nobody a writer credential for. v2.0 writes
//! `RoleAssignment` / `RoleUnassignment` governance entries via
//! `rekindle_governance_runtime::roles`, which both shells now call.
//!
//! Typed reads/writes via `dht/governance.rs`.

use tracing::info;

use crate::broadcast::node::TransportNode;
use crate::error::{Result, TransportError};
use crate::payload::dht_types::RoleEntry;

pub async fn list_roles(node: &TransportNode, governance_key: &str) -> Result<Vec<RoleEntry>> {
    node.dht()?.governance().read_roles(governance_key).await
}

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
    let role =
        roles
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

pub async fn delete_role(node: &TransportNode, governance_key: &str, role_id: u32) -> Result<()> {
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
