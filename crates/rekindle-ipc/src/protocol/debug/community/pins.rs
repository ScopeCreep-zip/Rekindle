//! Pins and search (plan E3.1 E3b): a search query shows its length only.

use super::super::super::CommunityRequest;

impl CommunityRequest {
    pub(super) fn fmt_pins(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelPins {
                community,
                channel_id,
            } => f
                .debug_struct("ChannelPins")
                .field("community", community)
                .field("channel_id", channel_id)
                .finish(),
            Self::MessagePin {
                community,
                channel_id,
                message_id,
            }
            | Self::MessageUnpin {
                community,
                channel_id,
                message_id,
            } => f
                .debug_struct(self.name())
                .field("community", community)
                .field("channel_id", channel_id)
                .field("message_id", message_id)
                .finish(),
            Self::AttachmentPin {
                community,
                attachment_id,
                pinned,
            } => f
                .debug_struct("AttachmentPin")
                .field("community", community)
                .field("attachment_id", attachment_id)
                .field("pinned", pinned)
                .finish(),
            Self::MessageSearch { request } => f
                .debug_struct("MessageSearch")
                .field("query", &request.query.len())
                .field("limit", &request.limit)
                .finish_non_exhaustive(),
            other => unreachable!("{} is not a pins or search request", other.name()),
        }
    }
}
