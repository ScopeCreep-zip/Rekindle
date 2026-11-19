//! A received channel post and where its body was sealed: at a slot of a
//! channel record (history before the outbox) or at its stream seq in its
//! author's channel outbox (plan E3.3).

use rekindle_channel::ingest;
use rekindle_codec::community::channel_record::ChannelMessage;

/// A received post: where it was written and what it holds.
#[derive(Debug, Clone)]
pub struct ReceivedPost {
    pub community_id: String,
    pub channel_id: String,
    /// Where its body was sealed.
    pub at: ReceivedAt,
    pub sequence: u64,
    pub message: ChannelMessage,
    /// The original author, when the post is a forward.
    pub forwarded_from_author: Option<String>,
}

/// Where a received post's body was sealed (`ingest::PostAt`, owned).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceivedAt {
    /// At `subkey_index` of the channel record `record_key` (history
    /// written before the outbox, until E3.3 slice 8).
    Slot {
        record_key: String,
        subkey_index: u32,
    },
    /// In its author's channel outbox at `stream_seq`; the anchor is the
    /// community's.
    Outbox { stream_seq: u64 },
}

impl ReceivedPost {
    pub(super) fn body_at(&self) -> ingest::PostAt<'_> {
        match &self.at {
            ReceivedAt::Slot {
                record_key,
                subkey_index,
            } => ingest::PostAt::Slot {
                record_key,
                subkey_index: *subkey_index,
            },
            ReceivedAt::Outbox { stream_seq } => ingest::PostAt::Outbox {
                anchor: &self.community_id,
                stream_seq: *stream_seq,
            },
        }
    }
}
