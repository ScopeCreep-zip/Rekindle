//! The one delivery-state vocabulary (D22): `queued < sent < delivered <
//! read`, plus `failed`, merged monotonically. A lower state never
//! overwrites a higher one; `failed` replaces only an unsettled state
//! (`queued`, or `sent` and waiting for the peer). `delivered` and `read`
//! come only from the peer ([`StateInput::Peer`]), never from a local send.

use rekindle_types::message::SendState;

/// What the peer acknowledged (`DmAck`, `ReadUpTo`), lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PeerAck {
    Delivered,
    Read,
}

/// One input to a row's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateInput {
    /// A send attempt left this device (the transport's `Ok`).
    LocalSent,
    /// The peer acknowledged the row.
    Peer(PeerAck),
    /// The row's deadline passed, or its stream wedged it.
    Failed,
}

fn rank(state: SendState) -> Option<u8> {
    match state {
        SendState::Queued => Some(0),
        SendState::Sent => Some(1),
        SendState::Delivered => Some(2),
        SendState::Read => Some(3),
        SendState::Failed => None,
    }
}

/// Whether `state` still waits on this device or the peer.
#[must_use]
pub fn is_unsettled(state: SendState) -> bool {
    matches!(state, SendState::Queued | SendState::Sent)
}

/// `current` after `input`.
#[must_use]
pub fn merge(current: SendState, input: StateInput) -> SendState {
    let incoming = match input {
        StateInput::LocalSent => SendState::Sent,
        StateInput::Peer(PeerAck::Delivered) => SendState::Delivered,
        StateInput::Peer(PeerAck::Read) => SendState::Read,
        StateInput::Failed => {
            return if is_unsettled(current) {
                SendState::Failed
            } else {
                current
            };
        }
    };
    match rank(current) {
        // The peer holding the row outranks a local failure; a local send
        // does not.
        None if matches!(input, StateInput::Peer(_)) => incoming,
        Some(have) if rank(incoming) > Some(have) => incoming,
        _ => current,
    }
}
