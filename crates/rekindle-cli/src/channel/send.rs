//! `rekindle channel send` — send a message to a channel.

use anyhow::Context;

use rekindle_transport::operations::channel;
use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Send a message to a community channel.
///
/// Steps:
/// 1. Resolve community and channel
/// 2. Call transport `send_message` operation (MEK encrypt + SMPL write)
/// 3. Display message ID and confirmation
pub async fn cmd_send(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    channel_ref: &str,
    message: &str,
    reply_to: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let channel_id = helpers::resolve_channel_id(channel_ref);

    if message.trim().is_empty() {
        anyhow::bail!("message body cannot be empty");
    }

    if message.len() > 2000 {
        anyhow::bail!(
            "message too long ({} chars, max 2000)",
            message.len()
        );
    }

    // Sanitize but preserve the message for sending — sanitization is
    // for display, not for the wire format. The recipient sanitizes on
    // their end before rendering.
    let _sanitized_preview = helpers::sanitize_for_display(message);

    let reply_to_seq = reply_to
        .map(|r| {
            r.parse::<u64>()
                .map_err(|_| anyhow::anyhow!("invalid reply-to sequence number: '{r}'"))
        })
        .transpose()?;

    // Find the channel log key — for now use the channel_id.
    // Full channel resolution will look up the log_key from the
    // governance channel list.
    let channel_log_key = &channel_id;

    let result = channel::send_message(
        handle.node(),
        membership,
        &channel_id,
        channel_log_key,
        message,
        reply_to_seq,
        &handle.mek_cache,
    )
    .await
    .context("failed to send message")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "sent",
                "message_id": result.message_id,
                "sequence": result.sequence,
                "timestamp": result.timestamp,
                "community": membership.community_name,
                "channel": channel_ref,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "Message sent to #{} in '{}' (id: {})",
            channel_ref,
            membership.community_name,
            helpers::abbreviate_key(&result.message_id),
        ))
    }
}
