//! The read markers and unread counts (plan E3.1), carried as `CommunityRequest::Unread`.

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A read markers and unread counts request.
#[derive(Clone, Serialize, Deserialize)]
pub enum UnreadRequest {
    /// Advance our read marker on `channel` to the message `up_to` (its
    /// lamport); a marker never moves back.
    ChannelMarkRead {
        community: String,
        channel: String,
        up_to: String,
    },

    /// Each channel's unread messages and mentions, counted after our read
    /// markers.
    UnreadCounts { community: String },
}

impl UnreadRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::Query;
        use SecurityLevel::Authenticated;
        match self {
            Self::ChannelMarkRead { .. } => ("ChannelMarkRead", Query, Authenticated),
            Self::UnreadCounts { .. } => ("UnreadCounts", Query, Authenticated),
        }
    }

    /// The capability an `Agent`-level sender needs for this request, which every
    /// request of this domain has.
    #[must_use]
    pub fn agent_capability(&self) -> AgentCapability {
        use AgentCapability as C;
        match self {
            Self::ChannelMarkRead { .. } | Self::UnreadCounts { .. } => C::CommunitiesRead,
        }
    }
}
