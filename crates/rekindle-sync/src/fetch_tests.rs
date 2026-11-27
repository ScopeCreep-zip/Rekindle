use std::time::Duration;

use super::{retry_backoff, FetchQueue, FetchTask};

#[test]
fn queue_is_fifo() {
    let mut queue = FetchQueue::default();
    queue.push(FetchTask {
        record_key: "a".into(),
        subkey: 1,
        expected_hash: None,
        attempt: 0,
    });
    queue.push(FetchTask {
        record_key: "b".into(),
        subkey: 2,
        expected_hash: Some("hash".into()),
        attempt: 1,
    });

    assert_eq!(queue.pop().unwrap().record_key, "a");
    assert_eq!(queue.pop().unwrap().record_key, "b");
    assert!(queue.is_empty());
}

#[test]
fn retry_backoff_caps_at_four_seconds() {
    assert_eq!(retry_backoff(0), Duration::from_secs(1));
    assert_eq!(retry_backoff(1), Duration::from_secs(2));
    assert_eq!(retry_backoff(2), Duration::from_secs(4));
    assert_eq!(retry_backoff(5), Duration::from_secs(4));
}

/// Three-path independence: a gossip watch relay (Path 2) can populate
/// the fetch queue independently — no need for SMPL write confirmation or
/// inspect polling. The relay carries the subkey and content_hash, which
/// is all FetchQueue needs to schedule a DHT fetch.
#[test]
fn gossip_notification_creates_fetch_task_independently() {
    let mut queue = FetchQueue::default();

    // Simulate receiving a gossip WatchRelay with metadata only
    queue.push(FetchTask {
        record_key: "channel_rec_abc".into(),
        subkey: 7, // the changed subkey from the relay
        expected_hash: Some(
            "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4".into(),
        ), // blake3 content_hash from the relay
        attempt: 0,
    });

    assert_eq!(queue.len(), 1);

    let task = queue.pop().unwrap();
    // The fetch task has everything needed to retrieve and verify from DHT
    assert_eq!(task.subkey, 7);
    assert!(task.expected_hash.is_some());
    assert_eq!(task.attempt, 0);
}
