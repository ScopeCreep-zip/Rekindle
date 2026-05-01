//! `rekindle key inspect` — show crypto state for a community.

use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::table;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Show the cryptographic state for a community: MEK cache, channel count,
/// and general crypto health.
pub fn cmd_inspect(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;

    let mek_snapshot = handle
        .mek_cache
        .read()
        .snapshot(&membership.governance_key);

    let total_meks = mek_snapshot.len();
    let channels_with_meks: std::collections::HashSet<&str> = mek_snapshot
        .iter()
        .map(|e| e.channel_id.as_str())
        .collect();

    if mode.is_structured() {
        return format::print_structured(
            &serde_json::json!({
                "community": membership.community_name,
                "governance_key": membership.governance_key,
                "pseudonym_key": membership.pseudonym_key,
                "total_mek_entries": total_meks,
                "channels_with_meks": channels_with_meks.len(),
                "mek_entries": mek_snapshot,
            }),
            mode,
        );
    }

    format::print_text(&format!(
        "Crypto state for '{}':",
        membership.community_name
    ))?;
    format::print_kv(
        &[
            (
                "Governance:",
                helpers::abbreviate_key(&membership.governance_key),
            ),
            (
                "Pseudonym:",
                helpers::abbreviate_key(&membership.pseudonym_key),
            ),
            ("MEK entries:", total_meks.to_string()),
            (
                "Channels with MEKs:",
                channels_with_meks.len().to_string(),
            ),
        ],
        mode,
    )?;

    if !mek_snapshot.is_empty() {
        format::print_text("")?;
        let headers = &["Channel", "Generation", "Age"];
        let rows: Vec<Vec<String>> = mek_snapshot
            .iter()
            .map(|e| {
                vec![
                    if e.channel_id.is_empty() {
                        "(community-wide)".into()
                    } else {
                        e.channel_id.clone()
                    },
                    e.generation.to_string(),
                    helpers::format_uptime(e.age_secs),
                ]
            })
            .collect();
        table::print_table(headers, &rows, mode)?;
    }

    Ok(())
}
