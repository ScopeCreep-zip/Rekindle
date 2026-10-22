//! A community's channels.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::protocol::{ChannelMessageRequest, ChannelRequest};

use super::{
    channel, channel_kind, community, hex16, message_body, name, opt, page, slowmode, text,
    Checked, MAX_TOPIC_LEN,
};

/// A channel message id: a UUID, in either of the forms peers write.
const MAX_MESSAGE_ID_LEN: usize = 64;

/// The most messages one bulk delete names (architecture §16.7).
const MAX_BULK_DELETE: usize = 100;

/// A moderator's reason for a delete.
const MAX_DELETE_REASON_LEN: usize = 512;

/// A community's channels: their names, kinds, topics and bodies.
/// `check` for the channel and category requests.
pub(super) fn check_channels(request: &ChannelRequest) -> Checked {
    match request {
        ChannelRequest::ChannelCreate {
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
        ChannelRequest::ChannelDelete {
            community: c,
            channel_id,
        } => {
            community(c)?;
            channel(channel_id)
        }
        ChannelRequest::ChannelUpdate {
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
        other => super::channel_admin::check(other),
    }
}

/// `check` for the channel message and channel key requests.
pub(super) fn check_messages(request: &ChannelMessageRequest) -> Checked {
    match request {
        ChannelMessageRequest::ChannelSend {
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
        ChannelMessageRequest::ChannelMessagesBefore {
            community: c,
            channel: ch,
            limit,
            ..
        } => {
            community(c)?;
            channel(ch)?;
            page(*limit)
        }
        ChannelMessageRequest::ChannelMessages { channel: ch, limit } => {
            channel(ch)?;
            page(*limit)
        }
        ChannelMessageRequest::ChannelMessageEdit {
            channel: ch,
            message_id,
            body,
        } => {
            channel(ch)?;
            text(message_id, MAX_MESSAGE_ID_LEN, "message id")?;
            message_body(body)
        }
        ChannelMessageRequest::ChannelMessageSend {
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
        ChannelMessageRequest::ChannelMessageDelete {
            channel: ch,
            message_id,
        } => {
            channel(ch)?;
            text(message_id, MAX_MESSAGE_ID_LEN, "message id")
        }
        ChannelMessageRequest::ChannelMessageForward {
            source_community,
            source_channel,
            source_message,
            dest_community,
            dest_channel,
        } => {
            community(source_community)?;
            channel(source_channel)?;
            text(source_message, MAX_MESSAGE_ID_LEN, "message id")?;
            community(dest_community)?;
            channel(dest_channel)
        }
        ChannelMessageRequest::ChannelMessageAdminDelete {
            community: c,
            channel: ch,
            message_id,
            reason,
        } => {
            community(c)?;
            channel(ch)?;
            text(message_id, MAX_MESSAGE_ID_LEN, "message id")?;
            opt(reason.as_deref(), |r| {
                text(r, MAX_DELETE_REASON_LEN, "reason")
            })
        }
        ChannelMessageRequest::ChannelMessageBulkDelete {
            community: c,
            channel: ch,
            message_ids,
            reason,
        } => {
            community(c)?;
            channel(ch)?;
            if message_ids.is_empty() || message_ids.len() > MAX_BULK_DELETE {
                return Err(IpcResponse::error(
                    400,
                    format!("a bulk delete names 1 to {MAX_BULK_DELETE} messages"),
                ));
            }
            for id in message_ids {
                text(id, MAX_MESSAGE_ID_LEN, "message id")?;
            }
            opt(reason.as_deref(), |r| {
                text(r, MAX_DELETE_REASON_LEN, "reason")
            })
        }
        ChannelMessageRequest::ChannelTyping {
            community: c,
            channel: ch,
        } => {
            community(c)?;
            channel(ch)
        }
        ChannelMessageRequest::ChannelSetActive {
            community: c,
            channel: ch,
            kind,
        } => {
            community(c)?;
            opt(ch.as_deref(), channel)?;
            channel_kind(kind)
        }
        other => unreachable!("{} is not a channel message request", other.name()),
    }
}
