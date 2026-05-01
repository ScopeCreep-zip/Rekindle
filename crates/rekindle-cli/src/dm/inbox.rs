//! `rekindle dm inbox` and `rekindle dm read` — read DM conversations.

use anyhow::Context;

use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Show DM inbox — recent messages grouped by friend.
///
/// Uses the QueryEngine to read the DM conversation log, group by
/// sender, and resolve display names from the friend list.
pub async fn cmd_inbox(
    handle: &TransportHandle,
    session: &Session,
    friend_filter: Option<&str>,
    limit: usize,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let dm_log_key = session.dm_log_key.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "no DM log key — you may not have received any DMs yet.\n\
             send a DM first: rekindle dm send --friend <name> --message <text>"
        )
    })?;

    let query = handle
        .node()
        .query(handle.mek_cache.clone())
        .map_err(|e| anyhow::anyhow!("query engine: {e}"))?;

    let threads = query
        .dm_inbox(
            dm_log_key,
            &session.identity.friend_list_dht_key,
            limit,
            &session.identity.public_key_hex,
        )
        .await
        .context("failed to read DM inbox")?;

    // Apply friend filter
    let filtered: Vec<_> = if let Some(friend) = friend_filter {
        threads
            .into_iter()
            .filter(|t| {
                t.peer_name.to_lowercase().contains(&friend.to_lowercase())
                    || t.peer_key.starts_with(friend)
            })
            .collect()
    } else {
        threads
    };

    if mode.is_structured() {
        return format::print_structured(&filtered, mode);
    }

    if filtered.is_empty() {
        return format::print_text("No DM conversations.");
    }

    for thread in &filtered {
        let name = helpers::sanitize_for_display(&thread.peer_name);
        let key_short = helpers::abbreviate_key(&thread.peer_key);
        let last_time = helpers::format_timestamp(thread.last_message_at);

        format::print_text(&format!(
            "{name} ({key_short}) — last message: {last_time}"
        ))?;

        for msg in &thread.messages {
            let sender = if msg.is_self { "you" } else { &msg.sender_name };
            let time = helpers::format_time_short(msg.timestamp);
            let body = helpers::sanitize_for_display(&msg.body);
            format::print_text(&format!("  [{time}] {sender}: {body}"))?;
        }
        format::print_text("")?;
    }

    Ok(())
}

/// Read a full conversation with a specific peer.
pub async fn cmd_read(
    handle: &TransportHandle,
    session: &Session,
    conversation_id: &str,
    limit: usize,
    mode: OutputMode,
) -> anyhow::Result<()> {
    // conversation_id is the peer's public key
    // Delegate to inbox with a filter
    cmd_inbox(handle, session, Some(conversation_id), limit, mode).await
}
