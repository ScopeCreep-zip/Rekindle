use super::raised_hands;
use crate::deps::OutboxItem;
use rekindle_codec::community::channel_record::{ChannelHandRaise, ChannelRecordEntry};
use rekindle_types::id::PseudonymKey;

fn hand(author: u8, seq: u64, raised: bool) -> OutboxItem {
    OutboxItem {
        admission: None,
        author: PseudonymKey([author; 32]),
        stream_seq: seq,
        committed_at: 0,
        entry: ChannelRecordEntry::HandRaise(ChannelHandRaise {
            raised,
            lamport: seq,
        }),
    }
}

/// A member's latest raise or lower by stream seq stands, wherever it
/// sits in the read.
#[test]
fn the_latest_hand_raise_per_member_stands() {
    let hands = raised_hands(&[
        hand(1, 2, false),
        hand(1, 1, true),
        hand(2, 1, false),
        hand(2, 3, true),
    ]);
    assert_eq!(hands, vec![hex::encode([2u8; 32])]);
}
