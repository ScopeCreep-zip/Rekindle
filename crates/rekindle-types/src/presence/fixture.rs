//! Test builders for presence values, shared by every crate whose tests
//! need them (`fixture` feature; always on in this crate's own tests).

use super::HistoryRange;
use crate::id::ChannelId;

/// A history range of the channel whose 16 id bytes are all `channel`,
/// spanning lamports `1..=newest`.
#[must_use]
pub fn history_range(channel: u8, newest: u64) -> HistoryRange {
    HistoryRange {
        channel_id: ChannelId([channel; 16]),
        oldest_lamport: 1,
        newest_lamport: newest,
    }
}
