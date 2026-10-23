//! The DM pump (live A/B, decided 2026-10-23): each `dm` and `group_dm`
//! stream has one ordered appender on the outbox's scope. A pass sends a
//! run's hints at once and hands the run's chain appends here
//! ([`AttemptOutcome::Appending`]); one append is in flight per stream, in
//! chain order, and the pass returns without awaiting it. A row whose
//! append is in flight stays `queued` and is not attempted again; the
//! append's outcome is its landing input (the two-input settle). The
//! appender stops with [`Outbox::run`]'s scope, and a row whose append was
//! cut off is attempted again by a later pass, at the `next_at` its hand-off
//! set.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use futures::stream::{FuturesUnordered, StreamExt};
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::deps::{AttemptOutcome, OutboxDeps, OutboxError, OutboxRow};
use crate::engine::Outbox;
use crate::state::is_unsettled;

/// One row's append, handed off by a pass.
pub(crate) struct Job {
    /// The row as the pass read it (its `attempts` before the hand-off).
    pub(crate) row: OutboxRow,
    pub(crate) frame: Vec<u8>,
}

#[derive(Default)]
struct State {
    /// The client ids handed off and not landed.
    in_flight: HashSet<String>,
    /// Per stream, the jobs waiting for the stream's append, in order.
    waiting: BTreeMap<String, VecDeque<Job>>,
    /// The streams with an append running.
    busy: BTreeSet<String>,
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
        self.state.lock().in_flight.contains(client_id)
    }

    /// The streams with a row in flight, in order.
    pub(crate) fn streams(&self) -> BTreeSet<String> {
        let s = self.state.lock();
        s.busy.iter().chain(s.waiting.keys()).cloned().collect()
    }

    /// Queue `jobs` (one stream's run, in order) behind its stream's append.
    pub(crate) fn hand(&self, jobs: Vec<Job>) {
        if jobs.is_empty() {
            return;
        }
        let mut s = self.state.lock();
        for job in jobs {
            s.in_flight.insert(job.row.client_id.clone());
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

impl<D: OutboxDeps> Outbox<D> {
    /// Run each stream's handed-off appends, one in flight per stream.
    pub(crate) async fn append_loop(&self) {
        let mut running = FuturesUnordered::new();
        loop {
            for (stream, jobs) in self.appends.take_ready() {
                running.push(self.append_run(stream, jobs));
            }
            tokio::select! {
                () = self.appends.wake.notified() => {}
                Some(()) = running.next(), if !running.is_empty() => {}
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
            if let Err(e) = self.settle_attempt(&job.row, outcome).await {
                tracing::warn!(error = %e, stream = %stream, "outbox landing not stored");
            }
        }
        self.appends.finish(&stream, &jobs, stopped);
        drop(gate);
        self.wake();
    }

    /// The due rows to attempt now, read under the gate: a stream with a row
    /// in flight is walked past it (its later rows are due, unless a row
    /// still to send that is not due holds them back), and its in-flight
    /// rows are not attempted; one whose `next_at` passed is scheduled
    /// again, so a pass does not spin on it.
    pub(crate) async fn select_due(
        &self,
        due: Vec<OutboxRow>,
        now: i64,
    ) -> Result<Vec<OutboxRow>, OutboxError> {
        let streams = self.appends.streams();
        let mut rows: Vec<OutboxRow> = due
            .into_iter()
            .filter(|row| !streams.contains(&row.stream))
            .collect();
        for stream in &streams {
            for row in self.deps.stream_rows(stream).await? {
                if self.appends.in_flight(&row.client_id) {
                    if row.next_at <= now {
                        self.hand_off(&row).await?;
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
