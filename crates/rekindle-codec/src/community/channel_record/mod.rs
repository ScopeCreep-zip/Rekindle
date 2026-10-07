//! Per-channel SMPL record page types and their signed encoding (moved
//! from `rekindle_protocol::dht::community::channel_record`, plan C8). The
//! record reads and writes stay in rekindle-protocol.

// Layout aliased from `rekindle_types::dht_layout::channel`, the one
// home both tracks index these records through.
pub use rekindle_types::dht_layout::channel::{
    HEADER_SUBKEY as CHANNEL_HEADER_SUBKEY, MEMBER_SUBKEY_COUNT as CHANNEL_MEMBER_SUBKEY_COUNT,
    OWNER_SUBKEY_COUNT as CHANNEL_OWNER_SUBKEY_COUNT,
};

mod codec;
mod types;

pub use codec::{decode_channel_entries, decode_own_page, encode_page_entries, message_from_entry};
pub use types::{
    ChannelAttachmentCached, ChannelForward, ChannelHandRaise, ChannelMessage, ChannelPollClose,
    ChannelPollCreate, ChannelPollVote, ChannelReaction, ChannelRecordEntry, ChannelRecordItem,
    ChannelSubkeyPayload,
};

#[cfg(test)]
mod tests;
