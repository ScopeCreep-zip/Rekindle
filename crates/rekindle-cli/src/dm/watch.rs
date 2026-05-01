//! `rekindle dm watch` — live-stream incoming DMs.

use std::io::Write;

use rekindle_transport::Session;
use rekindle_transport::payload::dm::DmPayload;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::{CliEvent, TransportHandle};

/// Live-stream incoming DMs.
///
/// Subscribes to the transport event channel and filters for DM events.
/// Optionally filters to a single friend. Runs until Ctrl+C.
#[allow(clippy::print_stderr)]
pub async fn cmd_watch(
    handle: &TransportHandle,
    _session: &Session,
    friend_filter: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    if !mode.is_structured() {
        let filter_msg = friend_filter
            .map(|f| format!(" from {}", helpers::abbreviate_key(f)))
            .unwrap_or_default();
        format::print_text(&format!(
            "Watching DMs{filter_msg}... (Ctrl+C to stop)"
        ))?;
        format::print_text("")?;
    }

    let mut event_rx = handle.event_rx.lock().await;

    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                if let CliEvent::Dm {
                    sender_key,
                    sender_name,
                    payload,
                    timestamp,
                } = event
                {
                    // Apply friend filter
                    if let Some(filter) = friend_filter {
                        if !sender_key.starts_with(filter)
                            && !sender_name.to_lowercase().contains(&filter.to_lowercase())
                        {
                            continue;
                        }
                    }

                    render_dm_event(
                        &sender_key,
                        &sender_name,
                        &payload,
                        timestamp,
                        mode,
                    )?;
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

/// Render a single incoming DM event.
#[allow(clippy::print_stderr)]
fn render_dm_event(
    sender_key: &str,
    sender_name: &str,
    payload: &DmPayload,
    timestamp: u64,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match payload {
        DmPayload::DirectMessage { body, reply_to } => {
            let body_str = String::from_utf8_lossy(body);
            let body_sanitized = helpers::sanitize_for_display(&body_str);

            if mode.is_structured() {
                return format::print_jsonl(&serde_json::json!({
                    "event": "dm",
                    "sender_key": sender_key,
                    "sender_name": sender_name,
                    "body": body_sanitized,
                    "timestamp": timestamp,
                    "has_reply": reply_to.is_some(),
                }));
            }

            let name = helpers::sanitize_for_display(sender_name);
            let time = helpers::format_time_short(timestamp);
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "  [{time}] {name}: {body_sanitized}")?;
        }
        DmPayload::Typing { typing } => {
            if !mode.is_structured() && *typing {
                let name = helpers::sanitize_for_display(sender_name);
                eprintln!("  {name} is typing...");
            }
        }
        DmPayload::FriendRequest {
            display_name,
            message,
            ..
        } => {
            let name = helpers::sanitize_for_display(display_name);
            let msg = helpers::sanitize_for_display(message);

            if mode.is_structured() {
                return format::print_jsonl(&serde_json::json!({
                    "event": "friend_request",
                    "sender_key": sender_key,
                    "display_name": name,
                    "message": msg,
                    "timestamp": timestamp,
                }));
            }

            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "  [FRIEND REQUEST] {name}: \"{msg}\"")?;
            writeln!(
                stdout,
                "    accept: rekindle friend accept --request-id \"{sender_key}\""
            )?;
        }
        DmPayload::FriendAccept { .. } => {
            if mode.is_structured() {
                return format::print_jsonl(&serde_json::json!({
                    "event": "friend_accepted",
                    "sender_key": sender_key,
                    "timestamp": timestamp,
                }));
            }
            let name = helpers::sanitize_for_display(sender_name);
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "  [ACCEPTED] {name} accepted your friend request.")?;
        }
        DmPayload::FriendReject => {
            if mode.is_structured() {
                return format::print_jsonl(&serde_json::json!({
                    "event": "friend_rejected",
                    "sender_key": sender_key,
                    "timestamp": timestamp,
                }));
            }
            let name = helpers::sanitize_for_display(sender_name);
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "  [REJECTED] {name} rejected your friend request.")?;
        }
        DmPayload::Unfriend => {
            if mode.is_structured() {
                return format::print_jsonl(&serde_json::json!({
                    "event": "unfriended",
                    "sender_key": sender_key,
                    "timestamp": timestamp,
                }));
            }
            let name = helpers::sanitize_for_display(sender_name);
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "  [UNFRIENDED] {name} removed you as a friend.")?;
        }
        DmPayload::PresenceUpdate { status, game_info } => {
            if mode.is_structured() {
                return format::print_jsonl(&serde_json::json!({
                    "event": "presence",
                    "sender_key": sender_key,
                    "status": status,
                    "game": game_info.as_ref().map(|g| &g.game_name),
                    "timestamp": timestamp,
                }));
            }
            // Presence updates are high-frequency — only show in structured mode
            // to avoid flooding the terminal.
        }
        _ => {
            // Other DM types (ProfileKeyRotated, FriendRequestAck, UnfriendAck)
            // are protocol-level signals, not user-visible messages.
        }
    }

    Ok(())
}
