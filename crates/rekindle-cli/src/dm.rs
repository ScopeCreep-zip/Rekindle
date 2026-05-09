//! DM commands: send, inbox, watch, read.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::cli::DmCmd;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::DaemonClient;

pub async fn dispatch(cmd: &DmCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        DmCmd::Send { friend, message, .. } => {
            let value = client.request_ok(IpcRequest::DmSend {
                peer_key: friend.clone(),
                body: message.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        DmCmd::Inbox { limit, since, .. } => {
            #[allow(clippy::cast_possible_truncation)]
            let value = client.request_ok(IpcRequest::DmInbox {
                limit: *limit as u32,
            }).await?;
            // Apply --since client-side filter if specified
            if let Some(ref since_str) = since {
                let since_ms = parse_since_timestamp(since_str)?;
                let filtered = filter_dm_threads_since(&value, since_ms);
                return format::print_structured(&filtered, mode);
            }
            format::print_structured(&value, mode)
        }
        DmCmd::Watch { .. } => {
            // Streaming path is intercepted in main.rs before dispatch.
            // This arm only fires if the event receiver was already taken.
            anyhow::bail!("dm watch: event receiver unavailable — is another streaming command already running?")
        }
        DmCmd::Read { conversation_id, limit, .. } => {
            let _ = conversation_id;
            #[allow(clippy::cast_possible_truncation)]
            let value = client.request_ok(IpcRequest::DmInbox {
                limit: *limit as u32,
            }).await?;
            format::print_structured(&value, mode)
        }
    }
}

/// Parse a --since value: epoch ms, or ISO 8601 date (YYYY-MM-DD).
fn parse_since_timestamp(s: &str) -> anyhow::Result<u64> {
    // Try epoch ms first
    if let Ok(ms) = s.parse::<u64>() {
        return Ok(ms);
    }
    // Try YYYY-MM-DD
    if let Some((y, rest)) = s.split_once('-') {
        if let Some((m, d)) = rest.split_once('-') {
            if let (Ok(year), Ok(month), Ok(day)) = (y.parse::<u32>(), m.parse::<u32>(), d.parse::<u32>()) {
                // Approximate conversion — ignores leap years and variable month
                // lengths. Off by at most ~3 days for any given date. Acceptable
                // for a CLI filter where precision to the day is sufficient.
                let days = u64::from(year.saturating_sub(1970)) * 365 + u64::from(month.saturating_sub(1)) * 30 + u64::from(day);
                return Ok(days * 86_400_000);
            }
        }
    }
    anyhow::bail!("--since: expected epoch ms or YYYY-MM-DD, got '{s}'")
}

/// Filter DM thread JSON by last_message_at >= since_ms.
fn filter_dm_threads_since(value: &serde_json::Value, since_ms: u64) -> serde_json::Value {
    match value.as_array() {
        Some(arr) => {
            let filtered: Vec<&serde_json::Value> = arr.iter()
                .filter(|t| {
                    t.get("last_message_at")
                        .and_then(serde_json::Value::as_u64)
                        .is_some_and(|ts| ts >= since_ms)
                })
                .collect();
            serde_json::json!(filtered)
        }
        None => value.clone(),
    }
}

/// Streaming DM watch — subscribe to events and print JSONL.
#[allow(clippy::print_stdout)]
pub async fn watch_streaming(
    client: &DaemonClient,
    event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<rekindle_types::subscription_events::SubscriptionEvent>,
    friend_filter: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    use rekindle_types::subscription_events::{SubscriptionEvent, ChannelMessageEvent};

    client.subscribe_all().await?;

    loop {
        match event_rx.recv().await {
            Some(SubscriptionEvent::ChannelMessage(
                ChannelMessageEvent::DirectMessageReceived {
                    ref peer_key, timestamp, ref sender_name, ref body, is_self,
                }
            )) => {
                if let Some(filter) = friend_filter {
                    let name_match = sender_name.as_deref().is_some_and(|n| n.contains(filter));
                    if !peer_key.contains(filter) && !name_match {
                        continue;
                    }
                }
                let obj = serde_json::json!({
                    "type": "dm",
                    "peer_key": peer_key,
                    "sender_name": sender_name,
                    "timestamp": timestamp,
                    "body": body,
                    "is_self": is_self,
                });
                format::print_structured(&obj, mode)?;
            }
            None => break,
            _ => {}
        }
    }
    Ok(())
}
