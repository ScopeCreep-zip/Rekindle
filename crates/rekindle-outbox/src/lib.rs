//! `rekindle-outbox` (plan E2.5, D22): the one owner of every send intent.
//! A send is a row in an ordered stream, written with its local echo in
//! one transaction, attempted by `next_at` until it settles. Its state is
//! one vocabulary (`queued < sent < delivered < read`, plus `failed`),
//! merged monotonically ([`state::merge`]); `delivered` comes only from
//! the peer. Each row carries a deadline (`DELIVERY_HORIZON`, D24); past
//! it the row fails and wedges its stream until Retry (same client id) or
//! Discard. The engine runs over [`OutboxDeps`]; it holds no SQL and no
//! transport.

mod appender;
mod deps;
mod engine;
mod receipt;
pub mod state;
mod stream;

pub use deps::{
    holds_back, AttemptOutcome, NewOutboxRow, OutboxDeps, OutboxError, OutboxRow, RowUpdate,
    StateChange,
};
pub use engine::{Deadline, Enqueued, Entry, Outbox, DEADLINE_REASON, OUTBOX_DEADLINE};
pub use state::{PeerAck, StateInput};
pub use stream::{OutboxRetry, StreamKind};

#[cfg(test)]
mod tests;
