use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use rekindle_events::SessionJournal;
use rekindle_types::media::MediaScope;
use rekindle_types::subscription_events::{StreamFrame, SubscriptionEvent, VideoEvent};
use rekindle_utils::random::OsRandom;
use tokio::sync::mpsc;

use super::queue_frame;

#[tokio::test]
async fn video_hints_shed_with_their_sequence_while_other_facts_backpressure() {
    let journal = SessionJournal::with_rng(Arc::new(OsRandom));
    let mut stream = journal.subscribe(None);
    let scope = MediaScope::DirectCall { peer: "ab".into() };
    let (tx, mut rx) = mpsc::channel(1);
    tx.try_send(vec![0]).unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    for hint in [
        VideoEvent::KeyframeRequest {
            scope: scope.clone(),
            sender: "ab".into(),
            stream_id: "cd".into(),
        },
        VideoEvent::BitrateTarget {
            scope: scope.clone(),
            kbps: 250,
        },
    ] {
        journal.publish(SubscriptionEvent::Video(hint));
        let frame = stream.next().await.unwrap();
        let mut queued = std::pin::pin!(queue_frame(&tx, vec![1], &frame));
        assert_eq!(queued.as_mut().poll(&mut cx), Poll::Ready(true));
    }
    journal.publish(SubscriptionEvent::Video(VideoEvent::CodecIncompatible {
        scope,
        peers: vec![],
    }));
    let frame = stream.next().await.unwrap();
    let StreamFrame::Event(sequenced) = &frame else {
        panic!("expected a journaled event");
    };
    assert_eq!(sequenced.cursor.seq, 3);
    let mut queued = std::pin::pin!(queue_frame(&tx, vec![3], &frame));
    assert_eq!(queued.as_mut().poll(&mut cx), Poll::Pending);
    assert_eq!(rx.recv().await, Some(vec![0]));
    assert!(queued.await);
    assert_eq!(rx.recv().await, Some(vec![3]));
}
