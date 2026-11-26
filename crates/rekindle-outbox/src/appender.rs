//! The DM pump (live A/B, decided 2026-10-23): each `dm` and `group_dm`
//! stream has one ordered appender on the outbox's scope. A pass sends a
//! run's hints at once and hands the run's chain appends here
//! ([`AttemptOutcome::Appending`]); one append is in flight per stream, in
//! chain order, and the pass returns without awaiting it. A row whose
//! append is in flight stays `queued`; within [`DM_APPEND_DEADLINE`] it is
//! not attempted again, and past it a due row is re-hinted (its hint only:
//! the running append is never cancelled and no second one starts, research
//! `prod_research_batch9.md` Q1, Q3). The append's outcome, however late, is
//! one landing input (the two-input settle, `Outbox::mark_landed`), a no-op on
//! a row another path landed. The appender stops with [`Outbox::run`]'s
//! scope, and a row whose append was cut off is attempted again by a later
//! pass, at the `next_at` its hand-off set, or by `Outbox::reconcile`.
//!
//! A row whose attempt is an `app_call` (a `StreamRecord`,
//! [`AttemptOutcome::Calling`]) is handed here the same way: its call runs
//! beside the appends, the pass returns without awaiting it, and the
//! stream's later rows are attempted meanwhile (a call never holds its
//! stream, as an append never does). One call per row is in flight; its
//! answer, however late, settles the row while it is still unsettled.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use futures::future::FutureExt;
use futures::stream::{FuturesUnordered, StreamExt};
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::deps::{AttemptOutcome, OutboxDeps, OutboxError, OutboxRow};
use crate::engine::Outbox;
use crate::state::is_unsettled;
use crate::stream::DM_APPEND_DEADLINE;
use rekindle_types::message::SendState;

/// One row's append, handed off by a pass.
pub(crate) struct Job {
    /// The row as the pass read it (its `attempts` before the hand-off).
    pub(crate) row: OutboxRow,
    pub(crate) frame: Vec<u8>,
    /// When the pass sent the row's hint: its first re-ask runs from it,
    /// not from the landing (live A/B finding 73c).
    pub(crate) sent_at: i64,
}

#[derive(Default)]
struct State {
    /// The client ids handed off and not landed, each with when its
    /// append's outcome turns unknown ([`DM_APPEND_DEADLINE`]).
    in_flight: HashMap<String, i64>,
    /// Per stream, the jobs waiting for the stream's append, in order.
    waiting: BTreeMap<String, VecDeque<Job>>,
    /// The streams with an append running.
    busy: BTreeSet<String>,
    /// The client ids whose call is in flight, each with its stream and
    /// when its outcome turns unknown ([`DM_APPEND_DEADLINE`], which spans
    /// the call timeout).
    calls: HashMap<String, (String, i64)>,
    /// The calls handed off and not started yet.
    calls_ready: Vec<Job>,
}

/// The appenders' shared state.
#[derive(Default)]
pub(crate) struct Appends {
    state: Mutex<State>,
    wake: Notify,
    /// Held by a pass while it reads and selects its rows, and by a landing
    /// while it settles: a pass never reads a row half landed.
    pub(crate) gate: tokio::sync::Mutex<()>,
}

impl Appends {
    pub(crate) fn in_flight(&self, client_id: &str) -> bool {
        let s = self.state.lock();
        s.in_flight.contains_key(client_id) || s.calls.contains_key(client_id)
    }

    /// Whether `client_id`'s append is in flight and its outcome still
    /// awaited at `now`.
    pub(crate) fn within_deadline(&self, client_id: &str, now: i64) -> bool {
        let s = self.state.lock();
        s.in_flight
            .get(client_id)
            .or_else(|| s.calls.get(client_id).map(|(_, deadline)| deadline))
            .is_some_and(|deadline| now < *deadline)
    }

    /// The streams a pass walks past a row in flight: [`Self::streams`]
    /// and the streams with a call in flight.
    pub(crate) fn walked(&self) -> BTreeSet<String> {
        let mut walked = self.streams();
        walked.extend(self.state.lock().calls.values().map(|(s, _)| s.clone()));
        walked
    }

    /// Start `jobs`' calls (rows handed off by [`AttemptOutcome::Calling`],
    /// each its frame), handed off at `now`.
    pub(crate) fn call(&self, jobs: Vec<Job>, now: i64) {
        if jobs.is_empty() {
            return;
        }
        let deadline = deadline_from(now);
        let mut s = self.state.lock();
        for job in jobs {
            let entry = (job.row.stream.clone(), deadline);
            s.calls.insert(job.row.client_id.clone(), entry);
            s.calls_ready.push(job);
        }
        drop(s);
        self.wake.notify_one();
    }

    fn take_calls(&self) -> Vec<Job> {
        std::mem::take(&mut self.state.lock().calls_ready)
    }

    fn finish_call(&self, client_id: &str) {
        self.state.lock().calls.remove(client_id);
    }

    /// The streams with a row in flight, in order.
    pub(crate) fn streams(&self) -> BTreeSet<String> {
        let s = self.state.lock();
        s.busy.iter().chain(s.waiting.keys()).cloned().collect()
    }

    /// Queue `jobs` (one stream's run, in order) behind its stream's
    /// append, handed off at `now`.
    pub(crate) fn hand(&self, jobs: Vec<Job>, now: i64) {
        if jobs.is_empty() {
            return;
        }
        let deadline = deadline_from(now);
        let mut s = self.state.lock();
        for job in jobs {
            s.in_flight.insert(job.row.client_id.clone(), deadline);
            s.waiting
                .entry(job.row.stream.clone())
                .or_default()
                .push_back(job);
        }
        drop(s);
        self.wake.notify_one();
    }

    /// Every waiting run whose stream has no append running, now running.
    fn take_ready(&self) -> Vec<(String, Vec<Job>)> {
        let mut s = self.state.lock();
        let ready: Vec<String> = s
            .waiting
            .keys()
            .filter(|stream| !s.busy.contains(*stream))
            .cloned()
            .collect();
        ready
            .into_iter()
            .filter_map(|stream| {
                let jobs: Vec<Job> = s.waiting.remove(&stream)?.into();
                s.busy.insert(stream.clone());
                Some((stream, jobs))
            })
            .collect()
    }

    /// `stream`'s append of `jobs` ended. When it stopped short, the runs
    /// waiting behind it are dropped too, so a later pass attempts them in
    /// chain order after the row that stopped.
    fn finish(&self, stream: &str, jobs: &[Job], stopped: bool) {
        let mut s = self.state.lock();
        s.busy.remove(stream);
        for job in jobs {
            s.in_flight.remove(&job.row.client_id);
        }
        if stopped {
            for job in s.waiting.remove(stream).unwrap_or_default() {
                s.in_flight.remove(&job.row.client_id);
            }
        }
    }

    /// The scope ended: nothing is in flight any more.
    pub(crate) fn clear(&self) {
        *self.state.lock() = State::default();
    }
}

/// When an outcome handed off at `now` turns unknown.
fn deadline_from(now: i64) -> i64 {
    now.saturating_add(i64::try_from(DM_APPEND_DEADLINE.as_millis()).unwrap_or(i64::MAX))
}

impl<D: OutboxDeps> Outbox<D> {
    /// Run each stream's handed-off appends, one in flight per stream, and
    /// the handed-off calls beside them.
    pub(crate) async fn append_loop(&self) {
        let mut running = FuturesUnordered::new();
        loop {
            for (stream, jobs) in self.appends.take_ready() {
                running.push(self.append_run(stream, jobs).left_future());
            }
            for job in self.appends.take_calls() {
                running.push(self.call_run(job).right_future());
            }
            // `biased`, as the pump's: one order on every run.
            tokio::select! {
                biased;
                Some(()) = running.next(), if !running.is_empty() => {}
                () = self.appends.wake.notified() => {}
            }
        }
    }

    async fn append_run(&self, stream: String, jobs: Vec<Job>) {
        let frames: Vec<(Vec<u8>, i64)> = jobs
            .iter()
            .map(|job| (job.frame.clone(), job.row.stream_seq))
            .collect();
        let kind = jobs[0].row.kind;
        let outcomes = self.deps.append(kind, &stream, &frames).await;
        let gate = self.appends.gate.lock().await;
        let stopped = outcomes.len() < jobs.len()
            || outcomes
                .iter()
                .any(|outcome| *outcome != AttemptOutcome::Sent);
        // The rows past the one that stopped were not appended: they stay
        // queued, due at the `next_at` their hand-off set.
        for (job, outcome) in jobs.iter().zip(outcomes) {
            if let Err(e) = self.settle_append(job, outcome).await {
                tracing::warn!(error = %e, stream = %stream, "outbox landing not stored");
            }
        }
        self.appends.finish(&stream, &jobs, stopped);
        drop(gate);
        self.wake();
    }

    /// Run `job`'s call; its answer settles the row while it is still
    /// unsettled (a receipt lands it, an unanswered call is sent again
    /// after the short retry), and a row settled meanwhile (the peer's ack,
    /// a deadline, a discard) is left as it is.
    async fn call_run(&self, job: Job) {
        let kind = job.row.kind;
        let outcome = self
            .deps
            .call(kind, &job.row.stream, job.frame.clone())
            .await;
        let gate = self.appends.gate.lock().await;
        let settled = match self.deps.row(&job.row.client_id).await {
            Ok(Some(row)) if is_unsettled(row.state) => {
                self.settle_attempt(&row, outcome, job.sent_at).await
            }
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        };
        if let Err(e) = settled {
            tracing::warn!(error = %e, kind = kind.as_str(), "outbox call outcome not stored");
        }
        self.appends.finish_call(&job.row.client_id);
        drop(gate);
        self.wake();
    }

    /// `job`'s append ended with `outcome`, however late: the row as it is
    /// now is settled by it, and a row no longer `queued` (landed by the
    /// tick or a restart's reconcile, or gone) is left as it is.
    async fn settle_append(&self, job: &Job, outcome: AttemptOutcome) -> Result<(), OutboxError> {
        match self.deps.row(&job.row.client_id).await? {
            Some(row) if row.state == SendState::Queued => {
                self.settle_attempt(&row, outcome, job.sent_at).await
            }
            _ => Ok(()),
        }
    }

    /// When a pass next has work on a stream with a row in flight, by
    /// [`select_due`](Self::select_due)'s walk: an in-flight row's own
    /// `next_at` (its re-hint past the deadline, or its re-schedule within
    /// it), and each unsettled row past it, up to the first row still to send
    /// that is not in flight, which holds the rest back. The store's
    /// `next_wake` cannot see the appender: it counts every row behind a
    /// `queued` one as blocked, so a re-hint due behind an append in flight
    /// never woke a pass (E2.5i). Each of these rows is attempted or
    /// re-scheduled when due, so a pass does not spin on it; a stream whose
    /// attempt is running is left out (its end wakes the pump).
    pub(crate) async fn busy_wake(&self) -> Result<Option<i64>, OutboxError> {
        let mut wake: Option<i64> = None;
        let attempting = self.attempting.streams();
        for stream in self.appends.walked().difference(&attempting) {
            for row in self.deps.stream_rows(stream).await? {
                let in_flight = self.appends.in_flight(&row.client_id);
                if in_flight || is_unsettled(row.state) {
                    wake = Some(wake.map_or(row.next_at, |w| w.min(row.next_at)));
                }
                if !in_flight && row.state == SendState::Queued {
                    break;
                }
            }
        }
        Ok(wake)
    }

    /// The due rows to attempt now, read under the gate: a stream with a row
    /// in flight is walked past it (its later rows are due, unless a row
    /// still to send that is not due holds them back). An in-flight row
    /// within its append's deadline is not attempted, and one whose
    /// `next_at` passed is scheduled again, so a pass does not spin on it;
    /// past the deadline a due one is attempted for its hint only
    /// (`Outbox::attempt_run` starts no second append). A stream whose
    /// attempt is running is skipped whole (plan E2.5j): its next run is
    /// selected once that attempt settled.
    pub(crate) async fn select_due(
        &self,
        due: Vec<OutboxRow>,
        now: i64,
    ) -> Result<Vec<OutboxRow>, OutboxError> {
        let attempting = self.attempting.streams();
        let streams = self.appends.walked();
        let mut rows: Vec<OutboxRow> = due
            .into_iter()
            .filter(|row| !streams.contains(&row.stream) && !attempting.contains(&row.stream))
            .collect();
        for stream in streams.difference(&attempting) {
            for row in self.deps.stream_rows(stream).await? {
                if self.appends.in_flight(&row.client_id) {
                    if row.next_at > now {
                        continue;
                    }
                    if self.appends.within_deadline(&row.client_id, now) {
                        self.hand_off_call(&row).await?;
                    } else {
                        rows.push(row);
                    }
                    continue;
                }
                if row.next_at > now {
                    if row.state == rekindle_types::message::SendState::Queued {
                        break;
                    }
                    continue;
                }
                if is_unsettled(row.state) {
                    rows.push(row);
                }
            }
        }
        Ok(rows)
    }
}
