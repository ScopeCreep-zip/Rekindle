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

/// Makes again the external write of a row the engine never attempts
/// (`rekindle_outbox::OutboxRetry::SettledExternally`): the user's Retry of
/// a failed channel post, through the channel send path that made it.
#[async_trait]
pub trait Reissuer: Send + Sync {
    /// Write `row` again under its client id; `Err(reason)` when it
    /// cannot be made.
    async fn reissue(&self, row: &OutboxRow) -> Result<(), String>;
}

/// The installed senders and reissuers, by stream kind.
#[derive(Default)]
pub struct Senders {
    by_kind: RwLock<HashMap<StreamKind, Arc<dyn StreamSender>>>,
    reissuers: RwLock<HashMap<StreamKind, Arc<dyn Reissuer>>>,
}

impl Senders {
    /// Install `sender` for `kind`, replacing any earlier one.
    pub fn install(&self, kind: StreamKind, sender: Arc<dyn StreamSender>) {
        self.by_kind.write().insert(kind, sender);
    }

    /// Remove `kind`'s sender and reissuer (their owner stopped).
    pub fn remove(&self, kind: StreamKind) {
        self.by_kind.write().remove(&kind);
        self.reissuers.write().remove(&kind);
    }

    /// Install `reissuer` for `kind`, replacing any earlier one.
    pub fn install_reissuer(&self, kind: StreamKind, reissuer: Arc<dyn Reissuer>) {
        self.reissuers.write().insert(kind, reissuer);
    }

    /// Write `row` again through its kind's reissuer.
    pub(super) async fn reissue(&self, row: &OutboxRow) -> Result<(), String> {
        let reissuer = self.reissuers.read().get(&row.kind).cloned();
        match reissuer {
            Some(reissuer) => reissuer.reissue(row).await,
            None => Err(format!("no {} writer is running", row.kind.as_str())),
        }
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
