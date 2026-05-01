//! `rekindle role` — role CRUD and assignment operations.

use anyhow::Context;

use rekindle_transport::operations::roles;
use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::table;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// List all roles in a community.
pub async fn cmd_role_list(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;

    let role_list = roles::list_roles(handle.node(), &membership.governance_key)
        .await
        .context("failed to list roles")?;

    if mode.is_structured() {
        return format::print_structured(&role_list, mode);
    }

    if role_list.is_empty() {
        return format::print_text("No roles defined.");
    }

    let headers = &["ID", "Name", "Position", "Color", "Permissions"];
    let rows: Vec<Vec<String>> = role_list
        .iter()
        .map(|r| {
            vec![
                r.id.to_string(),
                r.name.clone(),
                r.position.to_string(),
                format!("#{:06X}", r.color),
                format!("0x{:X}", r.permissions),
            ]
        })
        .collect();

    table::print_table(headers, &rows, mode)
}

/// Create a new role.
pub async fn cmd_role_create(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    name: &str,
    permissions: Option<&str>,
    color: Option<&str>,
    position: Option<u32>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let name = helpers::validate_name(name, "Role")?;

    let perms = parse_permissions(permissions)?;
    let color_val = parse_color(color)?;
    #[allow(clippy::cast_possible_wrap)]
    let pos = position.unwrap_or(0) as i32;

    let role = roles::create_role(
        handle.node(),
        &membership.governance_key,
        &name,
        perms,
        color_val,
        pos,
    )
    .await
    .context("failed to create role")?;

    if mode.is_structured() {
        format::print_structured(&role, mode)
    } else {
        format::print_text(&format!(
            "Role '{}' created (id: {}).",
            role.name, role.id
        ))
    }
}

/// Update an existing role.
pub async fn cmd_role_update(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    role_id: &str,
    name: Option<&str>,
    permissions: Option<&str>,
    color: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let rid: u32 = role_id
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid role ID: '{role_id}'"))?;

    let perms = permissions.map(|p| parse_permissions(Some(p))).transpose()?;
    let color_val = color.map(|c| parse_color(Some(c))).transpose()?;

    let role = roles::update_role(
        handle.node(),
        &membership.governance_key,
        rid,
        name,
        perms,
        color_val,
    )
    .await
    .context("failed to update role")?;

    if mode.is_structured() {
        format::print_structured(&role, mode)
    } else {
        format::print_text(&format!("Role '{}' updated.", role.name))
    }
}

/// Delete a role.
pub async fn cmd_role_delete(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    role_id: &str,
    skip_confirm: bool,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let rid: u32 = role_id
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid role ID: '{role_id}'"))?;

    if !skip_confirm {
        let confirmed = helpers::confirm(&format!("Delete role {rid}?"))?;
        if !confirmed {
            return format::print_text("Cancelled.");
        }
    }

    roles::delete_role(handle.node(), &membership.governance_key, rid)
        .await
        .context("failed to delete role")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({"status": "deleted", "role_id": rid}),
            mode,
        )
    } else {
        format::print_text(&format!("Role {rid} deleted."))
    }
}

/// Assign a role to a member.
pub async fn cmd_role_assign(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    member: &str,
    role_id: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let rid: u32 = role_id
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid role ID: '{role_id}'"))?;

    roles::assign_role(handle.node(), &membership.registry_key, member, rid)
        .await
        .context("failed to assign role")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({"status": "assigned", "member": member, "role_id": rid}),
            mode,
        )
    } else {
        format::print_text(&format!("Role {rid} assigned to {member}."))
    }
}

/// Remove a role from a member.
pub async fn cmd_role_unassign(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    member: &str,
    role_id: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let rid: u32 = role_id
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid role ID: '{role_id}'"))?;

    roles::unassign_role(handle.node(), &membership.registry_key, member, rid)
        .await
        .context("failed to unassign role")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({"status": "unassigned", "member": member, "role_id": rid}),
            mode,
        )
    } else {
        format::print_text(&format!("Role {rid} removed from {member}."))
    }
}

/// Parse a permissions string — decimal or hex (0x prefix).
fn parse_permissions(input: Option<&str>) -> anyhow::Result<u64> {
    let Some(s) = input else { return Ok(0) };
    if let Some(hex) = s.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)
            .map_err(|_| anyhow::anyhow!("invalid hex permissions: '{s}'"))
    } else {
        s.parse()
            .map_err(|_| anyhow::anyhow!("invalid permissions: '{s}' (use decimal or 0x hex)"))
    }
}

/// Parse a color string — hex without # prefix (e.g., "FF5733").
fn parse_color(input: Option<&str>) -> anyhow::Result<u32> {
    let Some(s) = input else { return Ok(0) };
    let hex = s.strip_prefix('#').unwrap_or(s);
    u32::from_str_radix(hex, 16)
        .map_err(|_| anyhow::anyhow!("invalid color hex: '{s}' (expected e.g., FF5733)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_permissions_decimal() {
        assert_eq!(parse_permissions(Some("42")).unwrap(), 42);
    }

    #[test]
    fn parse_permissions_hex() {
        assert_eq!(parse_permissions(Some("0xFF")).unwrap(), 255);
    }

    #[test]
    fn parse_permissions_none() {
        assert_eq!(parse_permissions(None).unwrap(), 0);
    }

    #[test]
    fn parse_color_hex() {
        assert_eq!(parse_color(Some("FF5733")).unwrap(), 0xFF5733);
    }

    #[test]
    fn parse_color_with_hash() {
        assert_eq!(parse_color(Some("#FF5733")).unwrap(), 0xFF5733);
    }

    #[test]
    fn parse_color_none() {
        assert_eq!(parse_color(None).unwrap(), 0);
    }
}
