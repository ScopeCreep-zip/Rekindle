//! The channel messages: sending, editing, deleting, forwarding, history, typing, the active channel and the channel keys (plan E3.1), carried as `CommunityRequest::Messages`.

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A channel messages request.
#[derive(Clone, Serialize, Deserialize)]
pub enum ChannelMessageRequest {
    /// Send a message to a channel.
    ChannelSend {
        community: String,
        channel: String,
        body: String,
        /// The message id of the message this one replies to.
        reply_to: Option<String>,
    },

    /// Replace the body of our message `message_id` in `channel` (a channel
    /// id; the Host finds its community).
    ChannelMessageEdit {
        channel: String,
        message_id: String,
        body: String,
    },

    /// Post `body` to `channel` (a channel id; the Host finds its community).
    ChannelMessageSend {
        channel: String,
        body: String,
        /// The message id of the message this one replies to.
        reply_to: Option<String>,
    },

    /// Delete our message `message_id` in `channel` (a channel id).
    ChannelMessageDelete { channel: String, message_id: String },

    /// Forward the stored message `source_message` of `source_channel` (in
    /// `source_community`) into `dest_channel` of `dest_community`.
    ChannelMessageForward {
        source_community: String,
        source_channel: String,
        source_message: String,
        dest_community: String,
        dest_channel: String,
    },

    /// Delete another member's message `message_id` in `channel` of
    /// `community` (`MANAGE_MESSAGES`).
    ChannelMessageAdminDelete {
        community: String,
        channel: String,
        message_id: String,
        reason: Option<String>,
    },

    /// Delete up to 100 messages in `channel` of `community`
    /// (`MANAGE_MESSAGES`): how many were deleted.
    ChannelMessageBulkDelete {
        community: String,
        channel: String,
        message_ids: Vec<String>,
        reason: Option<String>,
    },

    /// Tell the community's mesh we are typing in `channel`.
    ChannelTyping { community: String, channel: String },

    /// The channel we are in (`kind` "voice" or "text"), or none, which our
    /// presence carries.
    ChannelSetActive {
        community: String,
        channel: Option<String>,
        kind: String,
    },

    /// The latest `limit` messages of `channel` (a channel id; the Host
    /// finds its community): our stored rows merged with the channel
    /// record's (plan E3.1 E2c).
    ChannelMessages { channel: String, limit: u32 },

    /// Up to `limit` messages of `channel` in `community` older than
    /// `before_timestamp`, merged the same way (a page further back).
    ChannelMessagesBefore {
        community: String,
        channel: String,
        before_timestamp: u64,
        limit: u32,
    },

    /// List cached MEKs for a community.
    MekList { community: String },

    /// Rotate MEK for a channel.
    MekRotate { community: String, channel: String },

    /// Request a MEK from community peers (gossip broadcast).
    MekRequest {
        community: String,
        channel: String,
        generation: u64,
    },
}

impl ChannelMessageRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::ChannelSend { .. } => ("ChannelSend", Write, Authenticated),
            Self::ChannelMessages { .. } => ("ChannelMessages", Query, Authenticated),
            Self::ChannelMessagesBefore { .. } => ("ChannelMessagesBefore", Query, Authenticated),
            Self::ChannelMessageEdit { .. } => ("ChannelMessageEdit", Write, Authenticated),
            Self::ChannelMessageDelete { .. } => ("ChannelMessageDelete", Write, Authenticated),
            Self::ChannelMessageSend { .. } => ("ChannelMessageSend", Write, Authenticated),
            Self::ChannelMessageForward { .. } => ("ChannelMessageForward", Write, Authenticated),
            Self::ChannelMessageAdminDelete { .. } => {
                ("ChannelMessageAdminDelete", Write, Authenticated)
            }
            Self::ChannelMessageBulkDelete { .. } => {
                ("ChannelMessageBulkDelete", Write, Authenticated)
            }
            Self::ChannelTyping { .. } => ("ChannelTyping", Write, Authenticated),
            Self::ChannelSetActive { .. } => ("ChannelSetActive", Write, Authenticated),
            Self::MekList { .. } => ("MekList", Query, Authenticated),
            Self::MekRotate { .. } => ("MekRotate", Write, Authenticated),
            Self::MekRequest { .. } => ("MekRequest", Write, Authenticated),
        }
    }

    /// The capability an `Agent`-level sender needs for this request, or
    /// `None` when no agent may send it.
    #[must_use]
    pub fn agent_capability(&self) -> Option<AgentCapability> {
        use AgentCapability as C;
        match self {
            Self::ChannelMessages { .. } | Self::ChannelMessagesBefore { .. } => {
                Some(C::ChannelsHistory)
            }
            Self::ChannelSend { .. }
            | Self::ChannelMessageEdit { .. }
            | Self::ChannelMessageDelete { .. }
            | Self::ChannelMessageSend { .. }
            | Self::ChannelMessageForward { .. }
            | Self::ChannelTyping { .. }
            | Self::ChannelSetActive { .. } => Some(C::ChatWrite),
            Self::ChannelMessageAdminDelete { .. } | Self::ChannelMessageBulkDelete { .. } => {
                Some(C::ModerationWrite)
            }
            // Channel keys are key material; ownership is the owner's own.
            Self::MekList { .. } | Self::MekRotate { .. } | Self::MekRequest { .. } => None,
        }
    }
}
