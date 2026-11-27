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
#[path = "drain_tests.rs"]
mod tests;
