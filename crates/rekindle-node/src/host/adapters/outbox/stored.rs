//! A row's state as stored now, read in the storage call that writes it
//! (plan E2.5, D22): every own row's state moves through the lattice
//! (`rekindle_outbox::state::apply`) against the row as it is, never the
//! row an attempt began from, so a peer's ack that landed meanwhile is
//! never followed by a lower state (Linux real-app round 27, `delivered`
//! then `sent`).

use rekindle_outbox::state::StoredSend;
use rekindle_outbox::PeerAck;
use rekindle_types::message::SendState;

/// A row's stored `(state, peer_ack)` (`repo::outbox::stored_state`, read
/// in the closure that writes it) as the lattice reads it.
pub(super) fn stored_of((state, ack): (SendState, Option<SendState>)) -> StoredSend {
    StoredSend {
        state,
        ack: ack.map(|ack| match ack {
            SendState::Read => PeerAck::Read,
            _ => PeerAck::Delivered,
        }),
    }
}
