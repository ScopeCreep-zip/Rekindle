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
