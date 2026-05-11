//! Channel commands: list, create, delete, update, send, history, watch, edit, delete, pin, unpin, typing.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::v2::cli::ChannelCmd;
use crate::v2::helpers;
use crate::v2::output::{format, table};
use crate::v2::output::OutputMode;
use crate::v2::transport::DaemonClient;

pub async fn dispatch(cmd: &ChannelCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        ChannelCmd::List { community, .. } => {
            let value = client.request_ok(IpcRequest::ChannelList { community: community.clone() }).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            let rows = value.as_array().map(|arr| {
                arr.iter().map(|ch| vec![
                    ch.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                    ch.get("kind").and_then(|v| v.as_str()).unwrap_or("text").to_string(),
                    ch.get("topic").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                ]).collect::<Vec<_>>()
            }).unwrap_or_default();
            table::print_table(&["Name", "Kind", "Topic"], &rows, mode)
        }
        ChannelCmd::Create { community, name, kind, category, topic, slowmode } => {
            let validated_name = helpers::validate_name(name, "Channel")?;
            let value = client.request_ok(IpcRequest::ChannelCreate {
                community: community.clone(),
                name: validated_name,
                kind: kind.clone(),
                category: category.clone(),
                topic: topic.clone(),
                slowmode_seconds: slowmode.unwrap_or(0),
            }).await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::Delete { community, channel, yes } => {
            if !yes {
                let confirmed = helpers::confirm(&format!("Delete channel '{channel}'?"))?;
                if !confirmed { return format::print_text("Cancelled."); }
            }
            let channel_id = helpers::resolve_channel_id(channel);
            let value = client.request_ok(IpcRequest::ChannelDelete {
                community: community.clone(),
                channel_id,
            }).await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::Update { community, channel, name, topic, slowmode } => {
            let channel_id = helpers::resolve_channel_id(channel);
            let validated_name = name.as_ref().map(|n| helpers::validate_name(n, "Channel")).transpose()?;
            let value = client.request_ok(IpcRequest::ChannelUpdate {
                community: community.clone(),
                channel_id,
                name: validated_name,
                topic: topic.clone(),
                slowmode_seconds: *slowmode,
            }).await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::Send { community, channel, message, reply_to } => {
            let reply = reply_to.as_ref().and_then(|s| s.parse::<u64>().ok());
            let value = client.request_ok(IpcRequest::ChannelSend {
                community: community.clone(),
                channel: channel.clone(),
                body: message.clone(),
                reply_to: reply,
            }).await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::History { community, channel, limit, .. } => {
            #[allow(clippy::cast_possible_truncation)]
            let value = client.request_ok(IpcRequest::ChannelHistory {
                community: community.clone(),
                channel: channel.clone(),
                limit: *limit as u32,
            }).await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::Watch { .. } => {
            anyhow::bail!("channel watch: use the streaming path in main dispatch")
        }
        ChannelCmd::Edit { community, channel, message_id, new_body } => {
            let value = client.request_ok(IpcRequest::MessageEdit {
                community: community.clone(),
                channel: channel.clone(),
                message_id: message_id.clone(),
                new_body: new_body.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::MessageDelete { community, channel, message_id } => {
            let value = client.request_ok(IpcRequest::MessageDelete {
                community: community.clone(),
                channel: channel.clone(),
                message_id: message_id.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::Pin { community, channel, message_id } => {
            let value = client.request_ok(IpcRequest::PinAdd {
                community: community.clone(),
                channel: channel.clone(),
                message_id: message_id.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::Unpin { community, channel, message_id } => {
            let value = client.request_ok(IpcRequest::PinRemove {
                community: community.clone(),
                channel: channel.clone(),
                message_id: message_id.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::Typing { community, channel } => {
            let value = client.request_ok(IpcRequest::ChannelTyping {
                community: community.clone(),
                channel: channel.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
    }
}

/// Streaming channel watch — subscribe to community events and print JSONL.
pub async fn watch_streaming(
    client: &DaemonClient,
    event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<rekindle_types::subscription_events::SubscriptionEvent>,
    community: &str,
    channel: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    use rekindle_types::subscription_events::{SubscriptionEvent, ChannelMessageEvent};

    client.subscribe_scoped(community).await?;

    if !mode.is_structured() {
        format::print_text(&format!("Watching #{channel}  (Ctrl+C to stop)"))?;
    }

    loop {
        match event_rx.recv().await {
            Some(SubscriptionEvent::ChannelMessage(
                ChannelMessageEvent::New {
                    community: ref c, channel: ref ch, ref sender_pseudonym,
                    timestamp, ref body, ref message_id, sequence, is_self, ..
                }
            )) if c == community && ch == channel => {
                let obj = serde_json::json!({
                    "type": "message",
                    "community": c, "channel": ch,
                    "message_id": message_id, "sender": sender_pseudonym,
                    "sequence": sequence, "timestamp": timestamp,
                    "body": body, "is_self": is_self,
                });
                format::print_structured(&obj, mode)?;
            }
            Some(SubscriptionEvent::ChannelMessage(
                ChannelMessageEvent::Edited {
                    community: ref c, channel: ref ch, ref message_id,
                    edited_at, ref body,
                }
            )) if c == community && ch == channel => {
                let obj = serde_json::json!({
                    "type": "edited",
                    "community": c, "channel": ch,
                    "message_id": message_id, "edited_at": edited_at, "body": body,
                });
                format::print_structured(&obj, mode)?;
            }
            Some(SubscriptionEvent::ChannelMessage(
                ChannelMessageEvent::Deleted {
                    community: ref c, channel: ref ch, ref message_id,
                }
            )) if c == community && ch == channel => {
                let obj = serde_json::json!({
                    "type": "deleted",
                    "community": c, "channel": ch, "message_id": message_id,
                });
                format::print_structured(&obj, mode)?;
            }
            None => break,
            _ => {}
        }
    }
    Ok(())
}
