//! A community's channels.

use rekindle_ipc::protocol::IpcRequest;

use super::{
    channel, channel_kind, community, message_body, name, opt, page, slowmode, text, Checked,
    MAX_TOPIC_LEN,
};

/// A community's channels: their names, kinds, topics and bodies.
pub(super) fn check(request: &IpcRequest) -> Checked {
    match request {
        IpcRequest::ChannelCreate {
            community: c,
            name: channel_name,
            kind,
            category,
            topic,
            slowmode_seconds,
        } => {
            community(c)?;
            name(channel_name, "channel")?;
            channel_kind(kind)?;
            opt(category.as_deref(), |n| name(n, "category"))?;
            opt(topic.as_deref(), |t| text(t, MAX_TOPIC_LEN, "topic"))?;
            slowmode(*slowmode_seconds)
        }
        IpcRequest::ChannelDelete {
            community: c,
            channel_id,
        } => {
            community(c)?;
            channel(channel_id)
        }
        IpcRequest::ChannelUpdate {
            community: c,
            channel_id,
            name: new_name,
            topic,
            slowmode_seconds,
        } => {
            community(c)?;
            channel(channel_id)?;
            opt(new_name.as_deref(), |n| name(n, "channel"))?;
            opt(topic.as_deref(), |t| text(t, MAX_TOPIC_LEN, "topic"))?;
            opt(slowmode_seconds.as_ref(), |s| slowmode(*s))
        }
        IpcRequest::ChannelSend {
            community: c,
            channel: ch,
            body,
            reply_to: _,
        } => {
            community(c)?;
            channel(ch)?;
            message_body(body)
        }
        IpcRequest::ChannelHistory {
            community: c,
            channel: ch,
            limit,
        } => {
            community(c)?;
            channel(ch)?;
            page(*limit)
        }
        other => unreachable!("{} is not a channel request", other.name()),
    }
}
