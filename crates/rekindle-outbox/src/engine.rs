//! The engine: enqueue, attempt by `next_at`, settle.

use std::time::Duration;

use rekindle_crypto::horizon::DELIVERY_HORIZON;
use rekindle_types::message::SendState;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::deps::{
    AttemptOutcome, NewOutboxRow, OutboxDeps, OutboxError, OutboxRow, RowUpdate, StateChange,
};
use crate::state::{is_unsettled, merge, PeerAck, StateInput};
use crate::stream::StreamKind;

/// The wait before the next pass after the storage failed.
const STORAGE_RETRY: Duration = Duration::from_secs(5);

/// The reason a row past its deadline carries.
pub const DEADLINE_REASON: &str = "deadline";

/// When a row gives up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deadline {
    /// `DELIVERY_HORIZON` after the enqueue.
    Horizon,
    /// An earlier end the stream sets (a call session's end), ms.
    At(i64),
}

/// A send to enqueue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: StreamKind,
    /// The stream's scope within its kind (the peer, record or channel).
    pub scope: String,
    /// The id the frame carries on every attempt.
    pub client_id: String,
    pub payload: Vec<u8>,
    pub deadline: Deadline,
}

/// Where an enqueued row went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enqueued {
    pub stream: String,
    pub stream_seq: i64,
}

/// The outbox over its host `D`.
pub struct Outbox<D> {
    deps: D,
    wake: Notify,
}

fn horizon_ms() -> i64 {
    i64::try_from(DELIVERY_HORIZON.as_millis()).unwrap_or(i64::MAX)
}

impl<D: OutboxDeps> Outbox<D> {
    #[must_use]
    pub fn new(deps: D) -> Self {
        Self {
            deps,
            wake: Notify::new(),
        }
    }

    #[must_use]
    pub fn deps(&self) -> &D {
        &self.deps
    }

    fn now(&self) -> i64 {
        self.deps.clock().now_ms_i64()
    }

    fn backoff(&self, kind: StreamKind, attempts: u32) -> i64 {
        let wait = kind.policy().delay(attempts, self.deps.rng());
        self.now()
            .saturating_add(i64::try_from(wait.as_millis()).unwrap_or(i64::MAX))
    }

    /// Store `entry` with its local `echo`, `queued`, and wake the engine.
    ///
    /// # Errors
    /// The storage failed; nothing was queued.
    pub async fn enqueue(&self, entry: Entry, echo: D::Echo) -> Result<Enqueued, OutboxError> {
        let now = self.now();
        let deadline = match entry.deadline {
            Deadline::Horizon => now.saturating_add(horizon_ms()),
            Deadline::At(at) => at,
        };
        let stream = entry.kind.stream(&entry.scope);
        let row = NewOutboxRow {
            stream: stream.clone(),
            client_id: entry.client_id.clone(),
            kind: entry.kind,
            payload: entry.payload,
            next_at: now,
            deadline,
        };
        let stream_seq = self.deps.insert(row, echo).await?;
        self.deps.publish(StateChange {
            stream: stream.clone(),
            stream_seq,
            client_id: entry.client_id,
            kind: entry.kind,
            state: SendState::Queued,
            wedge_reason: None,
        });
        self.wake.notify_one();
        Ok(Enqueued { stream, stream_seq })
    }

    async fn fail(&self, row: &OutboxRow, reason: String) -> Result<(), OutboxError> {
        let state = merge(row.state, StateInput::Failed);
        let update = RowUpdate {
            state,
            attempts: row.attempts,
            next_at: row.next_at,
            deadline: row.deadline,
            wedge_reason: Some(reason.clone()),
        };
        self.deps.update(&row.client_id, &update).await?;
        self.deps.publish(StateChange::of(row, state, Some(reason)));
        Ok(())
    }

    async fn attempt(&self, row: &OutboxRow) -> Result<(), OutboxError> {
        let attempts = row.attempts.saturating_add(1);
        match self.deps.attempt(row).await {
            AttemptOutcome::Sent if !row.kind.awaits_ack() => {
                self.deps.settle(&row.client_id, SendState::Sent).await?;
                self.deps
                    .publish(StateChange::of(row, SendState::Sent, None));
            }
            outcome @ (AttemptOutcome::Sent | AttemptOutcome::Deferred) => {
                let state = if outcome == AttemptOutcome::Sent {
                    merge(row.state, StateInput::LocalSent)
                } else {
                    row.state
                };
                let update = RowUpdate {
                    state,
                    attempts,
                    next_at: self.backoff(row.kind, attempts),
                    deadline: row.deadline,
                    wedge_reason: None,
                };
                self.deps.update(&row.client_id, &update).await?;
                if state != row.state {
                    self.deps.publish(StateChange::of(row, state, None));
                }
            }
            AttemptOutcome::Wedged(reason) => self.fail(row, reason).await?,
        }
        Ok(())
    }

    /// One pass: fail the rows past their deadline, attempt the due ones.
    /// Returns when the next pass is due (ms), `None` when nothing waits.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn pump(&self) -> Result<Option<i64>, OutboxError> {
        let now = self.now();
        for row in self.deps.expired(now).await? {
            self.fail(&row, DEADLINE_REASON.to_owned()).await?;
        }
        for row in self.deps.due(now).await? {
            self.attempt(&row).await?;
        }
        self.deps.next_wake().await
    }

    /// The peer acknowledged `stream_seq` of `stream`. Returns whether a
    /// row was waiting for it.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn acknowledge(
        &self,
        stream: &str,
        stream_seq: i64,
        ack: PeerAck,
    ) -> Result<bool, OutboxError> {
        let Some(row) = self.deps.row_at(stream, stream_seq).await? else {
            return Ok(false);
        };
        let state = merge(row.state, StateInput::Peer(ack));
        self.deps.settle(&row.client_id, state).await?;
        self.deps.publish(StateChange::of(&row, state, None));
        self.wake.notify_one();
        Ok(true)
    }

    /// The user's Retry: a failed row is queued again under the **same**
    /// client id, with a fresh deadline; a queued one is attempted now.
    /// This is the one move down the lattice, and only the user makes it.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn retry(&self, client_id: &str) -> Result<bool, OutboxError> {
        let Some(row) = self.deps.row(client_id).await? else {
            return Ok(false);
        };
        if !matches!(row.state, SendState::Failed | SendState::Queued) {
            return Ok(false);
        }
        let now = self.now();
        let update = RowUpdate {
            state: SendState::Queued,
            attempts: 0,
            next_at: now,
            deadline: now.saturating_add(horizon_ms()),
            wedge_reason: None,
        };
        self.deps.update(client_id, &update).await?;
        if row.state != SendState::Queued {
            self.deps
                .publish(StateChange::of(&row, SendState::Queued, None));
        }
        self.wake.notify_one();
        Ok(true)
    }

    /// The user's Discard: the row settles `failed` and leaves its stream,
    /// so the rows behind it go on.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn discard(&self, client_id: &str) -> Result<bool, OutboxError> {
        let Some(row) = self.deps.row(client_id).await? else {
            return Ok(false);
        };
        if !is_unsettled(row.state) && row.state != SendState::Failed {
            return Ok(false);
        }
        self.deps.settle(client_id, SendState::Failed).await?;
        self.deps.publish(StateChange::of(
            &row,
            SendState::Failed,
            row.wedge_reason.clone(),
        ));
        self.wake.notify_one();
        Ok(true)
    }

    /// A frame `client_id` arrived on the peer's `stream`: `true` the
    /// first time, `false` for a resend already applied.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn receive(&self, stream: &str, client_id: &str) -> Result<bool, OutboxError> {
        self.deps.mark_received(stream, client_id).await
    }

    /// How many sends are not settled (D3's `PendingWork`).
    ///
    /// # Errors
    /// The storage failed.
    pub async fn unsettled(&self) -> Result<u64, OutboxError> {
        self.deps.unsettled_count().await
    }

    /// Pump whenever a row is due or the outbox is woken, until `stop`.
    pub async fn run(&self, stop: CancellationToken) {
        loop {
            let wait = match self.pump().await {
                Ok(Some(at)) => Some(Duration::from_millis(
                    u64::try_from(at.saturating_sub(self.now())).unwrap_or(0),
                )),
                Ok(None) => None,
                Err(e) => {
                    tracing::warn!(error = %e, "outbox pass failed");
                    Some(STORAGE_RETRY)
                }
            };
            let sleep = async {
                match wait {
                    Some(wait) => tokio::time::sleep(wait).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                () = stop.cancelled() => return,
                () = self.wake.notified() => {}
                () = sleep => {}
            }
        }
    }
}
