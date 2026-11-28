use std::sync::atomic::Ordering;

use super::{ControlIngressQueue, CONTROL_INGRESS_QUEUE_MAX};

#[test]
fn fifo_order_preserved() {
    let q = ControlIngressQueue::new();
    q.push(1);
    q.push(2);
    q.push(3);
    let mut got = Vec::new();
    while q.depth() > 0 {
        got.push(q.inner.lock().pop_front().unwrap());
    }
    assert_eq!(got, vec![1, 2, 3]);
}

#[test]
fn overflow_drops_oldest_and_counts() {
    let q = ControlIngressQueue::new();
    for i in 0..CONTROL_INGRESS_QUEUE_MAX as u64 {
        q.push(i);
    }
    assert_eq!(q.depth(), CONTROL_INGRESS_QUEUE_MAX);
    assert_eq!(q.drops.load(Ordering::Relaxed), 0);

    // One past capacity: the oldest (tag 0) is evicted, the newest
    // lands, depth stays pinned at the ceiling, counter ticks once.
    q.push(9999);
    assert_eq!(q.depth(), CONTROL_INGRESS_QUEUE_MAX);
    assert_eq!(q.drops.load(Ordering::Relaxed), 1);
    let tags: Vec<u64> = q.inner.lock().iter().copied().collect();
    assert_eq!(tags.first(), Some(&1), "oldest element must be dropped");
    assert_eq!(tags.last(), Some(&9999), "newest element must land");
}

#[test]
fn sustained_overflow_keeps_counting() {
    let q = ControlIngressQueue::new();
    for i in 0..(CONTROL_INGRESS_QUEUE_MAX as u64 + 100) {
        q.push(i);
    }
    assert_eq!(q.depth(), CONTROL_INGRESS_QUEUE_MAX);
    assert_eq!(q.drops.load(Ordering::Relaxed), 100);
}

#[tokio::test]
async fn pop_wakes_on_push() {
    let q = std::sync::Arc::new(ControlIngressQueue::new());
    let q2 = std::sync::Arc::clone(&q);
    let waiter = tokio::spawn(async move { q2.pop().await });
    // Give the waiter a moment to park on notified().
    tokio::task::yield_now().await;
    q.push(42);
    let got = tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
        .await
        .expect("pop must wake")
        .expect("task join");
    assert_eq!(got, 42);
}
