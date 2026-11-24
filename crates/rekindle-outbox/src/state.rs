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
        SendState::AwaitingKey => Some(0),
        SendState::Queued => Some(1),
        SendState::Sent => Some(2),
        SendState::Delivered => Some(3),
        SendState::Read => Some(4),
        SendState::Failed => None,
    }
}

/// Whether `state` still waits on this device (a channel post waiting for
/// its key, plan E3.2 S7l), or the peer.
#[must_use]
pub fn is_unsettled(state: SendState) -> bool {
    matches!(
        state,
        SendState::AwaitingKey | SendState::Queued | SendState::Sent
    )
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

/// A row as stored now: its state, and the peer's ack noted on it while
/// its send had not landed (`queued`, live A/B finding 42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredSend {
    pub state: SendState,
    pub ack: Option<PeerAck>,
}

impl StoredSend {
    /// The state the user sees: a noted ack is shown at once (D22).
    #[must_use]
    pub fn shown(self) -> SendState {
        match self.ack {
            Some(ack) => merge(self.state, StateInput::Peer(ack)),
            None => self.state,
        }
    }
}

/// How a write moved the state a row shows: published only when changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moved {
    pub before: SendState,
    pub after: SendState,
}

impl Moved {
    #[must_use]
    pub fn changed(self) -> bool {
        self.before != self.after
    }
}

/// Which write a host applies to a stored row ([`apply`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The engine's decision after an attempt, a failure or a deadline:
    /// the target merges forward ([`merge`]); a `queued` target moves
    /// nothing.
    Update,
    /// The user's Retry: `failed` (or `queued`) goes back to `queued`, the
    /// one move down the lattice.
    Retry,
    /// The row settles at the merged state and leaves its stream.
    Settle,
}

/// What the host stores for the row ([`apply`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write {
    /// Keep the row, at this state.
    Store(SendState),
    /// Project this state and remove the row.
    Settle(SendState),
}

fn input_of(target: SendState) -> Option<StateInput> {
    match target {
        SendState::Sent => Some(StateInput::LocalSent),
        SendState::Delivered => Some(StateInput::Peer(PeerAck::Delivered)),
        SendState::Read => Some(StateInput::Peer(PeerAck::Read)),
        SendState::Failed => Some(StateInput::Failed),
        SendState::Queued | SendState::AwaitingKey => None,
    }
}

/// `target` written to the row as it is `stored` now, through the lattice,
/// in the host's one storage call that read it: the write to store, and how
/// the shown state moved. A row whose send left this device (past
/// `queued`) with the peer's ack already noted settles at that ack, as the
/// landing does ([`crate::OutboxDeps::land`]).
#[must_use]
pub fn apply(stored: StoredSend, target: SendState, step: Step) -> (Write, Moved) {
    let state = match step {
        Step::Retry if matches!(stored.state, SendState::Failed | SendState::Queued) => {
            SendState::Queued
        }
        Step::Retry => stored.state,
        Step::Update | Step::Settle => {
            input_of(target).map_or(stored.state, |input| merge(stored.state, input))
        }
    };
    let after = StoredSend {
        state,
        ack: stored.ack,
    }
    .shown();
    let left = !matches!(state, SendState::Queued | SendState::AwaitingKey);
    let write = if step == Step::Settle || (left && stored.ack.is_some()) {
        Write::Settle(after)
    } else {
        Write::Store(state)
    };
    (
        write,
        Moved {
            before: stored.shown(),
            after,
        },
    )
}
