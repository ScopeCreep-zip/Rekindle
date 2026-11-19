//! Channel entry types: what an author's channel outbox (`M`/4) entry
//! seals (plan E3.3). A channel has no record of its own.

mod codec;
mod types;

pub use codec::message_from_entry;
pub use types::{
    ChannelAttachmentCached, ChannelDelete, ChannelEdit, ChannelForward, ChannelHandRaise,
    ChannelMessage, ChannelPollClose, ChannelPollCounted, ChannelPollCreate, ChannelPollVote,
    ChannelReaction, ChannelRecordEntry,
};
