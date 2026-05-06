//! Channel commands.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::cli::ChannelCmd;
use crate::helpers;
use crate::output::{format, table};
use crate::output::OutputMode;
use crate::transport::DaemonClient;

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
        ChannelCmd::Delete { community, channel, .. } => {
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
        ChannelCmd::Watch { community, channel, .. } => {
            #[allow(clippy::cast_possible_truncation)]
            let value = client.request_ok(IpcRequest::ChannelHistory {
                community: community.clone(),
                channel: channel.clone(),
                limit: 50,
            }).await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::Pin { .. } | ChannelCmd::Unpin { .. } => {
            format::print_text("Pin/unpin not yet implemented in daemon")
        }
    }
}
