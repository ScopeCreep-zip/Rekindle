//! `rekindle channel create` — create a new channel in a community.

use anyhow::Context;

use rekindle_transport::operations::channel_admin;
use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Create a new channel.
pub async fn cmd_create(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    name: &str,
    kind: &str,
    category: Option<&str>,
    topic: Option<&str>,
    slowmode: u32,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let name = helpers::validate_name(name, "Channel")?;

    let entry = channel_admin::create_channel(
        handle.node(),
        &membership.governance_key,
        &name,
        kind,
        category,
        topic,
        slowmode,
    )
    .await
    .context("failed to create channel")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "created",
                "channel_id": entry.id,
                "name": entry.name,
                "kind": format!("{:?}", entry.kind),
                "community": membership.community_name,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "Channel #{} created in '{}'.",
            entry.name, membership.community_name
        ))
    }
}
