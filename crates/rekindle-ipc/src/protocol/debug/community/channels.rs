//! The channel, category and MEK requests' `Debug` (plan E3.1): a
//! message body shows its length only.

use crate::protocol::ChannelRequest;

impl std::fmt::Debug for ChannelRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
            Self::ChannelSetSlowmode {
                community,
                channel_id,
                seconds,
            } => f
                .debug_struct("ChannelSetSlowmode")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("seconds", seconds)
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
        }
    }
}
