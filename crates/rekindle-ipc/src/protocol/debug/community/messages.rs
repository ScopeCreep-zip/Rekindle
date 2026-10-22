//! `Debug` for the channel messages requests: free text shows its length only.

use crate::protocol::ChannelMessageRequest;

impl std::fmt::Debug for ChannelMessageRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelSend {
                community,
                channel,
                body,
                reply_to,
            } => f
                .debug_struct("ChannelSend")
                .field("community", community)
                .field("channel", channel)
                .field("body_len", &body.len())
                .field("reply_to", reply_to)
                .finish(),
            Self::ChannelMessageEdit {
                channel,
                message_id,
                body,
            } => f
                .debug_struct("ChannelMessageEdit")
                .field("channel", channel)
                .field("message_id", message_id)
                .field("body_len", &body.len())
                .finish(),
            Self::ChannelMessageDelete {
                channel,
                message_id,
            } => f
                .debug_struct("ChannelMessageDelete")
                .field("channel", channel)
                .field("message_id", message_id)
                .finish(),
            Self::ChannelMessageForward {
                source_community,
                source_channel,
                source_message,
                dest_community,
                dest_channel,
            } => f
                .debug_struct("ChannelMessageForward")
                .field("source_community", source_community)
                .field("source_channel", source_channel)
                .field("source_message", source_message)
                .field("dest_community", dest_community)
                .field("dest_channel", dest_channel)
                .finish(),
            Self::ChannelMessageAdminDelete {
                community,
                channel,
                message_id,
                reason,
            } => f
                .debug_struct("ChannelMessageAdminDelete")
                .field("community", community)
                .field("channel", channel)
                .field("message_id", message_id)
                .field("has_reason", &reason.is_some())
                .finish(),
            Self::ChannelMessageBulkDelete {
                community,
                channel,
                message_ids,
                reason,
            } => f
                .debug_struct("ChannelMessageBulkDelete")
                .field("community", community)
                .field("channel", channel)
                .field("messages", &message_ids.len())
                .field("has_reason", &reason.is_some())
                .finish(),
            Self::ChannelTyping { community, channel } => f
                .debug_struct("ChannelTyping")
                .field("community", community)
                .field("channel", channel)
                .finish(),
            Self::ChannelSetActive {
                community,
                channel,
                kind,
            } => f
                .debug_struct("ChannelSetActive")
                .field("community", community)
                .field("channel", channel)
                .field("kind", kind)
                .finish(),
            Self::ChannelMessageSend { channel, .. } => f
                .debug_struct("ChannelMessageSend")
                .field("channel", channel)
                .finish_non_exhaustive(),
            Self::ChannelMessages { channel, limit } => f
                .debug_struct("ChannelMessages")
                .field("channel", channel)
                .field("limit", limit)
                .finish(),
            Self::ChannelMessagesBefore {
                community,
                channel,
                before_timestamp,
                limit,
            } => f
                .debug_struct("ChannelMessagesBefore")
                .field("community", community)
                .field("channel", channel)
                .field("before_timestamp", before_timestamp)
                .field("limit", limit)
                .finish(),
            Self::MekList { community } => f
                .debug_struct("MekList")
                .field("community", community)
                .finish(),
            Self::MekRotate { community, channel } => f
                .debug_struct("MekRotate")
                .field("community", community)
                .field("channel", channel)
                .finish(),
            Self::MekRequest {
                community,
                channel,
                generation,
            } => f
                .debug_struct("MekRequest")
                .field("community", community)
                .field("channel", channel)
                .field("generation", generation)
                .finish(),
        }
    }
}
