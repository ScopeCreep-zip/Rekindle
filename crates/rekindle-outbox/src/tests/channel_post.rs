//! A stream settled by its host (the slot write of an attachment
//! `channel_post` row, plan E3.3 until slice 7).

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
