//! `rekindle community info` — show detailed community information.

use anyhow::Context;

use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::table;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Display detailed community metadata, channels, roles, and member count.
pub async fn cmd_info(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    verbose: bool,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;

    let query = handle
        .node()
        .query(handle.mek_cache.clone())
        .map_err(|e| anyhow::anyhow!("query engine: {e}"))?;

    let detail = query
        .community_detail(membership)
        .await
        .context("failed to read community details")?;

    if mode.is_structured() {
        return format::print_structured(&detail, mode);
    }

    // Header
    format::print_text(&format!("Community: {}", detail.name))?;
    if !detail.description.is_empty() {
        format::print_text(&format!("  {}", detail.description))?;
    }
    format::print_text("")?;

    // Metadata
    format::print_kv(
        &[
            ("Governance:", helpers::abbreviate_key(&detail.governance_key)),
            ("Owner:", helpers::abbreviate_key(&detail.owner_pseudonym)),
            (
                "Created:",
                helpers::format_timestamp(detail.created_at),
            ),
            ("Members:", detail.member_count.to_string()),
            ("Your pseudonym:", helpers::abbreviate_key(&detail.our_pseudonym)),
            (
                "Your roles:",
                if detail.our_roles.is_empty() {
                    "none".into()
                } else {
                    detail
                        .our_roles
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                },
            ),
        ],
        mode,
    )?;

    // Channels
    format::print_text("")?;
    format::print_text(&format!("Channels ({})", detail.channels.len()))?;

    if verbose {
        let headers = &["Name", "Kind", "Topic", "MEK Gen"];
        let rows: Vec<Vec<String>> = detail
            .channels
            .iter()
            .map(|ch| {
                vec![
                    format!("#{}", ch.name),
                    ch.kind.clone(),
                    if ch.topic.is_empty() {
                        "-".into()
                    } else {
                        ch.topic.clone()
                    },
                    ch.mek_generation.to_string(),
                ]
            })
            .collect();
        table::print_table(headers, &rows, mode)?;
    } else {
        for ch in &detail.channels {
            format::print_text(&format!("  #{} ({})", ch.name, ch.kind))?;
        }
    }

    // Roles
    if !detail.roles.is_empty() {
        format::print_text("")?;
        format::print_text(&format!("Roles ({})", detail.roles.len()))?;

        if verbose {
            let headers = &["ID", "Name", "Position", "Permissions"];
            let rows: Vec<Vec<String>> = detail
                .roles
                .iter()
                .map(|r| {
                    vec![
                        r.id.to_string(),
                        r.name.clone(),
                        r.position.to_string(),
                        format!("0x{:X}", r.permissions),
                    ]
                })
                .collect();
            table::print_table(headers, &rows, mode)?;
        } else {
            for r in &detail.roles {
                format::print_text(&format!("  {} (id: {})", r.name, r.id))?;
            }
        }
    }

    Ok(())
}
