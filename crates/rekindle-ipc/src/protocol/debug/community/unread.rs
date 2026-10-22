//! `Debug` for the read markers and unread counts requests: free text shows its length only.

use crate::protocol::UnreadRequest;

impl std::fmt::Debug for UnreadRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelMarkRead {
                community,
                channel,
                up_to,
            } => f
                .debug_struct("ChannelMarkRead")
                .field("community", community)
                .field("channel", channel)
                .field("up_to", up_to)
                .finish(),
            Self::UnreadCounts { community } => f
                .debug_struct("UnreadCounts")
                .field("community", community)
                .finish(),
        }
    }
}
