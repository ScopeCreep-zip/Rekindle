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
    // Biased, stop first: work ready at the instant of the stop is dropped.
    tokio::select! {
        biased;
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
            // Biased: stop, then the source change, then events. A flood of
            // events cannot hold off a lock or a new unlock; the change
            // arm is ready once per replacement, so events never starve.
            tokio::select! {
                biased;
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
/// a new epoch makes every subscriber reload (plan D3.2); the session's
/// standing events survive it.
pub(crate) async fn run_forwarder(ctx: std::sync::Arc<crate::daemon::dispatch::DaemonContext>) {
    let source = ctx.event_watch_tx.subscribe();
    let journal = std::sync::Arc::clone(&ctx.journal);
    follow(
        ctx.shutdown.token(),
        source,
        |event| {
            ctx.publish_event(event);
            std::future::ready(())
        },
        |_| journal.resync(),
    )
    .await;
}

#[cfg(test)]
#[path = "event_forward_tests.rs"]
mod tests;
