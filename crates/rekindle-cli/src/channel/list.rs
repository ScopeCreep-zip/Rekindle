//! `rekindle channel list` — list channels in a community.

use anyhow::Context;

use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// List all channels in a community, optionally grouped by category.
pub async fn cmd_list(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;

    let query = handle
        .node()
        .query(handle.mek_cache.clone())
        .map_err(|e| anyhow::anyhow!("query engine: {e}"))?;

    let channels = query
        .list_channels(&membership.governance_key)
        .await
        .context("failed to list channels")?;

    if mode.is_structured() {
        return format::print_structured(&channels, mode);
    }

    if channels.is_empty() {
        return format::print_text("No channels.");
    }

    // Group by category
    let mut current_category: Option<String> = None;
    let mut sorted = channels.clone();
    sorted.sort_by(|a, b| {
        a.category_id
            .cmp(&b.category_id)
            .then(a.sort_order.cmp(&b.sort_order))
    });

    for ch in &sorted {
        let cat = ch.category_id.as_deref().unwrap_or("(uncategorized)");
        if current_category.as_deref() != Some(cat) {
            current_category = Some(cat.to_string());
            format::print_text(&format!("\n  {cat}"))?;
        }
        let topic = if ch.topic.is_empty() {
            String::new()
        } else {
            format!(" — {}", ch.topic)
        };
        format::print_text(&format!(
            "    #{} [{}]{topic}",
            ch.name, ch.kind
        ))?;
    }

    format::print_text(&format!("\n{} channels total", channels.len()))
}
