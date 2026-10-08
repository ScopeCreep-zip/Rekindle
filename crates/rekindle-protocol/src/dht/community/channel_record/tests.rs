use rekindle_codec::community::channel_record::{ChannelReaction, ChannelRecordEntry};

// ── Plan C7.13: idempotent append and compare-and-swap merge ──

fn reaction(expression: &str, lamport: u64) -> ChannelRecordEntry {
    ChannelRecordEntry::Reaction(ChannelReaction {
        message_id: "msg-1".into(),
        expression: expression.into(),
        added: true,
        lamport,
    })
}

#[test]
fn an_entry_already_on_the_page_is_found() {
    let page = vec![reaction("🎉", 1), reaction("👍", 3)];
    assert!(super::write::contains_entry(&page, &reaction("👍", 3)));
    assert!(!super::write::contains_entry(&page, &reaction("👍", 4)));
}

#[test]
fn a_merge_keeps_one_copy_of_each_entry_in_lamport_order() {
    let theirs = vec![reaction("a", 1), reaction("b", 5)];
    let ours = vec![reaction("a", 1), reaction("c", 3)];
    let merged = super::write::merge_entries(theirs, ours);
    let lamports: Vec<u64> = merged.iter().map(ChannelRecordEntry::lamport).collect();
    assert_eq!(lamports, vec![1, 3, 5], "the duplicate is kept once");
}
