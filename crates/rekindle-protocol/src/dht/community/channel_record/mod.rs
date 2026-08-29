//! Per-channel SMPL message records.
//!
//! Each channel has its own DHT record for storing message history.
//! Channel records use zero-owner SMPL: each member writes directly to the
//! subkey matching their registry slot.

/// No dedicated header subkey exists in v2.0 channel records.
pub const CHANNEL_HEADER_SUBKEY: u32 = 0;

/// Channel SMPL records use `o_cnt:0`; member slots start at subkey 0.
pub const CHANNEL_OWNER_SUBKEY_COUNT: u16 = 0;

/// Each member gets 1 subkey for message submission.
pub const CHANNEL_MEMBER_SUBKEY_COUNT: u16 = 1;

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
