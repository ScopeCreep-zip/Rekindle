//! The dispatch loop's last phase (live A/B round 3 R3-6): once the node
//! is stopping, the loop no longer dispatches, but it keeps receiving the
//! node's events until Veilid's last one, so none is dropped on a gone
//! consumer ("Veilid update channel full — dropped event" for Attachment
//! and Shutdown).
//!
//! Veilid 0.5.8 sends updates through shutdown: `VeilidAPI::shutdown`
//! runs `api_shutdown` (`veilid_api/api.rs:84-104`,
//! `core_context.rs:439-464`), whose context shutdown detaches (Attachment
//! updates as each routing domain goes down), runs `pre_terminate` and
//! `terminate`, logs "Veilid API shutdown complete" and then sends
//! `VeilidUpdate::Shutdown` as its final update (`core_context.rs:194-230`).
//! The update callback sends without waiting (`rekindle_protocol::ports`
//! `VeilidPorts::start`), so by the time `shutdown()` returns every update
//! is queued.

use rekindle_records::port::PortEvent;
use tokio::sync::mpsc;
use tracing::debug;

use crate::shared::{AttachmentState, SharedState};

/// Receive `events` after the stop signal until [`PortEvent::Shutdown`],
/// the channel's close, or `stopped` (the node's owner stopped the network,
/// or has none to stop): then take what is queued without waiting, through
/// `Shutdown`. Nothing is dispatched; an attachment still updates `shared`.
pub(crate) async fn drain_through_shutdown(
    events: &mut mpsc::Receiver<PortEvent>,
    stopped: &mut mpsc::Receiver<()>,
    shared: &SharedState,
) {
    loop {
        tokio::select! {
            biased;
            event = events.recv() => match event {
                Some(PortEvent::Shutdown) | None => return,
                Some(event) => record(shared, event),
            },
            _ = stopped.recv() => break,
        }
    }
    while let Ok(event) = events.try_recv() {
        if matches!(event, PortEvent::Shutdown) {
            return;
        }
        record(shared, event);
    }
}

fn record(shared: &SharedState, event: PortEvent) {
    match event {
        PortEvent::Attachment {
            state,
            is_attached,
            public_internet_ready,
            reliable_peers,
            live_peers,
            network_size,
            median_latency_us,
        } => {
            let att_state = AttachmentState::from_veilid_string(&state);
            shared.set_attachment(att_state, is_attached, public_internet_ready);
            shared.set_network_health(reliable_peers, live_peers, network_size, median_latency_us);
        }
        other => debug!(event = ?other, "event at shutdown, not dispatched"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn detached() -> PortEvent {
        PortEvent::Attachment {
            state: "detached".into(),
            is_attached: false,
            public_internet_ready: false,
            reliable_peers: 0,
            live_peers: 0,
            network_size: 0,
            median_latency_us: 0,
        }
    }

    /// Veilid's updates during its shutdown (Attachment ×2, then Shutdown,
    /// the live R3-6 sequence) all find a consumer, on a channel of one, and
    /// the drain ends at `Shutdown` with no `stopped` signal.
    #[tokio::test]
    async fn the_drain_takes_every_update_through_shutdown() {
        let (tx, mut events) = mpsc::channel(1);
        let (_stop_tx, mut stopped) = mpsc::channel::<()>(1);
        let shared = SharedState::new();
        let drain = {
            let shared = Arc::clone(&shared);
            tokio::spawn(async move {
                drain_through_shutdown(&mut events, &mut stopped, &shared).await;
            })
        };
        for event in [detached(), detached(), PortEvent::Shutdown] {
            // Veilid's callback never waits (`try_send`): yield until the
            // one slot is free, as the core thread's later update would.
            let mut event = event;
            let mut tries = 0;
            loop {
                match tx.try_send(event) {
                    Ok(()) => break,
                    Err(mpsc::error::TrySendError::Full(back)) if tries < 100 => {
                        event = back;
                        tries += 1;
                        tokio::task::yield_now().await;
                    }
                    Err(e) => panic!("update dropped at shutdown: {e}"),
                }
            }
        }
        drain.await.expect("the drain ends at Shutdown");
    }

    /// With the network already stopped, the queued updates are taken
    /// without waiting for more.
    #[tokio::test]
    async fn the_drain_ends_once_the_network_stopped() {
        let (tx, mut events) = mpsc::channel(4);
        let (stop_tx, mut stopped) = mpsc::channel::<()>(1);
        tx.try_send(detached()).expect("queued");
        stop_tx.try_send(()).expect("stopped");
        drain_through_shutdown(&mut events, &mut stopped, &SharedState::new()).await;
        assert!(events.try_recv().is_err(), "the queue was emptied");
        drop(tx);
    }
}
