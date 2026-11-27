use super::*;

#[test]
fn channel_kind_roundtrip_u8() {
    for v in 0..=8u8 {
        let kind = ChannelKind::from_u8(v).unwrap();
        assert_eq!(kind.to_u8(), v);
    }
    assert!(ChannelKind::from_u8(9).is_none());
}

#[test]
fn channel_kind_roundtrip_str() {
    let kinds = [
        "text",
        "voice",
        "announcement",
        "forum",
        "stage",
        "directory",
        "media",
        "events",
        "dm",
    ];
    for s in &kinds {
        let kind: ChannelKind = s.parse().unwrap();
        assert_eq!(kind.as_str(), *s);
        assert_eq!(kind.as_ref(), *s);
        assert_eq!(kind.to_string(), *s);
    }
}

#[test]
fn channel_kind_serde_json() {
    let kind = ChannelKind::Forum;
    let json = serde_json::to_string(&kind).unwrap();
    assert_eq!(json, "\"forum\"");
    let back: ChannelKind = serde_json::from_str(&json).unwrap();
    assert_eq!(back, kind);
}

#[test]
fn channel_entry_serde_roundtrip() {
    let entry = ChannelEntry::Message {
        message_id: MessageId([7u8; 16]),
        content: vec![0xDE, 0xAD],
        mek_generation: 3,
        timestamp: 1_710_000_000,
        lamport: 100,
        sequence: 5,
        reply_to: None,
        flags: 0,
        attachment: None,
    };
    let json = serde_json::to_string(&entry).unwrap();
    let back: ChannelEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(entry, back);
}

#[test]
fn legacy_message_without_attachment_field_deserializes() {
    // Verifies the #[serde(default)] backward-compat: an old payload
    // produced before AttachmentOffer existed must still round-trip.
    let legacy_json = r#"{
            "type": "message",
            "message_id": [1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],
            "content": [222, 173],
            "mek_generation": 3,
            "timestamp": 1710000000,
            "lamport": 100,
            "sequence": 5,
            "reply_to": null,
            "flags": 0
        }"#;
    let entry: ChannelEntry = serde_json::from_str(legacy_json).unwrap();
    match entry {
        ChannelEntry::Message { attachment, .. } => assert!(attachment.is_none()),
        _ => panic!("expected Message"),
    }
}

#[test]
fn reaction_toggle() {
    let add = ChannelEntry::Reaction {
        message_id: MessageId([1u8; 16]),
        expression: "👍".into(),
        added: true,
        lamport: 10,
    };
    let remove = ChannelEntry::Reaction {
        message_id: MessageId([1u8; 16]),
        expression: "👍".into(),
        added: false,
        lamport: 11,
    };
    // LWW: remove (lamport 11) wins over add (lamport 10)
    assert!(remove.lamport() > add.lamport());
}

#[test]
fn poll_entries_report_lamport() {
    let create = ChannelEntry::PollCreate {
        poll_id: [2u8; 16],
        message_id: MessageId([3u8; 16]),
        question: "Ready?".into(),
        answers: vec!["Yes".into(), "No".into()],
        multi_select: false,
        expires_at: None,
        lamport: 12,
    };
    let vote = ChannelEntry::PollVote {
        poll_id: [2u8; 16],
        selected_answers: vec![0],
        lamport: 13,
    };
    let close = ChannelEntry::PollClose {
        poll_id: [2u8; 16],
        lamport: 14,
    };

    assert_eq!(create.lamport(), 12);
    assert_eq!(vote.lamport(), 13);
    assert_eq!(close.lamport(), 14);
}
