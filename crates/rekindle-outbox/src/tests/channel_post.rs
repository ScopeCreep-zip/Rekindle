//! The `channel_post` stream: settled by its write's settle, never attempted.

use super::*;

#[tokio::test]
async fn a_channel_post_stream_settles_from_its_write_settle() {
    let o = outbox();
    o.enqueue(entry(StreamKind::ChannelPost, "rk:4", "p1"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::ChannelPost, "rk:4", "p2"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::ChannelPost, "rk:5", "p3"), ())
        .await
        .unwrap();
    let settled = (o
        .settle_stream(StreamKind::ChannelPost, "rk:4", Ok(()))
        .await)
        .unwrap();
    let ids: Vec<_> = settled.iter().map(|r| r.client_id.as_str()).collect();
    assert_eq!(ids, ["p1", "p2"]);
    assert_eq!(state_of(&o, "p3"), Some(SendState::Queued));
    let failed = (o.settle_stream(StreamKind::ChannelPost, "rk:5", Err("superseded".into())))
        .await
        .unwrap();
    assert_eq!(failed.len(), 1);
    let s = o.deps().store.lock();
    assert!(s.rows.is_empty());
    let states: Vec<_> = s.settled.iter().map(|e| (e.2.as_str(), e.3)).collect();
    assert_eq!(
        states,
        [
            ("p1", SendState::Sent),
            ("p2", SendState::Sent),
            ("p3", SendState::Failed)
        ]
    );
}

#[tokio::test]
async fn a_channel_post_gets_no_attempt_pass_and_fails_at_its_deadline() {
    let o = outbox();
    o.enqueue(entry(StreamKind::ChannelPost, "rk:4", "p1"), ())
        .await
        .unwrap();
    let wake = o.pump().await.unwrap();
    assert!(o.deps().store.lock().attempted.is_empty());
    let deadline = o.deps().store.lock().rows[0].deadline;
    assert_eq!(wake, Some(deadline), "no backoff pass, only its deadline");
    assert_eq!(o.flush(StreamKind::ChannelPost).await.unwrap(), 0);
    advance(&o, deadline);
    o.pump().await.unwrap();
    assert!(o.deps().store.lock().attempted.is_empty());
    assert_eq!(state_of(&o, "p1"), Some(SendState::Failed));
    // The user's Retry makes its write again under its id; the engine
    // still never attempts it.
    assert!(o.retry("p1").await.unwrap());
    o.pump().await.unwrap();
    assert!(o.deps().store.lock().attempted.is_empty());
    assert_eq!(o.deps().store.lock().reissued, ["p1"]);
    assert_eq!(state_of(&o, "p1"), Some(SendState::Queued));
    // A write that cannot be made again fails the row at once.
    o.enqueue(entry(StreamKind::ChannelPost, "rk:9", "gone1"), ())
        .await
        .unwrap();
    advance(&o, deadline);
    o.pump().await.unwrap();
    assert!(o.retry("gone1").await.unwrap());
    assert_eq!(state_of(&o, "gone1"), Some(SendState::Failed));
}
