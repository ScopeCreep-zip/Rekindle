//! The channel, category and MEK requests' `Debug` (plan E3.1): a
//! message body shows its length only.

use super::super::super::CommunityRequest;

impl CommunityRequest {
    /// The channel send, CRUD, history and MEK requests' `Debug`.
    pub(super) fn fmt_channels(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
            Self::ChannelList { community } => f
                .debug_struct("ChannelList")
                .field("community", community)
                .finish(),
            Self::ChannelCreate {
                community,
                name,
                kind,
                category,
                topic,
                slowmode_seconds,
                parent_voice_channel,
            } => f
                .debug_struct("ChannelCreate")
                .field("community", community)
                .field("name", name)
                .field("kind", kind)
                .field("category", category)
                .field("topic", topic)
                .field("slowmode_seconds", slowmode_seconds)
                .field("parent_voice_channel", parent_voice_channel)
                .finish(),
            Self::ChannelDelete {
                community,
                channel_id,
            } => f
                .debug_struct("ChannelDelete")
                .field("community", community)
                .field("channel_id", channel_id)
                .finish(),
            Self::ChannelUpdate {
                community,
                channel_id,
                name,
                topic,
                slowmode_seconds,
            } => f
                .debug_struct("ChannelUpdate")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("name", name)
                .field("topic", topic)
                .field("slowmode_seconds", slowmode_seconds)
                .finish(),
            Self::ChannelHistory {
                community,
                channel,
                limit,
            } => f
                .debug_struct("ChannelHistory")
                .field("community", community)
                .field("channel", channel)
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
            other => unreachable!("{} is not a channel or MEK request", other.name()),
        }
    }

    /// The channel and category administration requests' `Debug`.
    pub(super) fn fmt_channel_admin(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelRename {
                community,
                channel_id,
                name,
            } => f
                .debug_struct("ChannelRename")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("name", name)
                .finish(),
            Self::ChannelMove {
                community,
                channel_id,
                category,
            } => f
                .debug_struct("ChannelMove")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("category", category)
                .finish(),
            Self::ChannelReorder {
                community,
                channel_ids,
            } => f
                .debug_struct("ChannelReorder")
                .field("community", community)
                .field("channel_ids", channel_ids)
                .finish(),
            Self::ChannelSetTopic {
                community,
                channel_id,
                topic,
            } => f
                .debug_struct("ChannelSetTopic")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("topic", topic)
                .finish(),
            Self::ChannelSetForumTags {
                community,
                channel_id,
                forum_tags,
            } => f
                .debug_struct("ChannelSetForumTags")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("forum_tags", forum_tags)
                .finish(),
            Self::ChannelOverwriteSet {
                community,
                channel_id,
                target_type,
                target_id,
                allow,
                deny,
            } => f
                .debug_struct("ChannelOverwriteSet")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("target_type", target_type)
                .field("target_id", target_id)
                .field("allow", allow)
                .field("deny", deny)
                .finish(),
            Self::ChannelOverwriteDelete {
                community,
                channel_id,
                target_type,
                target_id,
            } => f
                .debug_struct("ChannelOverwriteDelete")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("target_type", target_type)
                .field("target_id", target_id)
                .finish(),
            Self::CategoryCreate { community, name } => f
                .debug_struct("CategoryCreate")
                .field("community", community)
                .field("name", name)
                .finish(),
            Self::CategoryDelete {
                community,
                category_id,
            } => f
                .debug_struct("CategoryDelete")
                .field("community", community)
                .field("category_id", category_id)
                .finish(),
            Self::CategoryRename {
                community,
                category_id,
                name,
            } => f
                .debug_struct("CategoryRename")
                .field("community", community)
                .field("category_id", category_id)
                .field("name", name)
                .finish(),
            Self::CategoryReorder {
                community,
                category_ids,
            } => f
                .debug_struct("CategoryReorder")
                .field("community", community)
                .field("category_ids", category_ids)
                .finish(),
            other => unreachable!("{} is not a channel admin request", other.name()),
        }
    }
}
