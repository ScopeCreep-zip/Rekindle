//! Per-channel SMPL message records.
//!
//! Each channel has its own DHT record for storing message history.
//! Channel records use zero-owner SMPL: each member writes directly to the
//! subkey matching their registry slot.

// Layout aliased from `rekindle_types::dht_layout::channel`, the one
// home both tracks index these records through.
pub use rekindle_types::dht_layout::channel::{
    HEADER_SUBKEY as CHANNEL_HEADER_SUBKEY, MEMBER_SUBKEY_COUNT as CHANNEL_MEMBER_SUBKEY_COUNT,
    OWNER_SUBKEY_COUNT as CHANNEL_OWNER_SUBKEY_COUNT,
};

mod codec;
mod read;
mod types;
mod write;

pub use codec::decode_channel_entries;
pub use read::{read_all_channel_entries, read_all_channel_messages, watch_channel};
pub use types::{
    ChannelAttachmentCached, ChannelForward, ChannelHandRaise, ChannelMessage, ChannelPollClose,
    ChannelPollCreate, ChannelPollVote, ChannelReaction, ChannelRecordEntry, ChannelRecordItem,
    ChannelSubkeyPayload,
};
pub use write::{
    create_smpl_channel_record, write_member_attachment_cached, write_member_forward,
    write_member_hand_raise, write_member_message, write_member_poll_close,
    write_member_poll_create, write_member_poll_vote, write_member_reaction,
};

#[cfg(test)]
mod tests;
