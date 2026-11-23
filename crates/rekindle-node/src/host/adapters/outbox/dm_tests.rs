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
        retry: None,
    };
    let Some(SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DmSendStatus {
        conversation_id,
        client_id,
        state,
        retry: None,
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

/// Plan §3.0 lane 1: an unanswered attempt's retry reaches `DmSendStatus`.
#[test]
fn a_retrying_row_publishes_its_retry() {
    let retry = rekindle_types::message::DeliveryRetry {
        attempts: 4,
        next_attempt_at: 9_000,
    };
    let change = StateChange {
        stream: "friend_handshake:bob".into(),
        stream_seq: 1,
        client_id: "FriendRequest:bob".into(),
        kind: StreamKind::FriendHandshake,
        state: SendState::Sent,
        wedge_reason: None,
        retry: Some(retry),
    };
    let Some(SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DmSendStatus {
        conversation_id,
        retry: published,
        ..
    })) = send_status(&change)
    else {
        panic!("our friend request publishes DmSendStatus");
    };
    assert_eq!(conversation_id, "bob");
    assert_eq!(published, Some(retry));
}

/// SR2b: a group send's unanswered attempts publish their retry on
/// `GroupDmEvent::SendStatus`, and its settling change clears it.
#[test]
fn a_retrying_group_row_publishes_its_retry_and_settling_clears_it() {
    let group_change = |retry| StateChange {
        stream: "group_dm:g1".into(),
        stream_seq: 1,
        client_id: "c1".into(),
        kind: StreamKind::GroupDm,
        state: SendState::Sent,
        wedge_reason: None,
        retry,
    };
    let published = |change: &StateChange| match send_status(change) {
        Some(SubscriptionEvent::GroupDm(GroupDmEvent::SendStatus {
            group_id, retry, ..
        })) => {
            assert_eq!(group_id, "g1");
            retry
        }
        other => panic!("a group row publishes GroupDmEvent::SendStatus, got {other:?}"),
    };
    for attempts in 1..=3 {
        let retry = rekindle_types::message::DeliveryRetry {
            attempts,
            next_attempt_at: 9_000 + i64::from(attempts),
        };
        assert_eq!(published(&group_change(Some(retry))), Some(retry));
    }
    assert_eq!(published(&group_change(None)), None, "settled: no retry");
}
