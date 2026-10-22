//! The pins and search (plan E3.1), carried as `CommunityRequest::Pins`.

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A pins and search request.
#[derive(Clone, Serialize, Deserialize)]
pub enum PinRequest {
    /// A channel's pinned messages.
    ChannelPins {
        community: String,
        channel_id: String,
    },

    /// Pin a channel message (`MANAGE_MESSAGES`).
    MessagePin {
        community: String,
        channel_id: String,
        message_id: String,
    },

    /// Unpin a channel message (`MANAGE_MESSAGES`).
    MessageUnpin {
        community: String,
        channel_id: String,
        message_id: String,
    },

    /// Pin or unpin an attachment, exempting it from cache eviction
    /// (`MANAGE_COMMUNITY`).
    AttachmentPin {
        community: String,
        attachment_id: String,
        pinned: bool,
    },

    /// Full-text search over the stored messages.
    MessageSearch {
        request: rekindle_types::search::MessageSearch,
    },
}

impl PinRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::ChannelPins { .. } => ("ChannelPins", Query, Authenticated),
            Self::MessagePin { .. } => ("MessagePin", Write, Authenticated),
            Self::MessageUnpin { .. } => ("MessageUnpin", Write, Authenticated),
            Self::AttachmentPin { .. } => ("AttachmentPin", Write, Authenticated),
            Self::MessageSearch { .. } => ("MessageSearch", Query, Authenticated),
        }
    }

    /// The capability an `Agent`-level sender needs for this request, which every
    /// request of this domain has.
    #[must_use]
    pub fn agent_capability(&self) -> AgentCapability {
        use AgentCapability as C;
        match self {
            Self::ChannelPins { .. } | Self::MessageSearch { .. } => C::CommunitiesRead,
            Self::MessagePin { .. } | Self::MessageUnpin { .. } | Self::AttachmentPin { .. } => {
                C::CommunitiesWrite
            }
        }
    }
}
