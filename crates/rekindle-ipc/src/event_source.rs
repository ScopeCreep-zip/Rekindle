//! Following the subscription event source across lock cycles.
//!
//! The daemon's unlock publishes its `SubscriptionManager` broadcast sender
//! on a watch channel and its lock clears it. A consumer that subscribed
//! once would die with the first lock; [`follow`] re-subscribes to each new
//! sender, idles while locked, and stops when told to. Both the bus
//! server's event delivery and the daemon's tier-3 consumer follow it.

use std::future::Future;

use rekindle_types::subscription_events::SubscriptionEvent;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};

use tokio_util::sync::CancellationToken;

/// The event source: `Some` while unlocked.
pub type EventSource = watch::Receiver<Option<broadcast::Sender<SubscriptionEvent>>>;

/// Run `work` until it finishes or `stop` is cancelled; `None` means `stop`
/// won and `work` was dropped.
async fn until<T>(stop: &CancellationToken, work: impl Future<Output = T>) -> Option<T> {
    tokio::select! {
        () = stop.cancelled() => None,
        value = work => Some(value),
    }
}

/// Hand every event from the current source to `on_event` until `stop` is
/// cancelled or the source's sender is gone.
pub async fn follow<F, Fut>(stop: &CancellationToken, mut source: EventSource, mut on_event: F)
where
    F: FnMut(SubscriptionEvent) -> Fut,
    Fut: Future<Output = ()>,
{
    loop {
        let sender = source.borrow_and_update().clone();
        let Some(sender) = sender else {
            match until(stop, source.changed()).await {
                Some(Ok(())) => continue,
                Some(Err(_)) | None => return,
            }
        };
        let mut events = sender.subscribe();
        drop(sender);
        tracing::debug!("event source subscribed");
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                // A lock or a new unlock replaced the source.
                changed = source.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    break;
                }
                event = events.recv() => match event {
                    Ok(event) => on_event(event).await,
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "event source: consumer lagging");
                    }
                    // The watch holds a sender while unlocked, so this only
                    // follows a replaced source; the change arm sees it.
                    Err(RecvError::Closed) => {
                        if until(stop, source.changed()).await.is_none_or(|r| r.is_err()) {
                            return;
                        }
                        break;
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
                follow(&stop, watch, |event| {
                    let seen_tx = seen_tx.clone();
                    async move {
                        let _ = seen_tx.send(event);
                    }
                })
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
}
