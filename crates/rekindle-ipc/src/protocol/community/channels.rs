//! The channels and categories: their list, creation, edits, order and overwrites (plan E3.1), carried as `CommunityRequest::Channels`.

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A channels and categories request.
#[derive(Clone, Serialize, Deserialize)]
pub enum ChannelRequest {
    /// List channels in a community.
    ChannelList { community: String },

    /// Create a new channel in a community.
    ChannelCreate {
        community: String,
        name: String,
        kind: String,
        category: Option<String>,
        topic: Option<String>,
        slowmode_seconds: u32,
        /// The voice channel a text-in-voice channel belongs to.
        parent_voice_channel: Option<String>,
    },

    /// Delete a channel.
    ChannelDelete {
        community: String,
        channel_id: String,
    },

    /// Update channel properties.
    ChannelUpdate {
        community: String,
        channel_id: String,
        name: Option<String>,
        topic: Option<String>,
        slowmode_seconds: Option<u32>,
    },

    /// Rename a channel.
    ChannelRename {
        community: String,
        channel_id: String,
        name: String,
    },

    /// Move a channel into a category, or out of every category (`None`).
    ChannelMove {
        community: String,
        channel_id: String,
        category: Option<String>,
    },

    /// Order a community's channels: each gets its index as its position.
    ChannelReorder {
        community: String,
        channel_ids: Vec<String>,
    },

    /// Set a channel's topic.
    ChannelSetTopic {
        community: String,
        channel_id: String,
        topic: String,
    },

    /// Set a forum channel's tags.
    ChannelSetForumTags {
        community: String,
        channel_id: String,
        forum_tags: Vec<String>,
    },

    /// Set a channel's slowmode: the seconds a member waits between posts
    /// (0 turns it off).
    ChannelSetSlowmode {
        community: String,
        channel_id: String,
        seconds: u32,
    },

    /// Set a channel's permission overwrite for a role or member.
    ChannelOverwriteSet {
        community: String,
        channel_id: String,
        target_type: String,
        target_id: String,
        allow: u64,
        deny: u64,
    },

    /// Clear a channel's permission overwrite for a role or member.
    ChannelOverwriteDelete {
        community: String,
        channel_id: String,
        target_type: String,
        target_id: String,
    },

    /// Create a channel category.
    CategoryCreate { community: String, name: String },

    /// Delete a channel category; its channels leave it.
    CategoryDelete {
        community: String,
        category_id: String,
    },

    /// Rename a channel category.
    CategoryRename {
        community: String,
        category_id: String,
        name: String,
    },

    /// Order a community's categories: each gets its index as its position.
    CategoryReorder {
        community: String,
        category_ids: Vec<String>,
    },
}

impl ChannelRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::ChannelList { .. } => ("ChannelList", Query, Authenticated),
            Self::ChannelCreate { .. } => ("ChannelCreate", Write, Authenticated),
            Self::ChannelDelete { .. } => ("ChannelDelete", Write, Authenticated),
            Self::ChannelUpdate { .. } => ("ChannelUpdate", Write, Authenticated),
            Self::ChannelRename { .. } => ("ChannelRename", Write, Authenticated),
            Self::ChannelMove { .. } => ("ChannelMove", Write, Authenticated),
            Self::ChannelReorder { .. } => ("ChannelReorder", Write, Authenticated),
            Self::ChannelSetTopic { .. } => ("ChannelSetTopic", Write, Authenticated),
            Self::ChannelSetSlowmode { .. } => ("ChannelSetSlowmode", Write, Authenticated),
            Self::ChannelSetForumTags { .. } => ("ChannelSetForumTags", Write, Authenticated),
            Self::ChannelOverwriteSet { .. } => ("ChannelOverwriteSet", Write, Authenticated),
            Self::ChannelOverwriteDelete { .. } => ("ChannelOverwriteDelete", Write, Authenticated),
            Self::CategoryCreate { .. } => ("CategoryCreate", Write, Authenticated),
            Self::CategoryDelete { .. } => ("CategoryDelete", Write, Authenticated),
            Self::CategoryRename { .. } => ("CategoryRename", Write, Authenticated),
            Self::CategoryReorder { .. } => ("CategoryReorder", Write, Authenticated),
        }
    }

    /// The capability an `Agent`-level sender needs for this request, which every
    /// request of this domain has.
    #[must_use]
    pub fn agent_capability(&self) -> AgentCapability {
        use AgentCapability as C;
        match self {
            Self::ChannelList { .. } => C::CommunitiesRead,
            Self::ChannelCreate { .. }
            | Self::ChannelDelete { .. }
            | Self::ChannelUpdate { .. }
            | Self::ChannelRename { .. }
            | Self::ChannelMove { .. }
            | Self::ChannelReorder { .. }
            | Self::ChannelSetTopic { .. }
            | Self::ChannelSetSlowmode { .. }
            | Self::ChannelSetForumTags { .. }
            | Self::ChannelOverwriteSet { .. }
            | Self::ChannelOverwriteDelete { .. }
            | Self::CategoryCreate { .. }
            | Self::CategoryDelete { .. }
            | Self::CategoryRename { .. }
            | Self::CategoryReorder { .. } => C::CommunitiesWrite,
        }
    }
}
