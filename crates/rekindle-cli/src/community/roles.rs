//! Render community roles and the Host's permission catalogue.

use rekindle_client::DaemonClient;
use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::display::RoleDisplay;
use rekindle_types::dto::permission_catalogue::PermissionCatalogue;

use crate::helpers;
use crate::output::{format, table, OutputMode};

pub(super) async fn permissions(client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    let value = helpers::send(client, IpcRequest::PermissionCatalogue).await?;
    let catalogue: PermissionCatalogue = serde_json::from_value(value)?;
    if mode.is_structured() {
        return format::print_structured(&catalogue, mode);
    }
    for group in catalogue.groups {
        format::print_text(&group.name)?;
        for entry in group.permissions {
            format::print_text(&format!("  {} ({})", entry.label, entry.key))?;
        }
    }
    Ok(())
}

pub(crate) fn print_roles(roles: &[RoleDisplay], mode: OutputMode) -> anyhow::Result<()> {
    table::print_table(
        &["Role", "Position", "Permissions", "ID"],
        &role_rows(roles),
        mode,
    )
}

fn role_rows(roles: &[RoleDisplay]) -> Vec<Vec<String>> {
    let mut sorted: Vec<_> = roles.iter().collect();
    sorted.sort_by(|a, b| b.position.cmp(&a.position).then_with(|| a.id.cmp(&b.id)));
    sorted
        .into_iter()
        .map(|r| {
            vec![
                r.name.clone(),
                r.position_label.clone(),
                format!("{:#x}", r.permissions),
                r.id.clone(),
            ]
        })
        .collect()
}
