//! A community's channels.

use rekindle_ipc::protocol::CommunityRequest;

use super::{
    channel, channel_kind, community, hex16, message_body, name, opt, page, slowmode, text,
    Checked, MAX_TOPIC_LEN,
};

/// A channel message id: a UUID, in either of the forms peers write.
const MAX_MESSAGE_ID_LEN: usize = 64;

/// A community's channels: their names, kinds, topics and bodies.
pub(super) fn check(request: &CommunityRequest) -> Checked {
    match request {
        CommunityRequest::ChannelCreate {
            community: c,
            name: channel_name,
            kind,
            category,
            topic,
            slowmode_seconds,
            parent_voice_channel,
        } => {
            community(c)?;
            opt(parent_voice_channel.as_deref(), |p| {
                hex16(p, "parent voice channel")
            })?;
            name(channel_name, "channel")?;
            channel_kind(kind)?;
            opt(category.as_deref(), |n| name(n, "category"))?;
            opt(topic.as_deref(), |t| text(t, MAX_TOPIC_LEN, "topic"))?;
            slowmode(*slowmode_seconds)
        }
        CommunityRequest::ChannelDelete {
            community: c,
            channel_id,
        } => {
            community(c)?;
            channel(channel_id)
        }
        CommunityRequest::ChannelUpdate {
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
        CommunityRequest::ChannelSend {
            community: c,
            channel: ch,
            body,
            reply_to,
        } => {
            community(c)?;
            channel(ch)?;
            opt(reply_to.as_deref(), |r| {
                text(r, MAX_MESSAGE_ID_LEN, "reply id")
            })?;
            message_body(body)
        }
        CommunityRequest::ChannelHistory {
            community: c,
            channel: ch,
            limit,
        } => {
            community(c)?;
            channel(ch)?;
            page(*limit)
        }
        CommunityRequest::ChannelMessageEdit {
            channel: ch,
            message_id,
            body,
        } => {
            channel(ch)?;
            text(message_id, MAX_MESSAGE_ID_LEN, "message id")?;
            message_body(body)
        }
        CommunityRequest::ChannelMessageSend {
            channel: ch,
            body,
            reply_to,
        } => {
            channel(ch)?;
            opt(reply_to.as_deref(), |r| {
                text(r, MAX_MESSAGE_ID_LEN, "reply id")
            })?;
            message_body(body)
        }
        CommunityRequest::ChannelMessageDelete {
            channel: ch,
            message_id,
        } => {
            channel(ch)?;
            text(message_id, MAX_MESSAGE_ID_LEN, "message id")
        }
        other => super::channel_admin::check(other),
    }
}
