//! Channel commands.

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};

use crate::cli::ChannelCmd;
use crate::helpers;
use crate::output::OutputMode;
use crate::output::{format, table};
use rekindle_client::DaemonClient;

pub async fn dispatch(
    cmd: &ChannelCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        ChannelCmd::List { community, .. } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::ChannelList {
                    community: community.clone(),
                }),
            )
            .await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            let rows = value
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|ch| {
                            vec![
                                ch.get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("?")
                                    .to_string(),
                                ch.get("kind")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("text")
                                    .to_string(),
                                ch.get("topic")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            ]
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            table::print_table(&["Name", "Kind", "Topic"], &rows, mode)
        }
        ChannelCmd::Create {
            community,
            name,
            kind,
            category,
            topic,
            slowmode,
        } => {
            let validated_name = helpers::validate_name(name, "Channel")?;
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::ChannelCreate {
                    community: community.clone(),
                    name: validated_name,
                    kind: kind.clone(),
                    category: category.clone(),
                    topic: topic.clone(),
                    slowmode_seconds: slowmode.unwrap_or(0),
                    parent_voice_channel: None,
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::Delete { community, channel } => {
            let channel_id = channel.clone();
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::ChannelDelete {
                    community: community.clone(),
                    channel_id,
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::Update {
            community,
            channel,
            name,
            topic,
            slowmode,
        } => {
            let channel_id = channel.clone();
            let validated_name = name
                .as_ref()
                .map(|n| helpers::validate_name(n, "Channel"))
                .transpose()?;
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::ChannelUpdate {
                    community: community.clone(),
                    channel_id,
                    name: validated_name,
                    topic: topic.clone(),
                    slowmode_seconds: *slowmode,
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::Send {
            community,
            channel,
            message,
            reply_to,
        } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::ChannelSend {
                    community: community.clone(),
                    channel: channel.clone(),
                    body: message.clone(),
                    reply_to: reply_to.clone(),
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::History {
            community,
            channel,
            limit,
            ..
        } => {
            let (_, channel_id) = resolve_channel(client, community, channel).await?;
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::ChannelMessages {
                    channel: channel_id,
                    limit: u32::try_from(*limit).unwrap_or(u32::MAX),
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        ChannelCmd::Watch { community, channel } => {
            watch_channel(client, community, channel, mode).await
        }
        ChannelCmd::Pin { .. } | ChannelCmd::Unpin { .. } => {
            Err(crate::identity::unimplemented("channel pin/unpin"))
        }
    }
}

/// The governance key of `community` and the id of its channel `channel`;
/// the user may have given either names or keys.
async fn resolve_channel(
    client: &DaemonClient,
    community: &str,
    channel: &str,
) -> anyhow::Result<(String, String)> {
    let detail: rekindle_types::display::CommunityDetail = serde_json::from_value(
        helpers::send(
            client,
            IpcRequest::Community(CommunityRequest::CommunityInfo {
                governance_key: community.to_owned(),
            }),
        )
        .await?,
    )?;
    let channel_id = detail
        .channels
        .iter()
        .find(|c| c.id == channel || c.name == channel)
        .map(|c| c.id.clone())
        .ok_or_else(|| anyhow::anyhow!("no channel '{channel}' in '{community}'"))?;
    Ok((detail.governance_key, channel_id))
}

/// `channel watch`: stream one channel's messages until Ctrl-C.
async fn watch_channel(
    client: &DaemonClient,
    community: &str,
    channel: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    use rekindle_types::subscription_events::{
        ChannelMessageEvent, EventCategory, SubscriptionEvent, SubscriptionFilter,
    };
    // Events name the community by governance key and the channel by id.
    let (governance_key, channel_id) = resolve_channel(client, community, channel).await?;
    let filter = SubscriptionFilter {
        categories: Some(vec![EventCategory::ChannelMessage]),
        community_scope: Some(governance_key),
    };
    let body = |text: &Option<String>| {
        text.as_deref().map_or_else(
            || "(encrypted — key not yet received)".to_owned(),
            rekindle_utils::text::sanitize_for_display,
        )
    };
    crate::watch::stream(client, vec![filter], mode, |event| match event {
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
            channel,
            sender_pseudonym,
            timestamp,
            body: text,
            ..
        }) if *channel == channel_id => Some(format!(
            "[{}] {}: {}",
            rekindle_client::fmt::format_time_short(*timestamp),
            rekindle_client::fmt::abbreviate_key(sender_pseudonym),
            body(text)
        )),
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Edited {
            channel,
            message_id,
            body: text,
            ..
        }) if *channel == channel_id => Some(format!(
            "(edited {}) {}",
            rekindle_client::fmt::abbreviate_key(message_id),
            body(text)
        )),
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Deleted {
            channel,
            message_id,
            ..
        }) if *channel == channel_id => Some(format!(
            "(deleted {})",
            rekindle_client::fmt::abbreviate_key(message_id)
        )),
        _ => None,
    })
    .await
}
