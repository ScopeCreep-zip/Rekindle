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

    /// Send `rows`, the due rows of one stream in `stream_seq` order, once
    /// each, stopping after the first that stays behind
    /// (`rekindle_outbox::holds_back`): the outcomes of a prefix. A kind whose send can carry
    /// several rows at once (the `dm` chain, plan E2.B F2) overrides it.
    async fn send_stream(&self, rows: &[OutboxRow]) -> Vec<AttemptOutcome> {
        let mut outcomes = Vec::with_capacity(rows.len());
        for row in rows {
            let outcome = self.send(row).await;
            let stop = rekindle_outbox::holds_back(&outcome);
            outcomes.push(outcome);
            if stop {
                break;
            }
        }
        outcomes
    }

    /// Append `frames`, the entries an attempt handed off
    /// ([`AttemptOutcome::Appending`]), to `stream`'s chain: the outcomes
    /// of a prefix. Only a chain sender hands appends off.
    async fn append(&self, stream: &str, frames: &[(Vec<u8>, i64)]) -> Vec<AttemptOutcome> {
        frames
            .iter()
            .take(1)
            .map(|_| AttemptOutcome::Wedged(format!("{stream} has no chain appender")))
            .collect()
    }
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

    /// Send `rows` (one stream's, in `stream_seq` order) through their
    /// kind's sender; each `Deferred` when none is installed.
    pub(super) async fn send_stream(&self, rows: &[OutboxRow]) -> Vec<AttemptOutcome> {
        let sender = rows
            .first()
            .and_then(|row| self.by_kind.read().get(&row.kind).cloned());
        if let Some(sender) = sender {
            return sender.send_stream(rows).await;
        }
        let mut outcomes = Vec::with_capacity(rows.len());
        for row in rows {
            let outcome = self.send(row).await;
            let stop = rekindle_outbox::holds_back(&outcome);
            outcomes.push(outcome);
            if stop {
                break;
            }
        }
        outcomes
    }

    /// Append `frames` to `stream` through `kind`'s sender; the first
    /// `Deferred` when none is installed (its owner stopped).
    pub(super) async fn append(
        &self,
        kind: StreamKind,
        stream: &str,
        frames: &[(Vec<u8>, i64)],
    ) -> Vec<AttemptOutcome> {
        let sender = self.by_kind.read().get(&kind).cloned();
        match sender {
            Some(sender) => sender.append(stream, frames).await,
            None => frames
                .iter()
                .take(1)
                .map(|_| AttemptOutcome::Deferred)
                .collect(),
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
