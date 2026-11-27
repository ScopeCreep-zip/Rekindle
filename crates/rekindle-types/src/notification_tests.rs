use super::*;

#[test]
fn attachment_state_round_trip() {
    for raw in 0..=7u8 {
        let state = AttachmentState::from_u8(raw);
        assert_eq!(state as u8, raw);
    }
    assert_eq!(AttachmentState::from_u8(255), AttachmentState::Detached);
}

#[test]
fn attachment_state_from_veilid_string() {
    assert_eq!(
        AttachmentState::from_veilid_string("FullyAttached"),
        AttachmentState::FullyAttached
    );
    assert_eq!(
        AttachmentState::from_veilid_string("garbage"),
        AttachmentState::Detached
    );
}

#[test]
fn is_attached_correct() {
    assert!(!AttachmentState::Detached.is_attached());
    assert!(!AttachmentState::Attaching.is_attached());
    assert!(AttachmentState::AttachedWeak.is_attached());
    assert!(AttachmentState::AttachedGood.is_attached());
    assert!(AttachmentState::FullyAttached.is_attached());
    assert!(!AttachmentState::Detaching.is_attached());
}

#[test]
fn notification_serializes_round_trip() {
    let notif = TransportNotification::MessagePosted {
        community_id: "abc".into(),
        channel_id: "general".into(),
        message_id: "msg1".into(),
        sender_pseudonym: "alice".into(),
        lamport_ts: 42,
    };
    let json = serde_json::to_string(&notif).unwrap();
    let parsed: TransportNotification = serde_json::from_str(&json).unwrap();
    match parsed {
        TransportNotification::MessagePosted { lamport_ts, .. } => {
            assert_eq!(lamport_ts, 42);
        }
        _ => panic!("wrong variant"),
    }
}
