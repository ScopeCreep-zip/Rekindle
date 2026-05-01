//! `rekindle channel history` — read channel message history.

use anyhow::Context;

use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Read and display channel message history.
///
/// Uses the QueryEngine to read the channel SMPL log, decrypt messages
/// with cached MEKs, and resolve author pseudonyms to display names.
/// Messages with missing MEK generations show `[encrypted]` placeholder.
pub async fn cmd_history(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    channel_ref: &str,
    limit: usize,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let channel_id = helpers::resolve_channel_id(channel_ref);

    let query = handle
        .node()
        .query(handle.mek_cache.clone())
        .map_err(|e| anyhow::anyhow!("query engine: {e}"))?;

    // Use channel_id as log key for now — full resolution will look up
    // the log_key from the governance channel list
    let channel_log_key = &channel_id;

    let messages = query
        .channel_history(
            &membership.governance_key,
            &channel_id,
            channel_log_key,
            &membership.registry_key,
            limit,
        )
        .await
        .context("failed to read channel history")?;

    if mode.is_structured() {
        return format::print_structured(&messages, mode);
    }

    if messages.is_empty() {
        return format::print_text(&format!(
            "No messages in #{channel_ref} (community: {}).",
            membership.community_name
        ));
    }

    format::print_text(&format!(
        "#{channel_ref} in '{}' — {} messages",
        membership.community_name,
        messages.len()
    ))?;
    format::print_text("")?;

    // Render messages with grouping
    let mut prev_author: Option<String> = None;
    let mut prev_timestamp: Option<u64> = None;

    for msg in &messages {
        let same_author = prev_author.as_deref() == Some(&msg.author_pseudonym);
        let close_in_time = prev_timestamp
            .is_some_and(|pt| msg.timestamp.saturating_sub(pt) < 7 * 60 * 1000);
        let compact = same_author && close_in_time;

        if !compact {
            // Full header: author + timestamp
            let time = helpers::format_time_short(msg.timestamp);
            let author = helpers::sanitize_for_display(&msg.author_display_name);
            format::print_text(&format!("  {author}  [{time}]"))?;
        }

        // Message body
        let body = helpers::sanitize_for_display(&msg.body);
        for line in body.lines() {
            format::print_text(&format!("    {line}"))?;
        }

        // Encrypted indicator
        if msg.is_encrypted {
            if let Some(gen) = msg.needs_mek {
                format::print_text(&format!(
                    "    [encrypted — request MEK gen {gen}: rekindle key mek request -c \"{}\" -C \"{channel_ref}\"]",
                    membership.community_name
                ))?;
            }
        }

        // Reply indicator
        if let Some(reply_seq) = msg.reply_to_sequence {
            format::print_text(&format!("    (reply to message #{reply_seq})"))?;
        }

        prev_author = Some(msg.author_pseudonym.clone());
        prev_timestamp = Some(msg.timestamp);
    }

    Ok(())
}
