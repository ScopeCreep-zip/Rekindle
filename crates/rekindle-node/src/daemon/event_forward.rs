//! Forwarding the transport's events into the host's journal (plan D3.2).
//!
//! The daemon's unlock publishes its `SubscriptionManager` broadcast sender
//! on a watch channel and its lock clears it. [`follow`] re-subscribes to
//! each new sender, idles while locked, and stops when told to; the
//! forwarder journals every event it sees. If the forwarder falls behind
//! the transport's broadcast, the events it missed were never journaled, so
//! `on_lag` begins a new journal epoch: every subscriber reloads.

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

/// Hand every event from the current source to `on_event`, and each lag
/// (how many events were skipped) to `on_lag`, until `stop` is cancelled
/// or the source's sender is gone.
pub async fn follow<F, Fut>(
    stop: &CancellationToken,
    mut source: EventSource,
    mut on_event: F,
    mut on_lag: impl FnMut(u64),
) where
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
                        tracing::warn!(skipped, "event forwarder fell behind the transport");
                        on_lag(skipped);
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

/// Journal every event the transport's subscription manager emits, across
/// lock cycles. A lag means events were lost before the journal saw them:
/// a new epoch makes every subscriber reload (plan D3.2).
pub(crate) async fn run_forwarder(ctx: std::sync::Arc<crate::daemon::dispatch::DaemonContext>) {
    let source = ctx.event_watch_tx.subscribe();
    let journal = std::sync::Arc::clone(&ctx.journal);
    follow(
        ctx.shutdown.token(),
        source,
        |event| {
            ctx.journal.publish(event);
            std::future::ready(())
        },
        |_| journal.begin(),
    )
    .await;
}

/// The daemon's tier-3 event consumer: each journaled `ValueChanged` that
/// tier 1 missed triggers the matching inbox scan. Each new epoch (an
/// unlock) or a lag ends its stream; it subscribes again, live.
pub(crate) async fn run_consumer(ctx: std::sync::Arc<crate::daemon::dispatch::DaemonContext>) {
    let stop = ctx.shutdown.token().clone();
    loop {
        let mut stream = ctx.journal.subscribe(None);
        loop {
            let frame = tokio::select! {
                () = stop.cancelled() => return,
                frame = stream.next() => frame,
            };
            match frame {
                Some(rekindle_types::subscription_events::StreamFrame::Event(sequenced)) => {
                    if let rekindle_types::subscription_events::SubscriptionEvent::Network(
                        rekindle_types::subscription_events::NetworkEvent::ValueChanged {
                            record_key,
                            ..
                        },
                    ) = sequenced.event
                    {
                        handle_value_changed(&ctx, &record_key).await;
                    }
                }
                Some(rekindle_types::subscription_events::StreamFrame::Resumed(_)) => {}
                Some(rekindle_types::subscription_events::StreamFrame::InvalidSession) | None => {
                    break
                }
            }
        }
    }
}

/// Route a tier-3 `ValueChanged` to our friend inbox scan when it names it.
async fn handle_value_changed(
    ctx: &std::sync::Arc<crate::daemon::dispatch::DaemonContext>,
    record_key: &str,
) {
    let friend_inbox_key = ctx.session.read().as_ref().and_then(|s| {
        (!s.identity.friend_inbox_key.is_empty() && s.identity.friend_inbox_key == *record_key)
            .then(|| s.identity.friend_inbox_key.clone())
    });
    if let Some(inbox_key) = friend_inbox_key {
        tracing::info!("tier 3 poll triggered friend inbox scan");
        crate::daemon::friend_inbox::scan_friend_inbox(
            &ctx.session,
            &ctx.transport,
            &ctx.db,
            &inbox_key,
        )
        .await;
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
}
