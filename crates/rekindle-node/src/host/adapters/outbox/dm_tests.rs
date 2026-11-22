//! The DM outbox adapter's send-status projection.

use rekindle_types::message::SendState;

use super::*;

#[test]
fn only_dm_rows_publish_a_send_status() {
    let change = StateChange {
        stream: "dm:bob".into(),
        stream_seq: 1,
        client_id: "c1".into(),
        kind: StreamKind::Dm,
        state: SendState::Delivered,
        wedge_reason: None,
    };
    let Some(SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DmSendStatus {
        conversation_id,
        client_id,
        state,
    })) = send_status(&change)
    else {
        panic!("a dm row publishes DmSendStatus");
    };
    assert_eq!(
        (conversation_id.as_str(), client_id.as_str(), state),
        ("bob", "c1", SendState::Delivered)
    );
    let other = StateChange {
        kind: StreamKind::DhtWrite,
        stream: "dht_write:r".into(),
        ..change
    };
    assert!(send_status(&other).is_none());
}
