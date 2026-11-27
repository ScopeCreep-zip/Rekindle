use super::*;

fn gossip(is_video: bool, tag: u8) -> IngressItem {
    IngressItem::Gossip {
        signed: SignedEnvelope {
            message_id: [7u8; 16],
            community_id: format!("c{tag}"),
            sender_pseudonym: "p".into(),
            envelope_bytes: vec![tag],
            signature: Vec::new(),
            ttl: 0,
        },
        is_video,
    }
}

fn tag_of(item: &IngressItem) -> u8 {
    match item {
        IngressItem::Gossip { signed, .. } => signed.envelope_bytes[0],
        IngressItem::Direct(b) => b[0],
    }
}

#[test]
fn fifo_order_preserved() {
    let q = GossipIngressQueue::new();
    q.push(gossip(false, 1));
    q.push(IngressItem::Direct(vec![2]));
    q.push(gossip(true, 3));
    let mut got = Vec::new();
    while q.depth() > 0 {
        got.push(tag_of(&q.inner.lock().pop_front().unwrap()));
    }
    assert_eq!(got, vec![1, 2, 3]);
}

#[test]
fn overflow_evicts_oldest_video_first() {
    let q = GossipIngressQueue::new();
    // Fill: one old video item buried among control.
    q.push(gossip(false, 0));
    q.push(gossip(true, 1)); // ← oldest video
    for _ in 2..INGRESS_QUEUE_MAX {
        q.push(gossip(false, 2));
    }
    assert_eq!(q.depth(), INGRESS_QUEUE_MAX);
    // Overflow with a control item: the old video is shed, the
    // control item lands.
    q.push(gossip(false, 9));
    assert_eq!(q.depth(), INGRESS_QUEUE_MAX);
    assert_eq!(q.video_drops.load(Ordering::Relaxed), 1);
    assert_eq!(q.other_drops.load(Ordering::Relaxed), 0);
    let has_video = q.inner.lock().iter().any(IngressItem::is_video);
    assert!(!has_video, "the only video item must be the one evicted");
}

#[test]
fn control_survives_video_flood() {
    let q = GossipIngressQueue::new();
    q.push(gossip(false, 7)); // the one control envelope
    for _ in 1..INGRESS_QUEUE_MAX {
        q.push(gossip(true, 1));
    }
    // Flood far past capacity with more video.
    for _ in 0..100 {
        q.push(gossip(true, 1));
    }
    assert_eq!(q.depth(), INGRESS_QUEUE_MAX);
    assert_eq!(q.video_drops.load(Ordering::Relaxed), 100);
    let control_alive = q.inner.lock().iter().any(|i| !i.is_video());
    assert!(control_alive, "control envelope must outlive the flood");
}

#[test]
fn all_control_overflow_refuses_newest() {
    let q = GossipIngressQueue::new();
    for _ in 0..INGRESS_QUEUE_MAX {
        q.push(gossip(false, 1));
    }
    // Incoming video against an all-control queue: incoming dropped.
    q.push(gossip(true, 2));
    assert_eq!(q.depth(), INGRESS_QUEUE_MAX);
    assert_eq!(q.video_drops.load(Ordering::Relaxed), 1);
    // Incoming control against an all-control queue: incoming dropped.
    q.push(gossip(false, 3));
    assert_eq!(q.depth(), INGRESS_QUEUE_MAX);
    assert_eq!(q.other_drops.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn pop_wakes_on_push() {
    let q = std::sync::Arc::new(GossipIngressQueue::new());
    let q2 = std::sync::Arc::clone(&q);
    let waiter = tokio::spawn(async move { tag_of(&q2.pop().await) });
    // Give the waiter a moment to park on notified().
    tokio::task::yield_now().await;
    q.push(gossip(false, 42));
    let got = tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
        .await
        .expect("pop must wake")
        .expect("task join");
    assert_eq!(got, 42);
}
