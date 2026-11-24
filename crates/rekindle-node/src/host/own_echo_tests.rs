//! Our own 1:1 message reaches every window as its stored row (plan E6.11,
//! E2.5): `send_dm` publishes `DirectMessageSent` once, through the
//! journal, so a window that never rendered the send (or reloaded its
//! history during the send's review) upserts it by its client id.

use rekindle_transport::operations::dm;
use rekindle_types::subscription_events::{ChannelMessageEvent, StreamFrame, SubscriptionEvent};

use super::adapters::friends::HostFriends;
use super::direct_tests::{ann, befriend};

#[tokio::test]
async fn our_send_publishes_its_stored_row_once() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let bob = rekindle_crypto::Identity::generate().public_key_bytes();
    befriend(&host, owner, bob).await;
    let friends = HostFriends::of(&host.ctx);
    let peer = hex::encode(bob);
    let mut events = host.ctx.journal.subscribe(None);

    let sent = dm::send_dm(&friends, &peer, "gg", None, false)
        .await
        .unwrap();

    let mut own = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        if let StreamFrame::Event(seq) = frame {
            if let SubscriptionEvent::ChannelMessage(
                e @ ChannelMessageEvent::DirectMessageSent { .. },
            ) = seq.event
            {
                own.push(e);
            }
        }
    }
    assert_eq!(own.len(), 1, "one own row per send: {own:?}");
    let ChannelMessageEvent::DirectMessageSent {
        conversation_id,
        reply_to_id,
        message,
    } = &own[0]
    else {
        unreachable!("filtered above")
    };
    assert_eq!(conversation_id, &peer);
    assert_eq!(reply_to_id, &None);
    assert_eq!(
        message, &sent,
        "the event carries the row the send answered"
    );
    assert!(message.is_own);
    assert_eq!(message.body, "gg");
    let history = dm::history(&friends, &peer, 50, None).await.unwrap();
    let stored = history
        .iter()
        .find(|m| m.message_id == message.message_id)
        .expect("the row is stored");
    assert_eq!(stored.id, message.id, "the stored row's id");
    assert_eq!(stored.body, message.body);
}
