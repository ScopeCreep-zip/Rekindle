//! Per-channel SMPL message records: their creation, reads and appends over
//! the record pool. Each channel has its own DHT record for message history;
//! members write directly to the subkey matching their registry slot. The page
//! types and their signed encoding live in
//! `rekindle_codec::community::channel_record` (plan C8).

mod read;
mod write;

pub use read::{read_all_channel_entries, read_all_channel_messages};
pub use write::{
    create_smpl_channel_record, write_member_attachment_cached, write_member_forward,
    write_member_hand_raise, write_member_message, write_member_poll_close,
    write_member_poll_create, write_member_poll_vote, write_member_reaction, AppendOutcome,
};

#[cfg(test)]
mod tests;
