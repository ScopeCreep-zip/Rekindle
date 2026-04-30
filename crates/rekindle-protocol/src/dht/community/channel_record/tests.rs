use super::{
    decode_channel_entries, ChannelMessage, ChannelPollCreate, ChannelPollVote, ChannelReaction,
    ChannelRecordEntry,
};

fn sample_message() -> ChannelMessage {
    ChannelMessage {
        sequence: 7,
        sender_pseudonym: "pseudo-1".into(),
        ciphertext: vec![1, 2, 3],
        mek_generation: 2,
        timestamp: 1234,
        reply_to: None,
        lamport_ts: 9,
        message_id: Some("msg-1".into()),
    }
}

#[test]
fn decode_legacy_message_page_as_entries() {
    let page = serde_json::to_vec(&vec![sample_message()]).unwrap();
    let entries = decode_channel_entries(&page).unwrap();
    assert!(matches!(&entries[0], ChannelRecordEntry::Message(message) if message.sequence == 7));
}

#[test]
fn decode_mixed_entry_page() {
    let entries = vec![
        ChannelRecordEntry::Message(sample_message()),
        ChannelRecordEntry::Reaction(ChannelReaction {
            message_id: "msg-1".into(),
            expression: "🔥".into(),
            added: true,
            lamport: 10,
        }),
        ChannelRecordEntry::PollCreate(ChannelPollCreate {
            poll_id: [4u8; 16],
            message_id: "msg-1".into(),
            question: "Ready?".into(),
            answers: vec!["Yes".into(), "No".into()],
            multi_select: false,
            expires_at: None,
            lamport: 11,
        }),
        ChannelRecordEntry::PollVote(ChannelPollVote {
            poll_id: [4u8; 16],
            selected_answers: vec![0],
            lamport: 12,
        }),
    ];
    let page = serde_json::to_vec(&entries).unwrap();
    let decoded = decode_channel_entries(&page).unwrap();
    assert_eq!(decoded.len(), 4);
    assert!(matches!(
        &decoded[1],
        ChannelRecordEntry::Reaction(reaction)
            if reaction.expression == "🔥" && reaction.added
    ));
    assert!(matches!(
        &decoded[2],
        ChannelRecordEntry::PollCreate(create)
            if create.question == "Ready?" && create.answers.len() == 2
    ));
    assert!(matches!(
        &decoded[3],
        ChannelRecordEntry::PollVote(vote)
            if vote.selected_answers == vec![0]
    ));
}
