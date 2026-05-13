//! DM commands: send, inbox, read, watch, typing.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::v2::cli::DmCmd;
use crate::v2::helpers;
use crate::v2::output::format;
use crate::v2::output::OutputMode;
use crate::v2::transport::DaemonClient;

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
            if let Some(ref since_str) = since {
                let since_ms = helpers::parse_since_timestamp(since_str)?;
                let filtered = filter_dm_threads_since(&value, since_ms);
                return format::print_structured(&filtered, mode);
            }
            format::print_structured(&value, mode)
        }
        DmCmd::Read { conversation_id, limit, .. } => {
            // Uses the new DmThread IpcRequest for efficient single-conversation load
            #[allow(clippy::cast_possible_truncation)]
            let value = client.request_ok(IpcRequest::DmThread {
                peer_key: conversation_id.clone(),
                limit: *limit as u32,
            }).await?;
            format::print_structured(&value, mode)
        }
        DmCmd::Watch { .. } => {
            anyhow::bail!("dm watch: event receiver unavailable — is another streaming command already running?")
        }
        DmCmd::Typing { friend, typing } => {
            let value = client.request_ok(IpcRequest::DmTyping {
                peer_key: friend.clone(),
                typing: *typing,
            }).await?;
            format::print_structured(&value, mode)
        }
    }
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
    event_rx: &mut tokio::sync::mpsc::Receiver<rekindle_types::subscription_events::SubscriptionEvent>,
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
