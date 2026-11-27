use std::time::Duration;

use rekindle_types::subscription_events::{NetworkEvent, SubscriptionEvent};

use super::*;

fn value_changed(key: &str) -> SubscriptionEvent {
    SubscriptionEvent::Network(NetworkEvent::ValueChanged {
        record_key: key.into(),
        changed_subkeys: vec![],
    })
}

#[tokio::test]
async fn lock_unlock_lock_keeps_the_consumer_alive() {
    let stop = CancellationToken::new();
    let (source, watch) = watch::channel(None);
    let (seen_tx, mut seen) = tokio::sync::mpsc::unbounded_channel();
    let consumer = tokio::spawn({
        let stop = stop.clone();
        async move {
            follow(
                &stop,
                watch,
                |event| {
                    let seen_tx = seen_tx.clone();
                    async move {
                        let _ = seen_tx.send(event);
                    }
                },
                |_| {},
            )
            .await;
        }
    });
    let wait = Duration::from_secs(5);

    for unlock in ["first", "second"] {
        let (events, _keep) = broadcast::channel(8);
        source.send_replace(Some(events.clone()));
        // The consumer subscribes asynchronously; resend until seen.
        let got = tokio::time::timeout(wait, async {
            loop {
                let _ = events.send(value_changed(unlock));
                if let Ok(Some(event)) =
                    tokio::time::timeout(Duration::from_millis(20), seen.recv()).await
                {
                    return event;
                }
            }
        })
        .await
        .expect("consumer saw an event after unlock");
        assert!(matches!(
            got,
            SubscriptionEvent::Network(NetworkEvent::ValueChanged { ref record_key, .. })
                if record_key == unlock
        ));
        source.send_replace(None); // lock
    }

    stop.cancel();
    tokio::time::timeout(wait, consumer)
        .await
        .expect("consumer stops at shutdown")
        .unwrap();
}
