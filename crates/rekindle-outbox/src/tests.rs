//! The engine over an in-memory host: a fixed clock, a counter RNG and
//! scripted attempt outcomes.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_types::message::SendState;
use rekindle_utils::random::RandomSource;
use rekindle_utils::time::Clock;

use crate::state::{apply, merge, Moved, PeerAck, StateInput, Step, StoredSend, Write};
use crate::*;

mod reask;

struct FixedClock(AtomicI64);
impl Clock for FixedClock {
    fn monotonic(&self) -> std::time::Instant {
        tokio::time::Instant::now().into_std()
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.0.load(Ordering::SeqCst)).unwrap_or(0)
    }
}

struct CounterRng(AtomicU64);
impl rand::CryptoRng for CounterRng {}
impl RandomSource for CounterRng {
    fn fill(&self, dest: &mut [u8]) {
        let n = self.0.fetch_add(1, Ordering::SeqCst).to_le_bytes();
        for (i, b) in dest.iter_mut().enumerate() {
            *b = n[i % 8];
        }
    }
}

#[derive(Default)]
struct Store {
    rows: Vec<OutboxRow>,
    last_seq: HashMap<String, i64>,
    received: HashSet<(String, String)>,
    published: Vec<(String, SendState)>,
    /// Every published change in full (its retry, plan §3.0 lane 1).
    changes: Vec<StateChange>,
    /// Acks noted on rows not yet landed, by client id.
    acks: HashMap<String, PeerAck>,
    /// Settled entries as their echo keeps them: stream, seq, id, state.
    settled: Vec<(String, i64, String, SendState)>,
    attempted: Vec<String>,
    outcomes: VecDeque<AttemptOutcome>,
    /// The stream seqs our chain records as landed (`on_chain`).
    chain: HashSet<i64>,
    /// How many appends started, and what holds each until notified.
    appends: usize,
    hold: Option<Arc<tokio::sync::Notify>>,
    /// How many calls started ([`OutboxDeps::call`]), what holds each until
    /// notified, and their scripted outcomes (`Sent`, a receipt, when none).
    calls: usize,
    call_hold: Option<Arc<tokio::sync::Notify>>,
    call_outcomes: VecDeque<AttemptOutcome>,
    /// Streams whose attempts never resolve (an unanswered call).
    hang: HashSet<String>,
    /// The store reads every stream as ordered, as the SQL's `due` does
    /// with a dependency kind missing from its list: a store that
    /// disagrees with `StreamKind::ordered` (lead decision 51).
    ordered_all: bool,
}

impl Store {
    /// `client_id` as stored now, its noted ack with it.
    fn stored(&self, client_id: &str) -> Option<StoredSend> {
        let row = self.rows.iter().find(|r| r.client_id == client_id)?;
        Some(StoredSend {
            state: row.state,
            ack: self.acks.get(client_id).copied(),
        })
    }

    /// `target` through the lattice against `client_id` as stored now.
    fn apply(&self, client_id: &str, target: SendState, step: Step) -> Option<(Write, Moved)> {
        Some(apply(self.stored(client_id)?, target, step))
    }

    /// Project `state` and remove `client_id`.
    fn settle_row(&mut self, client_id: &str, state: SendState) {
        if let Some(r) = self.rows.iter().find(|r| r.client_id == client_id).cloned() {
            self.settled
                .push((r.stream, r.stream_seq, r.client_id, state));
        }
        self.rows.retain(|r| r.client_id != client_id);
        self.acks.remove(client_id);
    }
}

#[derive(Clone)]
struct Fake {
    store: Arc<Mutex<Store>>,
    clock: Arc<FixedClock>,
    rng: Arc<CounterRng>,
}

/// Whether `row` waits at `now` (`i64::MIN`: whatever its time, as
/// `next_wake` reads): `ordered_all` is [`Store::ordered_all`].
fn blocked(rows: &[OutboxRow], row: &OutboxRow, ordered_all: bool, now: i64) -> bool {
    // A dependency stream (plan HOL1): only the named row holds it, as the
    // store's `DUE_UNBLOCKED` reads.
    if !row.kind.ordered() && !ordered_all {
        return row
            .depends_on
            .as_ref()
            .is_some_and(|d| rows.iter().any(|h| &h.client_id == d));
    }
    // A disagreeing store reads as the SQL does: a queued row ahead holds
    // this one only while it is not due at `now`.
    rows.iter().any(|h| {
        h.stream == row.stream
            && h.stream_seq < row.stream_seq
            && match h.state {
                SendState::Queued => !ordered_all || h.next_at > now,
                SendState::Failed => true,
                _ => false,
            }
    })
}

fn unsettled(row: &OutboxRow) -> bool {
    matches!(row.state, SendState::Queued | SendState::Sent)
}

#[async_trait]
impl OutboxDeps for Fake {
    type Echo = ();
    fn clock(&self) -> &dyn Clock {
        &*self.clock
    }
    fn rng(&self) -> &dyn RandomSource {
        &*self.rng
    }
    async fn insert(&self, row: NewOutboxRow, (): ()) -> Result<i64, OutboxError> {
        let mut s = self.store.lock();
        let seq = s.last_seq.entry(row.stream.clone()).or_insert(0);
        *seq += 1;
        let stream_seq = *seq;
        let depends_on = (s.rows.iter().rev())
            .find(|h| {
                h.stream == row.stream
                    && row
                        .waits_for
                        .iter()
                        .any(|p| h.client_id.starts_with(p.as_str()))
            })
            .map(|h| h.client_id.clone());
        for r in s.rows.iter_mut().filter(|r| r.stream == row.stream) {
            let taken = (r.depends_on.as_deref())
                .is_some_and(|d| row.takes_over.iter().any(|p| d.starts_with(p.as_str())));
            if taken {
                r.depends_on = Some(row.client_id.clone());
            }
        }
        s.rows.push(OutboxRow {
            stream: row.stream,
            stream_seq,
            client_id: row.client_id,
            kind: row.kind,
            payload: row.payload,
            state: SendState::Queued,
            attempts: 0,
            next_at: row.next_at,
            deadline: row.deadline,
            wedge_reason: None,
            sent_at: None,
            depends_on,
        });
        Ok(stream_seq)
    }
    async fn due(&self, now: i64) -> Result<Vec<OutboxRow>, OutboxError> {
        let s = self.store.lock();
        let mut due: Vec<OutboxRow> = (s.rows.iter())
            .filter(|r| {
                unsettled(r) && r.next_at <= now && !blocked(&s.rows, r, s.ordered_all, now)
            })
            .cloned()
            .collect();
        due.sort_by(|a, b| {
            (a.next_at, &a.stream, a.stream_seq).cmp(&(b.next_at, &b.stream, b.stream_seq))
        });
        Ok(due)
    }
    async fn prefix_rows(&self, prefix: &str) -> Result<Vec<OutboxRow>, OutboxError> {
        let s = self.store.lock();
        let mut rows: Vec<OutboxRow> = (s.rows.iter())
            .filter(|r| r.stream.starts_with(prefix))
            .cloned()
            .collect();
        rows.sort_by(|a, b| (&a.stream, a.stream_seq).cmp(&(&b.stream, b.stream_seq)));
        Ok(rows)
    }
    async fn stream_rows(&self, stream: &str) -> Result<Vec<OutboxRow>, OutboxError> {
        let s = self.store.lock();
        let mut rows: Vec<OutboxRow> = (s.rows.iter())
            .filter(|r| unsettled(r) && r.stream == stream)
            .cloned()
            .collect();
        rows.sort_by_key(|r| r.stream_seq);
        Ok(rows)
    }
    async fn expired(&self, now: i64) -> Result<Vec<OutboxRow>, OutboxError> {
        let s = self.store.lock();
        Ok(s.rows
            .iter()
            .filter(|r| unsettled(r) && r.deadline <= now)
            .cloned()
            .collect())
    }
    async fn reschedule_sent(&self, stream: &str, at: i64) -> Result<usize, OutboxError> {
        let mut s = self.store.lock();
        let rows = s.rows.iter_mut().filter(|r| r.stream == stream);
        let late = rows.filter(|r| r.state == SendState::Sent && r.next_at > at);
        Ok(late.map(|r| r.next_at = at).count())
    }
    async fn next_wake(&self) -> Result<Option<i64>, OutboxError> {
        let s = self.store.lock();
        let due = s
            .rows
            .iter()
            .filter(|r| unsettled(r) && !blocked(&s.rows, r, s.ordered_all, i64::MIN))
            .map(|r| r.next_at);
        let ends = s.rows.iter().filter(|r| unsettled(r)).map(|r| r.deadline);
        Ok(due.chain(ends).min())
    }
    async fn row(&self, client_id: &str) -> Result<Option<OutboxRow>, OutboxError> {
        Ok(self
            .store
            .lock()
            .rows
            .iter()
            .find(|r| r.client_id == client_id)
            .cloned())
    }
    async fn row_at(&self, stream: &str, seq: i64) -> Result<Option<OutboxRow>, OutboxError> {
        let s = self.store.lock();
        Ok(s.rows
            .iter()
            .find(|r| r.stream == stream && r.stream_seq == seq)
            .cloned())
    }
    async fn update(&self, client_id: &str, u: &RowUpdate) -> Result<Option<Moved>, OutboxError> {
        let step = if u.retry { Step::Retry } else { Step::Update };
        let mut s = self.store.lock();
        let Some((write, moved)) = s.apply(client_id, u.state, step) else {
            return Ok(None);
        };
        match write {
            Write::Settle(state) => s.settle_row(client_id, state),
            Write::Store(state) => {
                if let Some(r) = s.rows.iter_mut().find(|r| r.client_id == client_id) {
                    (r.state, r.attempts, r.next_at) = (state, u.attempts, u.next_at);
                    (r.deadline, r.wedge_reason) = (u.deadline, u.wedge_reason.clone());
                }
            }
        }
        Ok(Some(moved))
    }
    async fn reschedule_queued(&self, id: &str, n: u32, at: i64) -> Result<(), OutboxError> {
        let mut s = self.store.lock();
        let row = (s.rows.iter_mut()).find(|r| r.client_id == id && r.state == SendState::Queued);
        if let Some(r) = row {
            (r.attempts, r.next_at, r.wedge_reason) = (n, at, None);
        }
        Ok(())
    }
    async fn note_ack(&self, client_id: &str, ack: PeerAck) -> Result<Option<Moved>, OutboxError> {
        let mut s = self.store.lock();
        let Some(before) = s.stored(client_id).filter(|b| b.state == SendState::Queued) else {
            return Ok(None);
        };
        let kept = s.acks.entry(client_id.to_owned()).or_insert(ack);
        *kept = (*kept).max(ack);
        let after = StoredSend {
            state: before.state,
            ack: Some(*kept),
        };
        Ok(Some(Moved {
            before: before.shown(),
            after: after.shown(),
        }))
    }
    async fn settle(
        &self,
        client_id: &str,
        state: SendState,
    ) -> Result<Option<Moved>, OutboxError> {
        let mut s = self.store.lock();
        let Some((write, moved)) = s.apply(client_id, state, Step::Settle) else {
            return Ok(None);
        };
        let (Write::Settle(state) | Write::Store(state)) = write;
        s.settle_row(client_id, state);
        Ok(Some(moved))
    }
    async fn read_through(
        &self,
        st: &str,
        _: &str,
        up_to: i64,
    ) -> Result<Vec<(i64, String)>, OutboxError> {
        Ok(self.ack_range(st, (0, up_to), PeerAck::Read))
    }
    async fn deliver(
        &self,
        st: &str,
        _: &str,
        cover: Cover,
    ) -> Result<Vec<(i64, String)>, OutboxError> {
        // The fake's chain seq is the stream seq.
        let r = match cover {
            Cover::Seq(s) => (s, s),
            Cover::ChainThrough(t) => (1, t),
        };
        Ok(self.ack_range(st, r, PeerAck::Delivered))
    }
    async fn mark_received(&self, stream: &str, id: &str) -> Result<bool, OutboxError> {
        Ok(self
            .store
            .lock()
            .received
            .insert((stream.into(), id.into())))
    }
    async fn unsettled_count(&self) -> Result<u64, OutboxError> {
        Ok(self.store.lock().rows.len() as u64)
    }
    async fn append(&self, _: StreamKind, _: &str, f: &[(Vec<u8>, i64)]) -> Vec<AttemptOutcome> {
        let hold = {
            let mut s = self.store.lock();
            s.appends += 1;
            s.hold.clone()
        };
        if let Some(hold) = hold {
            hold.notified().await;
        }
        vec![AttemptOutcome::Sent; f.len()]
    }
    async fn call(&self, _: StreamKind, _: &str, _: Vec<u8>) -> AttemptOutcome {
        let hold = {
            let mut s = self.store.lock();
            s.calls += 1;
            s.call_hold.clone()
        };
        if let Some(hold) = hold {
            hold.notified().await;
        }
        let mut s = self.store.lock();
        s.call_outcomes.pop_front().unwrap_or(AttemptOutcome::Sent)
    }
    async fn land(
        &self,
        client_id: &str,
        attempts: u32,
        next_at: i64,
    ) -> Result<Landed, OutboxError> {
        let ack = {
            let mut s = self.store.lock();
            let Some(r) = s
                .rows
                .iter_mut()
                .find(|r| r.client_id == client_id && r.state == SendState::Queued)
            else {
                return Ok(Landed::Unchanged);
            };
            (r.state, r.attempts, r.next_at) = (SendState::Sent, attempts, next_at);
            s.acks.get(client_id).copied()
        };
        let Some(ack) = ack else {
            return Ok(Landed::Waiting);
        };
        let state = merge(SendState::Sent, StateInput::Peer(ack));
        self.store.lock().settle_row(client_id, state);
        Ok(Landed::Settled(state))
    }
    async fn on_chain(&self, _: &str, seqs: &[i64]) -> Result<Vec<i64>, OutboxError> {
        let s = self.store.lock();
        Ok(seqs
            .iter()
            .copied()
            .filter(|q| s.chain.contains(q))
            .collect())
    }
    async fn attempt(&self, row: &OutboxRow) -> AttemptOutcome {
        let hangs = {
            let mut s = self.store.lock();
            s.attempted.push(row.client_id.clone());
            s.hang.contains(&row.stream)
        };
        if hangs {
            std::future::pending::<()>().await;
        }
        let mut s = self.store.lock();
        s.outcomes.pop_front().unwrap_or(AttemptOutcome::Sent)
    }
    fn publish(&self, change: StateChange) {
        let mut s = self.store.lock();
        s.changes.push(change.clone());
        s.published.push((change.client_id, change.state));
    }
}

fn outbox() -> Outbox<Fake> {
    Outbox::new(Fake {
        store: Arc::default(),
        clock: Arc::new(FixedClock(AtomicI64::new(1_000))),
        rng: Arc::new(CounterRng(AtomicU64::new(7))),
    })
}

fn entry(kind: StreamKind, scope: &str, id: &str) -> Entry {
    Entry {
        kind,
        scope: scope.into(),
        client_id: id.into(),
        payload: b"frame".to_vec(),
        deadline: Deadline::Horizon,
    }
}

fn script(o: &Outbox<Fake>, outcomes: impl IntoIterator<Item = AttemptOutcome>) {
    o.deps().store.lock().outcomes.extend(outcomes);
}

fn state_of(o: &Outbox<Fake>, id: &str) -> Option<SendState> {
    o.deps()
        .store
        .lock()
        .rows
        .iter()
        .find(|r| r.client_id == id)
        .map(|r| r.state)
}

fn advance(o: &Outbox<Fake>, ms: i64) {
    o.deps().clock.0.fetch_add(ms, Ordering::SeqCst);
}

mod channel_post;

mod calling;
mod landing;

mod media_key;

mod deadline;

mod ack_order;

mod depends;
mod fake_acks;
mod forward;
mod independence;

mod lifecycle;
mod mek_transfer;
mod monotone;
#[cfg(test)]
mod retrying;
mod settle_landed;
