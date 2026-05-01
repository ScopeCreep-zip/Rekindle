//! `rekindle community list` — list joined communities.

use rekindle_transport::Session;

use crate::output::format;
use crate::output::table;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// List all communities the user has joined.
///
/// Reads community metadata from the session state. For live data
/// (member count, channel count), queries the transport's QueryEngine.
pub async fn cmd_list(
    handle: &TransportHandle,
    session: &Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    if session.communities.is_empty() {
        if mode.is_structured() {
            return format::print_structured(&serde_json::json!({"communities": []}), mode);
        }
        format::print_text("No communities joined.")?;
        return format::print_text("  join one: rekindle community join --invite <code>");
    }

    // Try to get live data from the query engine
    let memberships: Vec<_> = session.communities.values().collect();

    let query = handle
        .node()
        .query(handle.mek_cache.clone())
        .map_err(|e| anyhow::anyhow!("query engine: {e}"))?;

    let communities = query
        .list_communities(
            &memberships
                .iter()
                .map(|m| (*m).clone())
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "failed to query live community data, using cached");
            // Fall back to session data
            memberships
                .iter()
                .map(|m| rekindle_transport::CommunityOverview {
                    governance_key: m.governance_key.clone(),
                    name: m.community_name.clone(),
                    description: String::new(),
                    member_count: 0,
                    channel_count: 0,
                    our_pseudonym: m.pseudonym_key.clone(),
                })
                .collect()
        });

    if mode.is_structured() {
        return format::print_structured(&communities, mode);
    }

    let headers = &["Name", "Members", "Channels", "Governance Key"];
    let rows: Vec<Vec<String>> = communities
        .iter()
        .map(|c| {
            vec![
                c.name.clone(),
                c.member_count.to_string(),
                c.channel_count.to_string(),
                crate::helpers::abbreviate_key(&c.governance_key),
            ]
        })
        .collect();

    table::print_table(headers, &rows, mode)
}
