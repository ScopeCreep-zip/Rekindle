//! `rekindle channel watch` — live-stream channel messages.

use std::io::Write;

use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::{CliEvent, TransportHandle};

/// Live-stream messages from a channel.
///
/// Subscribes to the transport event channel and filters for gossip
/// `MessageNotification` events matching the target community and channel.
/// Runs until interrupted (Ctrl+C).
///
/// In text mode: renders messages with author + timestamp.
/// In JSONL mode: emits one JSON object per message.
#[allow(clippy::print_stderr)]
pub async fn cmd_watch(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    channel_ref: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let channel_id = helpers::resolve_channel_id(channel_ref);
    let community_gov_key = membership.governance_key.clone();
    let community_name = membership.community_name.clone();

    if !mode.is_structured() {
        format::print_text(&format!(
            "Watching #{channel_ref} in '{community_name}'... (Ctrl+C to stop)"
        ))?;
        format::print_text("")?;
    }

    // Drain the event channel for matching gossip events.
    // The transport bridge pushes CliEvent::Gossip for every gossip broadcast.
    let mut event_rx = handle.event_rx.lock().await;

    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                match event {
                    CliEvent::Gossip {
                        community_id,
                        sender_pseudonym: _,
                        payload,
                        lamport_ts,
                    } if community_id == community_gov_key => {
                        // Check if this is a message notification for our channel
                        if let rekindle_transport::payload::gossip::GossipPayload::MessageNotification {
                            channel_id: msg_channel_id,
                            message_id,
                            author_pseudonym,
                            timestamp,
                            ..
                        } = &payload
                        {
                            if *msg_channel_id == channel_id {
                                render_message_event(
                                    &community_name,
                                    channel_ref,
                                    author_pseudonym,
                                    message_id,
                                    *timestamp,
                                    lamport_ts,
                                    mode,
                                )?;
                            }
                        }

                        // Also show typing indicators
                        if let rekindle_transport::payload::gossip::GossipPayload::TypingIndicator {
                            channel_id: typing_channel_id,
                            pseudonym_key,
                        } = &payload
                        {
                            if *typing_channel_id == channel_id && !mode.is_structured() {
                                let name = helpers::abbreviate_key(pseudonym_key);
                                // Typing indicator on stderr to avoid corrupting JSONL output
                                eprintln!("  {name} is typing...");
                            }
                        }
                    }
                    CliEvent::Transport(rekindle_transport::TransportEvent::LocalRoutesDied { count }) => {
                        if !mode.is_structured() {
                            eprintln!("  [WARN] {count} local routes died — messages may not be received");
                        }
                    }
                    CliEvent::Transport(rekindle_transport::TransportEvent::RemoteRoutesDied { ref peer_keys }) => {
                        if !mode.is_structured() {
                            eprintln!("  [WARN] {} peer routes died — some peers unreachable", peer_keys.len());
                        }
                    }
                    CliEvent::ValueChange { ref record_key, ref subkeys } => {
                        // DHT value changed — a channel log subkey was updated
                        // This is an alternative notification path to gossip for
                        // channels with DHT watches enabled.
                        if !mode.is_structured() {
                            tracing::debug!(
                                record = record_key,
                                subkeys = ?subkeys,
                                "DHT value changed (may contain new messages)"
                            );
                        }
                    }
                    _ => {
                        // Other events not relevant to channel watch
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                if !mode.is_structured() {
                    format::print_text("\nStopped watching.")?;
                }
                break;
            }
        }
    }

    Ok(())
}

/// Render a single message notification event.
fn render_message_event(
    community_name: &str,
    channel_ref: &str,
    author_pseudonym: &str,
    message_id: &str,
    timestamp: u64,
    lamport_ts: u64,
    mode: OutputMode,
) -> anyhow::Result<()> {
    if mode.is_structured() {
        return format::print_jsonl(&serde_json::json!({
            "event": "message",
            "community": community_name,
            "channel": channel_ref,
            "author": author_pseudonym,
            "message_id": message_id,
            "timestamp": timestamp,
            "lamport_ts": lamport_ts,
        }));
    }

    let time = helpers::format_time_short(timestamp);
    let author = helpers::sanitize_for_display(&helpers::abbreviate_key(author_pseudonym));

    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "  {author}  [{time}]  (msg: {})", helpers::abbreviate_key(message_id))?;

    // Note: The actual message body is in the DHT channel log, not in the
    // gossip notification (which only carries metadata). To show the body,
    // we'd need to read and decrypt the channel log entry. This will be
    // added in M3 when the full watch-with-decrypt pipeline is wired.
    writeln!(stdout, "    [new message — read with: rekindle channel history -c \"{community_name}\" -C \"{channel_ref}\" --limit 1]")?;

    Ok(())
}
