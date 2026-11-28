//! Disappearing messages on the Host (plan E7.1): the timer is stored and
//! told to the friend on our DM stream, our next message carries it and
//! expires from its send, a received one's timer starts at its display,
//! and the expiry pass deletes what is due with its outbox row.

use rekindle_codec::capnp_codec::direct::{DirectBody, DmText};
use rekindle_transport::operations::direct::replay_unprocessed;
use rekindle_transport::operations::dm::{self, disappearing};
use rekindle_types::message::ConversationType;

use super::adapters::friends::HostFriends;
use super::direct_tests::{ann, befriend, pending};
use super::runtime::Host;

/// `(expire_seconds, expire_at)` of each message with `peer`, oldest first.
async fn expiries(host: &Host, owner: [u8; 32], peer: [u8; 32]) -> Vec<(u32, Option<i64>)> {
    let (o, p) = (hex::encode(owner), hex::encode(peer));
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::messages::dm_history(conn, &o, &p, 50, None))
        .await
        .unwrap()
        .into_iter()
        .map(|m| (m.expire_seconds, m.expire_at))
        .collect()
}

/// Our outbox rows on the DM stream to `peer`.
async fn outbox_rows(host: &Host, owner: [u8; 32], peer: &str) -> usize {
    let (o, stream) = (hex::encode(owner), format!("dm:{peer}"));
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::outbox::stream_rows(conn, &o, &stream))
        .await
        .unwrap()
        .len()
}

#[tokio::test]
async fn our_timer_is_told_and_our_message_expires_from_its_send() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let bob = rekindle_crypto::Identity::generate().public_key_bytes();
    befriend(&host, owner, bob).await;
    let friends = HostFriends::of(&host.ctx);
    let peer = hex::encode(bob);

    let before = outbox_rows(&host, owner, &peer).await;
    let read =
        |s| async move { disappearing::current(s, &hex::encode(bob), ConversationType::Dm).await };
    assert_eq!(read(&friends).await.unwrap().seconds, 0, "off until set");
    disappearing::set(&friends, &peer, ConversationType::Dm, 1)
        .await
        .unwrap();
    assert_eq!(
        read(&friends).await.unwrap().seconds,
        1,
        "the window's control reads it"
    );
    assert_eq!(
        outbox_rows(&host, owner, &peer).await,
        before + 1,
        "the timer goes to Bob"
    );
    assert!(
        disappearing::set(&friends, &peer, ConversationType::Dm, 2_419_201)
            .await
            .is_err()
    );

    let sent = dm::send_dm(&friends, &peer, "gone soon", None, false)
        .await
        .unwrap();
    assert_eq!(sent.expire_seconds, 1);
    assert_eq!(sent.expire_at, Some(sent.timestamp + 1_000));

    // Due after its second: the pass deletes it and its send.
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    assert_eq!(disappearing::expire_due(&friends).await.unwrap(), None);
    assert!(expiries(&host, owner, bob).await.is_empty());
    assert_eq!(
        outbox_rows(&host, owner, &peer).await,
        before + 1,
        "only the timer's row is left"
    );
}

#[tokio::test]
async fn a_received_timer_starts_when_the_window_displays_it() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let bob = rekindle_crypto::Identity::generate().public_key_bytes();
    befriend(&host, owner, bob).await;
    let friends = HostFriends::of(&host.ctx);
    let text = DirectBody::DmText(DmText {
        client_id: [3; 16],
        body: "psst".into(),
        reply_to: None,
        expire_seconds: 60,
    });
    pending(&host, owner, bob, 0, &text).await;
    replay_unprocessed(&friends).await;
    assert_eq!(
        expiries(&host, owner, bob).await,
        [(60, None)],
        "unread: not started"
    );

    dm::mark_read(&friends, &hex::encode(bob), None)
        .await
        .unwrap();
    let started = expiries(&host, owner, bob).await;
    let at = started[0].1.expect("displayed: started");
    let now = rekindle_utils::timestamp_ms_i64();
    assert!((now + 50_000..=now + 60_000).contains(&at), "{at} vs {now}");
}

/// A received group message's timer starts when the group window marks it
/// read (`GroupDmMarkRead`), not when its history loads (plan E7.1).
#[tokio::test]
async fn a_group_timer_starts_at_mark_read_not_at_history_load() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let (o, group) = (hex::encode(owner), "0f".repeat(16));
    let (og, gid) = (o.clone(), group.clone());
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            rekindle_db::repo::messages::insert_dm(
                conn,
                &rekindle_db::repo::messages::NewDm {
                    owner_key: &og,
                    peer_key: &gid,
                    conversation_type: ConversationType::GroupDm,
                    expire_seconds: 60,
                    expire_at: None,
                    sender_key: &"b".repeat(64),
                    body: "psst",
                    timestamp: 1_000,
                    is_read: false,
                    message_id: "m-1",
                    reply_to: None,
                    send_state: rekindle_types::message::SendState::Delivered,
                    stream_seq: None,
                },
            )
        })
        .await
        .unwrap();
    for _ in 0..2 {
        let shown = rekindle_transport::operations::group_dm::messages(&friends, &group, 50, None)
            .await
            .unwrap();
        assert_eq!(shown[0].expire_at, None, "a history load starts nothing");
    }
    rekindle_transport::operations::group_dm::mark_read(&friends, &group)
        .await
        .unwrap();
    let shown = rekindle_transport::operations::group_dm::messages(&friends, &group, 50, None)
        .await
        .unwrap();
    assert!(shown[0].expire_at.is_some(), "marked read: started");
}

/// Plan E7.1 (the UI slice's gap): a received timer change is stored as a
/// timeline notice row, so a reopened window shows it; the event names
/// that row; the same value again stores and publishes nothing (Signal's
/// `DataMessageProcessor` ignores an unchanged timer).
#[tokio::test]
async fn a_received_timer_change_is_a_stored_notice_once() {
    use rekindle_codec::capnp_codec::direct::DisappearingTimer;
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let mut events = host.ctx.journal.subscribe(None);
    let bob = rekindle_crypto::Identity::generate().public_key_bytes();
    befriend(&host, owner, bob).await;
    let friends = HostFriends::of(&host.ctx);
    let timer = DirectBody::DisappearingTimer(DisappearingTimer {
        seconds: 300,
        group_id: None,
        group_version: None,
    });
    pending(&host, owner, bob, 0, &timer).await;
    pending(&host, owner, bob, 1, &timer).await;
    replay_unprocessed(&friends).await;

    let shown = dm::history(&friends, &hex::encode(bob), 50, None)
        .await
        .unwrap();
    let notices: Vec<_> = shown
        .iter()
        .filter(|m| m.timer_notice_seconds.is_some())
        .collect();
    assert_eq!(notices.len(), 1, "one notice for one change: {shown:?}");
    let notice = notices[0];
    assert_eq!(notice.timer_notice_seconds, Some(300));
    assert_eq!(notice.sender_id, hex::encode(bob));
    assert_eq!((notice.expire_seconds, notice.expire_at), (300, None));

    let id = notice.message_id.clone().expect("named");
    let mut published = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        let text = format!("{frame:?}");
        if text.contains("DisappearingTimerChanged") {
            published.push(text);
        }
    }
    assert_eq!(published.len(), 1, "{published:?}");
    assert!(published[0].contains(&id), "the event names the row");
}
