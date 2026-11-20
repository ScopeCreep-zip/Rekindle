//! Stream attempts off the pump (plan E2.5j). Each stream's run of due rows
//! is attempted as a future of its own: [`Outbox::run`]'s pump polls the
//! running ones beside its timer and its wake, so one stream's unanswered
//! call (a `friend_handshake` `app_call` to a peer whose replies are lost)
//! never holds another stream's send, and an enqueue's wake starts a pass
//! at once. A stream has at most one attempt running: a pass skips a
//! stream already in flight, so the rows of one stream are still attempted
//! in `stream_seq` order and [`crate::holds_back`] still stops only the
//! rows behind a stalled head of that stream. A finished attempt settles
//! its rows (`Outbox::settle_attempt`, the appender hand-off unchanged),
//! releases its stream and wakes the pump.

use std::collections::{BTreeMap, BTreeSet};

use futures::future::join_all;
use parking_lot::Mutex;

use rekindle_types::message::SendState;

use super::{Outbox, DEADLINE_REASON, KEY_DID_NOT_ARRIVE};
use crate::deps::{AttemptOutcome, OutboxDeps, OutboxError, OutboxRow};
use crate::stream::StreamKind;

/// The streams with an attempt running.
#[derive(Default)]
pub(crate) struct Attempting(Mutex<BTreeSet<String>>);

impl Attempting {
    pub(crate) fn streams(&self) -> BTreeSet<String> {
        self.0.lock().clone()
    }

    fn release(&self, stream: &str) {
        self.0.lock().remove(stream);
    }
}

/// One stream's due rows, claimed, in `stream_seq` order. Its stream is
/// released when the run ends or is cut off (the pump's scope ended).
pub(crate) struct Run<'a> {
    stream: String,
    rows: Vec<OutboxRow>,
    attempting: &'a Attempting,
}

impl Drop for Run<'_> {
    fn drop(&mut self) {
        self.attempting.release(&self.stream);
    }
}

impl<D: OutboxDeps> Outbox<D> {
    /// Claim the rows due at `at` (of `kind` only, when given), selected
    /// under the appenders' gate, as one run per stream not already in
    /// flight.
    pub(crate) async fn claim_due(
        &self,
        at: i64,
        kind: Option<StreamKind>,
    ) -> Result<Vec<Run<'_>>, OutboxError> {
        let _gate = self.appends.gate.lock().await;
        let mut due = self.deps.due(at).await?;
        due.retain(|row| kind.is_none_or(|k| row.kind == k));
        let mut streams: BTreeMap<String, Vec<OutboxRow>> = BTreeMap::new();
        for row in self.select_due(due, self.now()).await? {
            streams.entry(row.stream.clone()).or_default().push(row);
        }
        let mut claimed = self.attempting.0.lock();
        Ok(streams
            .into_iter()
            .filter(|(stream, _)| claimed.insert(stream.clone()))
            .map(|(stream, mut rows)| {
                rows.sort_by_key(|row| row.stream_seq);
                Run {
                    stream,
                    rows,
                    attempting: &self.attempting,
                }
            })
            .collect())
    }

    /// Attempt the rows due at `at` (of `kind` only, when given), each
    /// stream's run on its own, and wait for every run; how many rows.
    pub(crate) async fn attempt_due(
        &self,
        at: i64,
        kind: Option<StreamKind>,
    ) -> Result<usize, OutboxError> {
        let runs = self.claim_due(at, kind).await?;
        let attempted = runs.iter().map(|run| run.rows.len()).sum();
        for result in join_all(runs.into_iter().map(|run| self.attempt_run(run))).await {
            result?;
        }
        Ok(attempted)
    }

    /// Attempt `run`: its rows go to the host together, in `stream_seq`
    /// order, so a send that carries several (the `dm` chain's page) makes
    /// one write for them (E2.B F2); its chain appends go to its stream's
    /// appender, together. Then its stream is released and the pump woken.
    pub(crate) async fn attempt_run(&self, run: Run<'_>) -> Result<(), OutboxError> {
        // A prefix: the rows past the first that stayed behind were not
        // attempted and keep their schedule.
        let outcomes = self.deps.attempt_stream(&run.rows).await;
        // Read once the hints left: a cold route fetch or a run of hints
        // inside the attempt must not shrink the ack's window.
        let sent_at = self.now();
        let mut jobs = Vec::new();
        for (row, outcome) in run.rows.iter().zip(outcomes) {
            // Each attempt's outcome, by stream kind only (no identifier
            // leaves through a log, B19): what a scenario's `RUST_LOG`
            // shows of a row retried until it lands.
            tracing::debug!(
                stream = row.stream.split(':').next().unwrap_or_default(),
                attempts = row.attempts,
                outcome = outcome_label(&outcome),
                "outbox attempt"
            );
            if let AttemptOutcome::Appending(frame) = outcome {
                self.hand_off(row).await?;
                // Its append is still running past its deadline: the
                // attempt resent the hint, and no second append starts.
                if self.appends.in_flight(&row.client_id) {
                    continue;
                }
                jobs.push(crate::appender::Job {
                    row: row.clone(),
                    frame,
                    sent_at,
                });
                continue;
            }
            self.settle_attempt(row, outcome, sent_at).await?;
        }
        self.appends.hand(jobs, sent_at);
        drop(run);
        self.wake();
        Ok(())
    }

    /// Fail the rows past their deadline at `now` (a post that waited for
    /// its key with [`KEY_DID_NOT_ARRIVE`]), but a stream's whose
    /// attempt is running: its rows fail on the pass its end wakes, so an
    /// attempt never settles a row failed under it.
    pub(crate) async fn expire(&self, now: i64) -> Result<(), OutboxError> {
        let attempting = self.attempting.streams();
        for row in self.deps.expired(now).await? {
            if !attempting.contains(&row.stream) {
                let reason = if row.state == SendState::AwaitingKey {
                    KEY_DID_NOT_ARRIVE
                } else {
                    DEADLINE_REASON
                };
                self.fail(&row, reason.to_owned()).await?;
            }
        }
        Ok(())
    }

    /// When the next pass is due (ms), `None` when nothing waits. The
    /// store's `next_wake` cannot see the streams in flight: a row of one
    /// that is due now would wake a pass that skips it, over and over, so
    /// while it is past due the wake is read again without those streams.
    pub(crate) async fn pass_wake(&self) -> Result<Option<i64>, OutboxError> {
        let attempting = self.attempting.streams();
        let stored = match self.deps.next_wake().await? {
            Some(at) if !attempting.is_empty() && at <= self.now() => {
                self.wake_past(&attempting).await?
            }
            stored => stored,
        };
        let busy = self.busy_wake().await?;
        Ok(stored.into_iter().chain(busy).min())
    }

    /// The earliest time a row of a stream neither in `attempting` nor in
    /// an appender is due (by `due`'s rule: a row waits for every `queued`
    /// row ahead of it in its stream), or a row out of `attempting`
    /// reaches its deadline.
    async fn wake_past(&self, attempting: &BTreeSet<String>) -> Result<Option<i64>, OutboxError> {
        let appending = self.appends.streams();
        let skip =
            |row: &OutboxRow| attempting.contains(&row.stream) || appending.contains(&row.stream);
        let mut streams: BTreeMap<String, Vec<OutboxRow>> = BTreeMap::new();
        for row in self.deps.due(i64::MAX).await? {
            if !skip(&row) {
                streams.entry(row.stream.clone()).or_default().push(row);
            }
        }
        let mut wake: Option<i64> = None;
        for mut rows in streams.into_values() {
            rows.sort_by_key(|row| row.stream_seq);
            let mut ahead = i64::MIN;
            for row in rows {
                let at = row.next_at.max(ahead);
                wake = Some(wake.map_or(at, |w| w.min(at)));
                if row.state == rekindle_types::message::SendState::Queued {
                    ahead = ahead.max(row.next_at);
                }
            }
        }
        let ends = self.deps.expired(i64::MAX).await?;
        let ends = ends
            .iter()
            .filter(|row| !attempting.contains(&row.stream))
            .map(|row| row.deadline);
        Ok(wake.into_iter().chain(ends).min())
    }
}

/// An attempt outcome's name for a log line, without its payload.
fn outcome_label(outcome: &AttemptOutcome) -> &'static str {
    match outcome {
        AttemptOutcome::Appending(_) => "appending",
        AttemptOutcome::Sent => "sent",
        AttemptOutcome::Awaiting => "awaiting",
        AttemptOutcome::Receipted => "receipted",
        AttemptOutcome::Unanswered => "unanswered",
        AttemptOutcome::Deferred => "deferred",
        AttemptOutcome::Wedged(_) => "wedged",
        AttemptOutcome::Superseded(_) => "superseded",
    }
}
