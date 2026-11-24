//! The host's event stream (plan D21, D3.2).
//!
//! Every [`SubscriptionEvent`] a host publishes is journaled under a
//! [`Cursor`]: the epoch of the unlock that published it and its sequence
//! number within that epoch. A subscriber that comes back with the last
//! cursor it processed is sent what it missed and then [`StreamFrame::Resumed`];
//! one whose cursor the journal can no longer serve (another epoch, or
//! evicted) gets [`StreamFrame::InvalidSession`] and reloads its state.

use serde::{Deserialize, Serialize};

use super::SubscriptionEvent;

/// Where a subscriber is in the host's stream.
///
/// The epoch is random per unlock and below 2^53, so it survives a process
/// restart without colliding and stays exact as a JavaScript number in the
/// webview; the sequence number counts from 1 within the epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    pub epoch: u64,
    pub seq: u64,
}

/// A journaled event and its cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sequenced {
    pub cursor: Cursor,
    pub event: SubscriptionEvent,
}

/// What a subscription delivers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamFrame {
    /// The next event, boxed: an event is far larger than every other
    /// frame (serde writes a `Box` as its content, so the wire is the same).
    Event(Box<Sequenced>),
    /// The replay of what the subscriber missed is complete; live events
    /// follow from this cursor.
    Resumed(Cursor),
    /// The subscriber's cursor cannot be served (another epoch, evicted,
    /// or the subscriber fell behind): reload state, then subscribe anew.
    InvalidSession,
    /// The host is stopping (plan E7.1). Sent once to every live stream,
    /// whatever its filters, before the bus drains; unsequenced and never
    /// replayed. A client handles it as the end of the stream; on
    /// [`StopReason::DataWiped`] the GUI clears its webview stores and exits.
    HostStopping { reason: StopReason },
}

/// Why the host is stopping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    /// A stop was asked for (an IPC `Shutdown`, a signal, the app quitting).
    Requested,
    /// The quick wipe or a factory reset took the data root.
    DataWiped,
    /// The host stops on a fault (a handler panicked, its subscriber failed).
    Fault,
}

#[cfg(test)]
mod tests {
    use super::{StopReason, StreamFrame};

    /// The stop frame's wire shape, as the bus and the GUI carry it.
    #[test]
    fn the_stop_frame_serializes_as_its_golden_sample() {
        let frame = StreamFrame::HostStopping {
            reason: StopReason::DataWiped,
        };
        assert_eq!(
            serde_json::to_value(&frame).expect("json"),
            serde_json::json!({ "hostStopping": { "reason": "dataWiped" } })
        );
    }
}
