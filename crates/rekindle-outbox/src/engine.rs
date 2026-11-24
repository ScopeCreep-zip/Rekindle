//! The engine: enqueue, attempt by `next_at`, settle.

use std::time::Duration;

use futures::stream::{FuturesUnordered, StreamExt};

use rekindle_crypto::horizon::DELIVERY_HORIZON;
use rekindle_types::message::SendState;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::deps::{
    AttemptOutcome, NewOutboxRow, OutboxDeps, OutboxError, OutboxRow, RowUpdate, StateChange,
};
use crate::state::{merge, Moved, PeerAck, StateInput};
use crate::stream::StreamKind;

mod attempts;
mod depends;
mod discard;
mod forward;
mod landed;

/// The wait before the next pass after the storage failed.
const STORAGE_RETRY: Duration = Duration::from_secs(5);

/// The reason a row past its deadline carries.
pub const DEADLINE_REASON: &str = "deadline";

/// The reason a channel post that waited for its channel's key past its
/// deadline carries (plan E3.2 S7l, F9-1).
pub const KEY_DID_NOT_ARRIVE: &str = "the community key did not arrive";

/// How long a row may wait for its peer before it settles failed at its
/// deadline: [`DELIVERY_HORIZON`] (D22, D24).
pub const OUTBOX_DEADLINE: std::time::Duration = DELIVERY_HORIZON;

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
    pub(crate) deps: D,
    wake: Notify,
    pub(crate) appends: crate::appender::Appends,
    pub(crate) attempting: attempts::Attempting,
    /// Whether a past-due wake the store's `due` withholds was reported
    /// (lead decision 51): logged once, never looped on.
    pub(crate) unhonoured_wake: std::sync::atomic::AtomicBool,
}

fn horizon_ms() -> i64 {
    i64::try_from(OUTBOX_DEADLINE.as_millis()).unwrap_or(i64::MAX)
}

impl<D: OutboxDeps> Outbox<D> {
    #[must_use]
    pub fn new(deps: D) -> Self {
        Self {
            deps,
            wake: Notify::new(),
            appends: crate::appender::Appends::default(),
            attempting: attempts::Attempting::default(),
            unhonoured_wake: std::sync::atomic::AtomicBool::new(false),
        }
    }

    #[must_use]
    pub fn deps(&self) -> &D {
        &self.deps
    }

    fn now(&self) -> i64 {
        self.deps.clock().now_ms_i64()
    }

    /// When `kind`'s row is next due after `attempts` attempts (0: its
    /// first) at `now`, never past its `deadline`, where the pass fails it.
    fn next_due(&self, kind: StreamKind, attempts: u32, now: i64, deadline: i64) -> i64 {
        if attempts == 0 {
            return now;
        }
        let wait = kind.policy().delay(attempts, self.deps.rng());
        now.saturating_add(i64::try_from(wait.as_millis()).unwrap_or(i64::MAX))
            .min(deadline)
    }

    /// When `row`, now `state`, is due again: one that just reached `sent`
    /// waits its kind's first re-ask for the peer's ack, measured from
    /// `sent_at`, when its send (the hint) left, not from a later landing
    /// (live A/B finding 73c); any other its backoff.
    pub(crate) fn next_after(
        &self,
        row: &OutboxRow,
        state: SendState,
        attempts: u32,
        sent_at: i64,
    ) -> i64 {
        let now = self.now();
        match row.kind.first_reask(self.deps.rng()) {
            Some(wait) if row.state != SendState::Sent && state == SendState::Sent => {
                sent_at.saturating_add(i64::try_from(wait.as_millis()).unwrap_or(i64::MAX))
            }
            _ => self.next_due(row.kind, attempts, now, row.deadline),
        }
    }

    /// Store `entry` with its local `echo`, `queued`, and wake the engine.
    ///
    /// # Errors
    /// The storage failed; nothing was queued.
    pub async fn enqueue(&self, entry: Entry, echo: D::Echo) -> Result<Enqueued, OutboxError> {
        self.enqueue_row(self.new_row(entry), echo).await
    }

    /// Store `row` with its local `echo`, `queued`, and wake the engine.
    pub(crate) async fn enqueue_row(
        &self,
        row: NewOutboxRow,
        echo: D::Echo,
    ) -> Result<Enqueued, OutboxError> {
        let (stream, client_id, kind) = (row.stream.clone(), row.client_id.clone(), row.kind);
        let stream_seq = self.deps.insert(row, echo).await?;
        self.deps.publish(StateChange {
            stream: stream.clone(),
            stream_seq,
            client_id,
            kind,
            state: SendState::Queued,
            wedge_reason: None,
            retry: None,
        });
        self.wake.notify_one();
        Ok(Enqueued { stream, stream_seq })
    }

    /// The row `entry` is stored as, `queued` and due now: for a host that
    /// stores it in a transaction of its own with other state (a friend's
    /// removal and its `Unfriended` notice), then calls [`Self::wake`].
    #[must_use]
    pub fn new_row(&self, entry: Entry) -> NewOutboxRow {
        let now = self.now();
        let deadline = match entry.deadline {
            Deadline::Horizon => now.saturating_add(horizon_ms()),
            Deadline::At(at) => at,
        };
        NewOutboxRow {
            stream: entry.kind.stream(&entry.scope),
            client_id: entry.client_id,
            kind: entry.kind,
            payload: entry.payload,
            next_at: self.next_due(entry.kind, 0, now, deadline),
            deadline,
            waits_for: Vec::new(),
            takes_over: Vec::new(),
        }
    }

    /// Attempt the due rows now (a row was stored through [`Self::new_row`]).
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Fail `row` with `reason`, and the rows waiting for it with it.
    /// A row the peer's ack settled meanwhile is left as it is, and so are
    /// the rows waiting for it.
    pub(crate) async fn fail(&self, row: &OutboxRow, reason: String) -> Result<(), OutboxError> {
        if self.fail_one(row, reason).await? {
            self.fail_dependents(row).await?;
        }
        Ok(())
    }

    pub(crate) async fn fail_one(
        &self,
        row: &OutboxRow,
        reason: String,
    ) -> Result<bool, OutboxError> {
        let update = RowUpdate {
            state: SendState::Failed,
            attempts: row.attempts,
            next_at: row.next_at,
            deadline: row.deadline,
            wedge_reason: Some(reason.clone()),
            retry: false,
        };
        let moved = self.deps.update(&row.client_id, &update).await?;
        let moved = self.publish_moved(row, moved, Some(reason));
        Ok(moved.is_some_and(|m| m.after == SendState::Failed))
    }

    /// Publish `row`'s shown state when a write `moved` it, never one the
    /// row already shows or one below it (D22); wake the pump when the row
    /// settled at the peer's ack, so its stream head moves.
    pub(crate) fn publish_moved(
        &self,
        row: &OutboxRow,
        moved: Option<Moved>,
        reason: Option<String>,
    ) -> Option<Moved> {
        if let Some(m) = moved.filter(|m| m.changed()) {
            self.deps.publish(StateChange::of(row, m.after, reason));
            if matches!(m.after, SendState::Delivered | SendState::Read) {
                self.wake.notify_one();
            }
        }
        moved
    }

    /// `row`'s append is handed off: it stays `queued`, counted, and is due
    /// again (if the append is cut off) after its backoff. Written only
    /// while it is still `queued`: `row` was read before the hint was
    /// awaited, and the appender may have landed it since (review LV22 M2).
    pub(crate) async fn hand_off(&self, row: &OutboxRow) -> Result<(), OutboxError> {
        let attempts = row.attempts.saturating_add(1);
        let next_at = self.next_due(row.kind, attempts, self.now(), row.deadline);
        (self.deps)
            .reschedule_queued(&row.client_id, attempts, next_at)
            .await
    }

    /// Settle `row`'s attempt, whose send left at `sent_at`.
    pub(crate) async fn settle_attempt(
        &self,
        row: &OutboxRow,
        outcome: AttemptOutcome,
        sent_at: i64,
    ) -> Result<(), OutboxError> {
        let attempts = row.attempts.saturating_add(1);
        match outcome {
            AttemptOutcome::Sent if !row.kind.awaits_ack() => {
                let moved = self.deps.settle(&row.client_id, SendState::Sent).await?;
                self.publish_moved(row, moved, None);
            }
            // The receipt is the landing and the ack at once.
            AttemptOutcome::Receipted => {
                let state = merge(row.state, StateInput::Peer(PeerAck::Delivered));
                self.settle_at(row, state).await?;
            }
            outcome @ (AttemptOutcome::Sent
            | AttemptOutcome::Awaiting
            | AttemptOutcome::Deferred
            | AttemptOutcome::Appending(_)) => {
                let landed = matches!(outcome, AttemptOutcome::Sent | AttemptOutcome::Awaiting);
                if landed && row.state == SendState::Queued {
                    return self.mark_landed(row, attempts, sent_at).await;
                }
                let state = if landed {
                    merge(row.state, StateInput::LocalSent)
                } else {
                    row.state
                };
                let update = RowUpdate {
                    state,
                    attempts,
                    next_at: self.next_after(row, state, attempts, sent_at),
                    deadline: row.deadline,
                    wedge_reason: None,
                    retry: false,
                };
                let moved = self.deps.update(&row.client_id, &update).await?;
                self.publish_moved(row, moved, None);
            }
            AttemptOutcome::Unanswered => {
                let state = merge(row.state, StateInput::LocalSent);
                let wait = row.kind.unanswered_retry().delay(attempts, self.deps.rng());
                let wait = i64::try_from(wait.as_millis()).unwrap_or(i64::MAX);
                let update = RowUpdate {
                    state,
                    attempts,
                    next_at: self.now().saturating_add(wait).min(row.deadline),
                    deadline: row.deadline,
                    wedge_reason: None,
                    retry: false,
                };
                let moved = self.deps.update(&row.client_id, &update).await?;
                // Every unanswered attempt of a row still `sent` is
                // published, its state changed or not: the send reads not
                // delivered yet, still trying, with its count and next
                // attempt (plan §3.0 lane 1). A row the peer's ack raised
                // (or a deadline failed) meanwhile shows that instead.
                match moved {
                    Some(m) if m.after == SendState::Sent => self.deps.publish(
                        StateChange::retrying(row, m.after, attempts, update.next_at),
                    ),
                    moved => {
                        self.publish_moved(row, moved, None);
                    }
                }
            }
            AttemptOutcome::Wedged(reason) => self.fail(row, reason).await?,
            AttemptOutcome::Superseded(reason) => {
                let moved = self.deps.settle(&row.client_id, SendState::Failed).await?;
                let moved = self.publish_moved(row, moved, Some(reason));
                if moved.is_some_and(|m| m.after == SendState::Failed) {
                    self.fail_dependents(row).await?;
                }
                self.wake.notify_one();
            }
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
        self.expire(now).await?;
        self.attempt_due(now, None).await?;
        self.pass_wake().await
    }

    /// The pump's pass (plan E2.5j): fail the rows past their deadline and
    /// claim the due ones, one run per stream not in flight, to attempt off
    /// the pass; and when the next pass is due.
    async fn begin_pass(&self) -> Result<(Vec<attempts::Run<'_>>, Option<i64>), OutboxError> {
        let now = self.now();
        self.expire(now).await?;
        let runs = self.claim_due(now, None).await?;
        Ok((runs, self.pass_wake().await?))
    }

    /// Attempt every unsettled row of `kind` that no stream head blocks
    /// now, whatever its `next_at`: a host hands a stream to its new
    /// sender at once (the `dht_write` rows at unlock, before the session
    /// writes); how many were attempted.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn flush(&self, kind: StreamKind) -> Result<usize, OutboxError> {
        self.attempt_due(i64::MAX, Some(kind)).await
    }

    /// The write that carries every unsettled row of stream `kind:scope`
    /// settled (a `channel_post` stream, whose send is the record pool's
    /// write of one record slot, plan E2.5): `Ok` when it landed, each row
    /// `sent`; `Err(reason)` when it did not, each row `failed` with
    /// `reason`. Every row leaves its stream. Returns the rows settled.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn settle_stream(
        &self,
        kind: StreamKind,
        scope: &str,
        outcome: Result<(), String>,
    ) -> Result<Vec<OutboxRow>, OutboxError> {
        let rows = self.deps.stream_rows(&kind.stream(scope)).await?;
        let (state, reason) = match outcome {
            Ok(()) => (StateInput::LocalSent, None),
            Err(reason) => (StateInput::Failed, Some(reason)),
        };
        for row in &rows {
            let settled = merge(row.state, state);
            let moved = self.deps.settle(&row.client_id, settled).await?;
            self.publish_moved(row, moved, reason.clone());
        }
        if !rows.is_empty() {
            self.wake.notify_one();
        }
        Ok(rows)
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
        self.settle_acked(&row, ack).await?;
        Ok(true)
    }

    /// The peer acknowledged row `client_id` by a frame that names no
    /// stream seq (`UnfriendedAck`). Returns whether a row was waiting.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn acknowledge_client(
        &self,
        client_id: &str,
        ack: PeerAck,
    ) -> Result<bool, OutboxError> {
        let Some(row) = self.deps.row(client_id).await? else {
            return Ok(false);
        };
        self.settle_acked(&row, ack).await?;
        Ok(true)
    }

    /// The peer's `ack` of `row`, shown at once (D22: `delivered` comes from
    /// the peer's `DmAck`). A row whose send has not landed (`queued`: the
    /// peer read the hint before our chain append returned) keeps the ack,
    /// durably, and stays until the landing comes, then goes with the state
    /// already shown (live A/B finding 42).
    async fn settle_acked(&self, row: &OutboxRow, ack: PeerAck) -> Result<(), OutboxError> {
        if row.state == SendState::Queued {
            if let Some(moved) = self.deps.note_ack(&row.client_id, ack).await? {
                self.publish_moved(row, Some(moved), None);
                return Ok(());
            }
        }
        self.settle_at(row, merge(row.state, StateInput::Peer(ack)))
            .await
    }

    pub(crate) async fn settle_at(
        &self,
        row: &OutboxRow,
        state: SendState,
    ) -> Result<(), OutboxError> {
        let moved = self.deps.settle(&row.client_id, state).await?;
        self.publish_moved(row, moved, None);
        self.wake.notify_one();
        Ok(())
    }

    /// The user's Retry: a failed row is queued again under the **same**
    /// client id, with a fresh deadline; a queued one is attempted now.
    /// This is the one move down the lattice, and only the user makes it.
    ///
    /// # Errors
    /// The storage failed, or the row's kind is not the user's to retry
    /// ([`OutboxError::NotRetryable`]; nothing changed).
    pub async fn retry(&self, client_id: &str) -> Result<bool, OutboxError> {
        let Some(row) = self.deps.row(client_id).await? else {
            return Ok(false);
        };
        if !row.kind.user_retryable() {
            return Err(OutboxError::NotRetryable(row.kind));
        }
        if !matches!(row.state, SendState::Failed | SendState::Queued) {
            return Ok(false);
        }
        self.requeue(&row).await?;
        self.retry_dependents(&row).await?;
        self.wake.notify_one();
        Ok(true)
    }

    /// Queue `row` (failed or queued) again under its client id with a
    /// fresh deadline, due now.
    pub(crate) async fn requeue(&self, row: &OutboxRow) -> Result<(), OutboxError> {
        let client_id = row.client_id.as_str();
        let now = self.now();
        let deadline = now.saturating_add(horizon_ms());
        let update = RowUpdate {
            state: SendState::Queued,
            attempts: 0,
            next_at: self.next_due(row.kind, 0, now, deadline),
            deadline,
            wedge_reason: None,
            retry: true,
        };
        let moved = self.deps.update(client_id, &update).await?;
        self.publish_moved(row, moved, None);
        Ok(())
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

    /// Pump whenever a row is due or the outbox is woken, and run the
    /// streams' appenders, until `stop`; an append still in flight then is
    /// cut off, and its row is attempted again by a later pass.
    pub async fn run(&self, stop: CancellationToken) {
        // The startup pass (research `prod_research_batch9.md` Q3): rows
        // whose entry landed before a restart settle, the rest re-queue.
        if let Err(e) = self.reconcile().await {
            tracing::warn!(error = %e, "outbox startup reconcile failed");
        }
        // `biased`: the pump, then the appenders, on every poll (as the
        // pump's own select).
        tokio::select! {
            biased;
            () = self.pump_loop(&stop) => {}
            () = self.append_loop() => {}
        }
        self.appends.clear();
    }

    /// Pass whenever a row is due, the outbox is woken or a stream's
    /// attempt ends; each pass's runs are attempted beside the pump, never
    /// awaited by it (plan E2.5j).
    async fn pump_loop(&self, stop: &CancellationToken) {
        let mut running = FuturesUnordered::new();
        loop {
            let wait = match self.begin_pass().await {
                Ok((runs, at)) => {
                    running.extend(runs.into_iter().map(|run| self.attempt_run(run)));
                    at.map(|at| {
                        Duration::from_millis(
                            u64::try_from(at.saturating_sub(self.now())).unwrap_or(0),
                        )
                    })
                }
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
            // `biased`: the branches are polled in this order, never in
            // tokio's per-process random one, so which of a stop, an
            // attempt's end, a wake and the timer ready at one instant is
            // taken (and whether the attempts are polled at all this turn)
            // is the same on every run (ADR 0021: one seed, one trace).
            tokio::select! {
                biased;
                () = stop.cancelled() => return,
                Some(ended) = running.next(), if !running.is_empty() => {
                    if let Err(e) = ended {
                        tracing::warn!(error = %e, "outbox attempt not stored");
                    }
                }
                () = self.wake.notified() => {}
                () = sleep => {}
            }
        }
    }
}
