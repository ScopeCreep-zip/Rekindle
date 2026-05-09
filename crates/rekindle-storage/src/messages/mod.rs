//! Message storage — sent DMs, received DMs, channel message cache.

pub mod dm_sent;
pub mod dm_received;
pub mod channel;

/// A single DM message record returned by queries.
#[derive(Debug, Clone)]
pub struct DmRecord {
    pub sender_name: String,
    pub body: String,
    pub timestamp: u64,
    pub message_id: String,
    pub is_self: bool,
}

/// A single channel message record returned by queries.
#[derive(Debug, Clone)]
pub struct ChannelRecord {
    pub author_pseudonym: String,
    pub author_display_name: String,
    pub body: String,
    pub timestamp: u64,
    pub sequence: u64,
    pub message_id: String,
    pub mek_generation: u64,
}
