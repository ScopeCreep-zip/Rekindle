use super::{build_reaction_groups, MAX_REACTIONS_PER_MESSAGE, MAX_REACTION_KEY_LEN};
use crate::deps::OutboxItem;
use rekindle_codec::community::channel_record::{ChannelReaction, ChannelRecordEntry};
use rekindle_types::id::PseudonymKey;

fn reaction(author: u8, seq: u64, target: &str, emoji: &str, added: bool) -> OutboxItem {
    OutboxItem {
        author: PseudonymKey([author; 32]),
        stream_seq: seq,
        committed_at: seq * 1_000,
        entry: ChannelRecordEntry::Reaction(ChannelReaction {
            message_id: target.into(),
            expression: emoji.into(),
            added,
            lamport: seq,
        }),
    }
}

/// R1: a reactor's latest op by stream seq wins wherever it sits in the
/// read, and a repeat changes nothing.
#[test]
fn the_latest_op_per_reactor_wins_by_stream_seq() {
    let groups = build_reaction_groups(&[
        reaction(1, 5, "m", "tada", false),
        reaction(1, 3, "m", "tada", true),
        reaction(2, 1, "m", "tada", true),
        reaction(2, 2, "m", "tada", true),
    ]);
    let tada = &groups["m"][0];
    assert_eq!(tada.reactors, vec![hex::encode([2u8; 32])]);
    assert_eq!(tada.count, 1);
}

/// R2: an over-long key is dropped and a message counts at most
/// `MAX_REACTIONS_PER_MESSAGE` emojis, the first added.
#[test]
fn readers_cap_the_key_length_and_distinct_emojis() {
    let mut items = vec![reaction(
        1,
        1,
        "m",
        &"x".repeat(MAX_REACTION_KEY_LEN + 1),
        true,
    )];
    items.extend(
        (0..=MAX_REACTIONS_PER_MESSAGE as u64)
            .map(|i| reaction(2, i + 10, "m", &format!("e{i:02}"), true)),
    );
    let groups = &build_reaction_groups(&items)["m"];
    assert_eq!(groups.len(), MAX_REACTIONS_PER_MESSAGE);
    assert!(groups.iter().all(|g| g.emoji.starts_with('e')));
    assert!(!groups
        .iter()
        .any(|g| g.emoji == format!("e{MAX_REACTIONS_PER_MESSAGE:02}")));
}
