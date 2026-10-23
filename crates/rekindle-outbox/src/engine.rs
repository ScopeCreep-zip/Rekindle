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
use crate::stream::{OutboxRetry, StreamKind};

/// The wait before the next pass after the storage failed.
const STORAGE_RETRY: Duration = Duration::from_secs(5);

/// The reason a row past its deadline carries.
pub const DEADLINE_REASON: &str = "deadline";

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
    /// first) at `now`: a kind the engine never attempts
    /// ([`crate::OutboxRetry::SettledExternally`]) is due only at its
    /// `deadline`, where the pass fails it first.
    fn next_due(&self, kind: StreamKind, attempts: u32, now: i64, deadline: i64) -> i64 {
        if attempts == 0 && kind.policy() != OutboxRetry::SettledExternally {
            return now;
        }
        match kind.policy().delay(attempts, self.deps.rng()) {
            Some(wait) => now.saturating_add(i64::try_from(wait.as_millis()).unwrap_or(i64::MAX)),
            None => deadline,
        }
    }

    /// Store `entry` with its local `echo`, `queued`, and wake the engine.
    ///
    /// # Errors
    /// The storage failed; nothing was queued.
    pub async fn enqueue(&self, entry: Entry, echo: D::Echo) -> Result<Enqueued, OutboxError> {
        let row = self.new_row(entry);
        let (stream, client_id, kind) = (row.stream.clone(), row.client_id.clone(), row.kind);
        let stream_seq = self.deps.insert(row, echo).await?;
        self.deps.publish(StateChange {
            stream: stream.clone(),
            stream_seq,
            client_id,
            kind,
            state: SendState::Queued,
            wedge_reason: None,
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
        }
    }

    /// Attempt the due rows now (a row was stored through [`Self::new_row`]).
    pub fn wake(&self) {
        self.wake.notify_one();
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

    /// Attempt the rows due at `at` (of `kind` only, when given), selected
    /// under the appenders' gate; how many.
    async fn attempt_due(&self, at: i64, kind: Option<StreamKind>) -> Result<usize, OutboxError> {
        let rows = {
            let _gate = self.appends.gate.lock().await;
            let mut due = self.deps.due(at).await?;
            due.retain(|row| kind.is_none_or(|k| row.kind == k));
            self.select_due(due, self.now()).await?
        };
        let attempted = rows.len();
        self.attempt_all(rows).await?;
        Ok(attempted)
    }

    /// `row`'s append is handed off: it stays `queued`, counted, and is due
    /// again (if the append is cut off) after its backoff.
    pub(crate) async fn hand_off(&self, row: &OutboxRow) -> Result<(), OutboxError> {
        let attempts = row.attempts.saturating_add(1);
        let update = RowUpdate {
            state: row.state,
            attempts,
            next_at: self.next_due(row.kind, attempts, self.now(), row.deadline),
            deadline: row.deadline,
            wedge_reason: None,
        };
        self.deps.update(&row.client_id, &update).await.map(drop)
    }

    /// Attempt `rows`, due rows in `due` order: each stream's run of due
    /// rows goes to the host together, in `stream_seq` order, so a send that carries
    /// several (the `dm` chain's page) makes one write for them (E2.B F2).
    /// A run's chain appends go to its stream's appender, together.
    async fn attempt_all(&self, rows: Vec<OutboxRow>) -> Result<(), OutboxError> {
        let mut streams: Vec<Vec<OutboxRow>> = Vec::new();
        for row in rows {
            if row.kind.policy() == OutboxRetry::SettledExternally {
                continue;
            }
            match streams.iter_mut().find(|s| s[0].stream == row.stream) {
                Some(stream) => stream.push(row),
                None => streams.push(vec![row]),
            }
        }
        for mut stream in streams {
            stream.sort_by_key(|row| row.stream_seq);
            // A prefix: the rows past the first that stayed behind were not
            // attempted and keep their schedule.
            let outcomes = self.deps.attempt_stream(&stream).await;
            let mut jobs = Vec::new();
            for (row, outcome) in stream.iter().zip(outcomes) {
                if let AttemptOutcome::Appending(frame) = outcome {
                    self.hand_off(row).await?;
                    jobs.push(crate::appender::Job {
                        row: row.clone(),
                        frame,
                    });
                    continue;
                }
                self.settle_attempt(row, outcome).await?;
            }
            self.appends.hand(jobs);
        }
        Ok(())
    }

    pub(crate) async fn settle_attempt(
        &self,
        row: &OutboxRow,
        outcome: AttemptOutcome,
    ) -> Result<(), OutboxError> {
        let attempts = row.attempts.saturating_add(1);
        match outcome {
            AttemptOutcome::Sent if !row.kind.awaits_ack() => {
                self.deps.settle(&row.client_id, SendState::Sent).await?;
                self.deps
                    .publish(StateChange::of(row, SendState::Sent, None));
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
                let state = if landed {
                    merge(row.state, StateInput::LocalSent)
                } else {
                    row.state
                };
                let update = RowUpdate {
                    state,
                    attempts,
                    next_at: self.next_due(row.kind, attempts, self.now(), row.deadline),
                    deadline: row.deadline,
                    wedge_reason: None,
                };
                let acked = self.deps.update(&row.client_id, &update).await?;
                // The peer's ack came before this landing: both are in now
                // (live A/B finding 42).
                // Its state was shown when the ack came (D22); the landing
                // lets the row go.
                if let (true, Some(ack)) = (landed, acked) {
                    self.deps
                        .settle(&row.client_id, merge(state, StateInput::Peer(ack)))
                        .await?;
                    self.wake.notify_one();
                    return Ok(());
                }
                if state != row.state {
                    self.deps.publish(StateChange::of(row, state, None));
                }
            }
            AttemptOutcome::Wedged(reason) => self.fail(row, reason).await?,
            AttemptOutcome::Superseded(reason) => {
                self.deps.settle(&row.client_id, SendState::Failed).await?;
                self.deps
                    .publish(StateChange::of(row, SendState::Failed, Some(reason)));
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
        for row in self.deps.expired(now).await? {
            self.fail(&row, DEADLINE_REASON.to_owned()).await?;
        }
        self.attempt_due(now, None).await?;
        self.deps.next_wake().await
    }

    /// Attempt every unsettled row of `kind` that no stream head blocks
    /// now, whatever its `next_at`: a host hands a stream to its new
    /// sender at once (the `dht_write` rows at unlock, before the session
    /// writes); how many were attempted.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn flush(&self, kind: StreamKind) -> Result<usize, OutboxError> {
        if kind.policy() == OutboxRetry::SettledExternally {
            return Ok(0);
        }
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
            self.deps.settle(&row.client_id, settled).await?;
            self.deps
                .publish(StateChange::of(row, settled, reason.clone()));
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
        if row.state == SendState::Queued && self.deps.note_ack(&row.client_id, ack).await? {
            let shown = merge(row.state, StateInput::Peer(ack));
            self.deps.publish(StateChange::of(row, shown, None));
            return Ok(());
        }
        self.settle_at(row, merge(row.state, StateInput::Peer(ack)))
            .await
    }

    pub(crate) async fn settle_at(
        &self,
        row: &OutboxRow,
        state: SendState,
    ) -> Result<(), OutboxError> {
        self.deps.settle(&row.client_id, state).await?;
        self.deps.publish(StateChange::of(row, state, None));
        self.wake.notify_one();
        Ok(())
    }

    /// The peer read our `kind` stream to `scope` through `up_to`
    /// (`ReadUpTo`, plan E2.5): every entry at or below it, settled or
    /// not, becomes `read`, the top of the lattice (`merge` of any state
    /// with the peer's `Read`). Returns how many entries changed.
    ///
    /// # Errors
    /// The storage failed; nothing changed.
    pub async fn read_up_to(
        &self,
        kind: StreamKind,
        scope: &str,
        up_to: i64,
    ) -> Result<usize, OutboxError> {
        let stream = kind.stream(scope);
        let raised = self.deps.read_through(&stream, scope, up_to).await?;
        for (stream_seq, client_id) in &raised {
            self.deps.publish(StateChange {
                stream: stream.clone(),
                stream_seq: *stream_seq,
                client_id: client_id.clone(),
                kind,
                state: SendState::Read,
                wedge_reason: None,
            });
        }
        if !raised.is_empty() {
            self.wake.notify_one();
        }
        Ok(raised.len())
    }

    /// The user's Retry: a failed row is queued again under the **same**
    /// client id, with a fresh deadline; a queued one is attempted now. A
    /// row the engine never attempts has its write made again
    /// ([`OutboxDeps::reissue`]); it fails at once when that cannot be.
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
        let now = self.now();
        let deadline = now.saturating_add(horizon_ms());
        let update = RowUpdate {
            state: SendState::Queued,
            attempts: 0,
            next_at: self.next_due(row.kind, 0, now, deadline),
            deadline,
            wedge_reason: None,
        };
        self.deps.update(client_id, &update).await?;
        if row.state != SendState::Queued {
            self.deps
                .publish(StateChange::of(&row, SendState::Queued, None));
        }
        if row.kind.policy() == OutboxRetry::SettledExternally {
            if let Err(reason) = self.deps.reissue(&row).await {
                let queued = OutboxRow {
                    state: SendState::Queued,
                    attempts: 0,
                    next_at: update.next_at,
                    deadline,
                    wedge_reason: None,
                    ..row
                };
                self.fail(&queued, reason).await?;
            }
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

    /// Discard every unsettled row of the `kind` streams whose scope starts
    /// with `scope_prefix` (a call session ended: its `media_key` rows go,
    /// whoever they were for); each settles `failed` with `reason`.
    /// Returns how many were discarded.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn discard_streams(
        &self,
        kind: StreamKind,
        scope_prefix: &str,
        reason: &str,
    ) -> Result<usize, OutboxError> {
        let rows = self.deps.prefix_rows(&kind.stream(scope_prefix)).await?;
        for row in &rows {
            self.deps.settle(&row.client_id, SendState::Failed).await?;
            self.deps.publish(StateChange::of(
                row,
                SendState::Failed,
                Some(reason.to_owned()),
            ));
        }
        Ok(rows.len())
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
        tokio::select! {
            () = self.pump_loop(&stop) => {}
            () = self.append_loop() => {}
        }
        self.appends.clear();
    }

    async fn pump_loop(&self, stop: &CancellationToken) {
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
