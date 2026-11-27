use super::*;

#[tokio::test]
async fn send_then_recv_preserves_order() {
    let (tx, mut rx) = media_channel::<u32>(8);
    for i in 0..5 {
        assert!(!tx.send(i), "no eviction below capacity");
    }
    for i in 0..5 {
        assert_eq!(rx.recv().await, Some(i));
    }
    assert_eq!(tx.dropped(), 0);
}

#[tokio::test]
async fn overflow_drops_oldest_keeps_newest() {
    let cap = 4;
    let (tx, mut rx) = media_channel::<u32>(cap);
    // Push 10 items into a queue of capacity 4: the 6 oldest are evicted.
    let overflow = 6u32;
    let total = u32::try_from(cap).unwrap() + overflow;
    for i in 0..total {
        tx.send(i);
    }
    assert_eq!(
        tx.dropped(),
        u64::from(overflow),
        "oldest evicted on overflow"
    );
    assert_eq!(rx.len(), cap, "queue never exceeds capacity");

    // What remains is the NEWEST `cap` items, in order.
    let mut got = Vec::new();
    while let Some(v) = rx.try_recv() {
        got.push(v);
    }
    assert_eq!(
        got,
        vec![overflow, overflow + 1, overflow + 2, overflow + 3]
    );
}

#[tokio::test]
async fn recv_returns_none_after_all_senders_drop() {
    let (tx, mut rx) = media_channel::<u32>(2);
    tx.send(1);
    tx.send(2);
    drop(tx);
    // Buffered items drain first, then closure is observed.
    assert_eq!(rx.recv().await, Some(1));
    assert_eq!(rx.recv().await, Some(2));
    assert_eq!(rx.recv().await, None);
}

#[tokio::test]
async fn recv_wakes_on_concurrent_send() {
    let (tx, mut rx) = media_channel::<u32>(4);
    let handle = tokio::spawn(async move { rx.recv().await });
    // Give the receiver a moment to park on an empty queue.
    tokio::task::yield_now().await;
    tx.send(42);
    assert_eq!(handle.await.unwrap(), Some(42));
}

#[tokio::test]
async fn clone_keeps_channel_open_until_last_sender_drops() {
    let (tx, mut rx) = media_channel::<u32>(2);
    let tx2 = tx.clone();
    tx.send(7);
    drop(tx);
    // A live clone keeps the channel open.
    assert_eq!(rx.recv().await, Some(7));
    tx2.send(8);
    drop(tx2);
    assert_eq!(rx.recv().await, Some(8));
    assert_eq!(rx.recv().await, None);
}
