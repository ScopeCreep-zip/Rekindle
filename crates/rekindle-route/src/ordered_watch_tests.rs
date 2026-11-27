use super::OrderedWatch;

#[tokio::test(flavor = "current_thread")]
async fn followers_are_sent_in_subscription_order() {
    let watch = OrderedWatch::new(0u8);
    let order = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let mut tasks = Vec::new();
    for i in 0..8 {
        let mut rx = watch.subscribe();
        let order = std::sync::Arc::clone(&order);
        tasks.push(tokio::spawn(async move {
            rx.changed().await.expect("sender alive");
            order.lock().push(i);
        }));
    }
    tokio::task::yield_now().await;
    watch.send_replace(1);
    for task in tasks {
        task.await.expect("a follower");
    }
    assert_eq!(*order.lock(), (0..8).collect::<Vec<_>>());
    assert!(!watch.send_if_changed(1));
    assert_eq!(watch.get(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn a_follower_of_a_dropped_value_is_closed() {
    let watch = std::sync::Arc::new(OrderedWatch::new(false));
    let follow = watch.follower();
    let mut live = follow.follow();
    watch.send_replace(true);
    assert!(*live.borrow_and_update());
    drop(watch);
    assert!(live.changed().await.is_err());
    assert!(follow.follow().wait_for(|r| *r).await.is_err());
}
