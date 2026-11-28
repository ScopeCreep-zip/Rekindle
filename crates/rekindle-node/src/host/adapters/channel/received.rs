//! A received channel post and where its body was sealed: at its stream
//! seq in its author's channel outbox (plan E3.3; a channel has no record).

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
    /// In its author's channel outbox at `stream_seq`; the anchor is the
    /// community's. `departure_head`: the author departed, and its entry
    /// is at or below a head its departures recorded of this outbox, which
    /// the chain we read verified (plan E3.3 over A/B R3-8 and S9).
    Outbox {
        stream_seq: u64,
        departure_head: Option<OutboxHead>,
    },
}

/// A departure's head of a member's channel outbox (`M`/4) that the chain
/// we read reproduces: the member record, the head's chain seq and the
/// digest of the entry at it (`rekindle_secrets::slot_head`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxHead {
    pub member_record: String,
    pub seq: u32,
    pub digest: [u8; 32],
}

impl ReceivedPost {
    pub(super) fn body_at(&self) -> ingest::PostAt<'_> {
        let ReceivedAt::Outbox { stream_seq, .. } = &self.at;
        ingest::PostAt {
            anchor: &self.community_id,
            stream_seq: *stream_seq,
        }
    }

    /// The departure head the gate judges a departed author's post by
    /// (`ingest::GateItem::slot`): only an outbox entry the chain placed at
    /// or below a recorded head carries one.
    pub(super) fn departure_position(&self) -> Option<ingest::SlotPosition<'_>> {
        match &self.at {
            ReceivedAt::Outbox {
                departure_head: Some(head),
                ..
            } => Some(ingest::SlotPosition {
                record_key: &head.member_record,
                seq: head.seq,
                digest: head.digest,
            }),
            ReceivedAt::Outbox { .. } => None,
        }
    }
}
