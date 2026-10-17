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
    /// The next event.
    Event(Sequenced),
    /// The replay of what the subscriber missed is complete; live events
    /// follow from this cursor.
    Resumed(Cursor),
    /// The subscriber's cursor cannot be served (another epoch, evicted,
    /// or the subscriber fell behind): reload state, then subscribe anew.
    InvalidSession,
}
