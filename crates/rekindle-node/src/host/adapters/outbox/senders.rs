//! The senders the Host installs per stream kind (plan E2.5): the outbox's
//! attempt dispatches each row to its kind's sender. A kind with no sender
//! installed is not attempted: its rows stay `queued` until one is.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use rekindle_outbox::{AttemptOutcome, OutboxRow, StreamKind};

/// Sends one stream kind's rows (`dm::send`'s deliver, the friend
/// handshake, the record pool's held write, …).
#[async_trait]
pub trait StreamSender: Send + Sync {
    /// Send `row` once.
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome;
}

/// The installed senders, by stream kind.
#[derive(Default)]
pub struct Senders {
    by_kind: RwLock<HashMap<StreamKind, Arc<dyn StreamSender>>>,
}

impl Senders {
    /// Install `sender` for `kind`, replacing any earlier one.
    pub fn install(&self, kind: StreamKind, sender: Arc<dyn StreamSender>) {
        self.by_kind.write().insert(kind, sender);
    }

    /// Remove `kind`'s sender (its owner stopped).
    pub fn remove(&self, kind: StreamKind) {
        self.by_kind.write().remove(&kind);
    }

    /// Send `row` through its kind's sender; `Deferred` when none is
    /// installed.
    pub(super) async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        let sender = self.by_kind.read().get(&row.kind).cloned();
        if let Some(sender) = sender {
            return sender.send(row).await;
        }
        tracing::debug!(
            kind = row.kind.as_str(),
            stream_seq = row.stream_seq,
            "no sender installed for the stream kind; the row stays queued"
        );
        AttemptOutcome::Deferred
    }
}
