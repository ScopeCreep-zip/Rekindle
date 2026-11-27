use super::*;
use veilid_core::{VeilidLog, VeilidLogLevel};

// `Log` is a variant this queue never actually carries (the dispatch
// loop only pushes `ValueChange`/`AppCall`), but it is the one
// trivially-constructible `VeilidUpdate` — perfect for exercising the
// queue's FIFO/overflow mechanics with a distinguishable tag.
fn update(tag: u64) -> VeilidUpdate {
    VeilidUpdate::Log(Box::new(VeilidLog {
        log_level: VeilidLogLevel::Info,
        message: tag.to_string(),
        backtrace: None,
    }))
}

fn tag_of(update: &VeilidUpdate) -> u64 {
    match update {
        VeilidUpdate::Log(log) => log.message.parse().expect("tag parse"),
        _ => panic!("unexpected update variant in test"),
    }
}

#[test]
fn fifo_order_preserved() {
    let q = ControlIngressQueue::new();
    q.push(update(1));
    q.push(update(2));
    q.push(update(3));
    let mut got = Vec::new();
    while q.depth() > 0 {
        got.push(tag_of(&q.inner.lock().pop_front().unwrap()));
    }
    assert_eq!(got, vec![1, 2, 3]);
}

#[test]
fn overflow_drops_oldest_and_counts() {
    let q = ControlIngressQueue::new();
    for i in 0..CONTROL_INGRESS_QUEUE_MAX as u64 {
        q.push(update(i));
    }
    assert_eq!(q.depth(), CONTROL_INGRESS_QUEUE_MAX);
    assert_eq!(q.drops.load(Ordering::Relaxed), 0);

    // One past capacity: the oldest (tag 0) is evicted, the newest
    // lands, depth stays pinned at the ceiling, counter ticks once.
    q.push(update(9999));
    assert_eq!(q.depth(), CONTROL_INGRESS_QUEUE_MAX);
    assert_eq!(q.drops.load(Ordering::Relaxed), 1);
    let tags: Vec<u64> = q.inner.lock().iter().map(tag_of).collect();
    assert_eq!(tags.first(), Some(&1), "oldest element must be dropped");
    assert_eq!(tags.last(), Some(&9999), "newest element must land");
}

#[test]
fn sustained_overflow_keeps_counting() {
    let q = ControlIngressQueue::new();
    for i in 0..(CONTROL_INGRESS_QUEUE_MAX as u64 + 100) {
        q.push(update(i));
    }
    assert_eq!(q.depth(), CONTROL_INGRESS_QUEUE_MAX);
    assert_eq!(q.drops.load(Ordering::Relaxed), 100);
}

#[tokio::test]
async fn pop_wakes_on_push() {
    let q = std::sync::Arc::new(ControlIngressQueue::new());
    let q2 = std::sync::Arc::clone(&q);
    let waiter = tokio::spawn(async move { tag_of(&q2.pop().await) });
    // Give the waiter a moment to park on notified().
    tokio::task::yield_now().await;
    q.push(update(42));
    let got = tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
        .await
        .expect("pop must wake")
        .expect("task join");
    assert_eq!(got, 42);
}
