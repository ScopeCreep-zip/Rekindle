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

use crate::state::{merge, PeerAck, StateInput};
use crate::*;

struct FixedClock(AtomicI64);
impl Clock for FixedClock {
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
    attempted: Vec<String>,
    outcomes: VecDeque<AttemptOutcome>,
}

#[derive(Clone)]
struct Fake {
    store: Arc<Mutex<Store>>,
    clock: Arc<FixedClock>,
    rng: Arc<CounterRng>,
}

fn blocked(rows: &[OutboxRow], row: &OutboxRow) -> bool {
    rows.iter().any(|h| {
        h.stream == row.stream
            && h.stream_seq < row.stream_seq
            && matches!(h.state, SendState::Queued | SendState::Failed)
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
        });
        Ok(stream_seq)
    }
    async fn due(&self, now: i64) -> Result<Vec<OutboxRow>, OutboxError> {
        let s = self.store.lock();
        let mut due: Vec<OutboxRow> = (s.rows.iter())
            .filter(|r| unsettled(r) && r.next_at <= now && !blocked(&s.rows, r))
            .cloned()
            .collect();
        due.sort_by(|a, b| {
            (a.next_at, &a.stream, a.stream_seq).cmp(&(b.next_at, &b.stream, b.stream_seq))
        });
        Ok(due)
    }
    async fn expired(&self, now: i64) -> Result<Vec<OutboxRow>, OutboxError> {
        let s = self.store.lock();
        Ok(s.rows
            .iter()
            .filter(|r| unsettled(r) && r.deadline <= now)
            .cloned()
            .collect())
    }
    async fn next_wake(&self) -> Result<Option<i64>, OutboxError> {
        let s = self.store.lock();
        let due = s
            .rows
            .iter()
            .filter(|r| unsettled(r) && !blocked(&s.rows, r))
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
    async fn update(&self, client_id: &str, u: &RowUpdate) -> Result<(), OutboxError> {
        let mut s = self.store.lock();
        if let Some(r) = s.rows.iter_mut().find(|r| r.client_id == client_id) {
            (r.state, r.attempts, r.next_at) = (u.state, u.attempts, u.next_at);
            (r.deadline, r.wedge_reason) = (u.deadline, u.wedge_reason.clone());
        }
        Ok(())
    }
    async fn settle(&self, client_id: &str, _: SendState) -> Result<(), OutboxError> {
        self.store.lock().rows.retain(|r| r.client_id != client_id);
        Ok(())
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
    async fn attempt(&self, row: &OutboxRow) -> AttemptOutcome {
        let mut s = self.store.lock();
        s.attempted.push(row.client_id.clone());
        s.outcomes.pop_front().unwrap_or(AttemptOutcome::Sent)
    }
    fn publish(&self, change: StateChange) {
        self.store
            .lock()
            .published
            .push((change.client_id, change.state));
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

#[test]
fn the_lattice_only_climbs_and_failed_only_replaces_unsettled() {
    use SendState::*;
    assert_eq!(merge(Queued, StateInput::LocalSent), Sent);
    assert_eq!(merge(Delivered, StateInput::LocalSent), Delivered);
    assert_eq!(merge(Read, StateInput::Peer(PeerAck::Delivered)), Read);
    assert_eq!(merge(Sent, StateInput::Peer(PeerAck::Read)), Read);
    assert_eq!(merge(Sent, StateInput::Failed), Failed);
    assert_eq!(merge(Queued, StateInput::Failed), Failed);
    assert_eq!(merge(Delivered, StateInput::Failed), Delivered);
    assert_eq!(merge(Failed, StateInput::LocalSent), Failed);
    assert_eq!(
        merge(Failed, StateInput::Peer(PeerAck::Delivered)),
        Delivered
    );
}

#[tokio::test]
async fn a_local_ok_is_sent_and_only_the_peer_makes_it_delivered() {
    let o = outbox();
    let at = o
        .enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "a"), Some(SendState::Sent));
    // Unacknowledged, it is sent again later under the same id.
    advance(&o, 600_000);
    o.pump().await.unwrap();
    assert_eq!(o.deps().store.lock().attempted, ["a", "a"]);
    assert!(o
        .acknowledge(&at.stream, at.stream_seq, PeerAck::Delivered)
        .await
        .unwrap());
    assert_eq!(state_of(&o, "a"), None);
    let published = o.deps().store.lock().published.clone();
    let states: Vec<_> = published.into_iter().map(|(_, s)| s).collect();
    assert_eq!(
        states,
        [SendState::Queued, SendState::Sent, SendState::Delivered]
    );
    assert!(!o
        .acknowledge(&at.stream, at.stream_seq, PeerAck::Read)
        .await
        .unwrap());
}

#[tokio::test]
async fn a_dht_write_settles_when_sent() {
    let o = outbox();
    o.enqueue(entry(StreamKind::DhtWrite, "rec/0", "w"), ())
        .await
        .unwrap();
    assert_eq!(o.unsettled().await.unwrap(), 1);
    o.pump().await.unwrap();
    assert_eq!(o.unsettled().await.unwrap(), 0);
}

#[tokio::test]
async fn the_stream_head_blocks_its_stream_only() {
    let o = outbox();
    script(&o, [AttemptOutcome::Deferred]);
    o.enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::Dm, "bob", "b"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::Dm, "carol", "c"), ())
        .await
        .unwrap();
    let next = o.pump().await.unwrap().unwrap();
    assert_eq!(o.deps().store.lock().attempted, ["a", "c"]);
    assert_eq!(state_of(&o, "b"), Some(SendState::Queued));
    advance(&o, next - 1_000);
    o.pump().await.unwrap();
    o.pump().await.unwrap();
    assert_eq!(o.deps().store.lock().attempted, ["a", "c", "a", "b"]);
}

#[tokio::test]
async fn past_the_deadline_a_row_fails_and_retry_keeps_its_id() {
    let o = outbox();
    script(&o, std::iter::repeat_n(AttemptOutcome::Deferred, 64));
    o.enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::Dm, "bob", "b"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    advance(
        &o,
        i64::try_from(rekindle_crypto::horizon::DELIVERY_HORIZON.as_millis()).unwrap(),
    );
    o.pump().await.unwrap();
    let row = o.deps().row("a").await.unwrap().unwrap();
    assert_eq!(
        (row.state, row.wedge_reason.as_deref()),
        (SendState::Failed, Some(DEADLINE_REASON))
    );
    // `b` passed the same deadline; a local send never revives a failed row.
    assert_eq!(state_of(&o, "b"), Some(SendState::Failed));
    o.deps().store.lock().outcomes.clear();
    assert!(o.retry("a").await.unwrap());
    assert_eq!(state_of(&o, "a"), Some(SendState::Queued));
    o.pump().await.unwrap();
    assert_eq!(
        o.deps().store.lock().attempted.last().map(String::as_str),
        Some("a")
    );
    assert_eq!(state_of(&o, "a"), Some(SendState::Sent));
}

#[tokio::test]
async fn discard_settles_failed_and_frees_the_stream() {
    let o = outbox();
    script(&o, [AttemptOutcome::Wedged("prekey_rejected".into())]);
    o.enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::Dm, "bob", "b"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    o.pump().await.unwrap();
    assert_eq!(o.deps().store.lock().attempted, ["a"]);
    assert!(o.discard("a").await.unwrap());
    assert_eq!(state_of(&o, "a"), None);
    assert_eq!(
        o.deps().store.lock().published.last(),
        Some(&("a".into(), SendState::Failed))
    );
    o.pump().await.unwrap();
    assert_eq!(o.deps().store.lock().attempted, ["a", "b"]);
}

#[tokio::test]
async fn rows_outlive_the_engine_and_resends_are_received_once() {
    let first = outbox();
    script(&first, [AttemptOutcome::Deferred]);
    first
        .enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    first.pump().await.unwrap();
    // A restart: a new engine over the same rows sends the same id.
    let again = Outbox::new(first.deps().clone());
    advance(&again, 10_000);
    again.pump().await.unwrap();
    assert_eq!(again.deps().store.lock().attempted, ["a", "a"]);
    assert!(again.receive("dm:bob", "a").await.unwrap());
    assert!(!again.receive("dm:bob", "a").await.unwrap());
}

#[test]
fn backoff_is_jittered_within_its_step_and_capped() {
    let rng = CounterRng(AtomicU64::new(3));
    let ms = |kind: StreamKind, n| kind.policy().delay(n, &rng).as_millis();
    let first = ms(StreamKind::Dm, 1);
    assert!((500..=1_000).contains(&first), "{first}");
    let third = ms(StreamKind::Dm, 3);
    assert!((2_000..=4_000).contains(&third), "{third}");
    let late = ms(StreamKind::MediaKey, 30);
    assert!((7_500..=15_000).contains(&late), "{late}");
    let again = CounterRng(AtomicU64::new(3));
    assert_eq!(StreamKind::Dm.policy().delay(1, &again).as_millis(), first);
    assert_eq!(
        StreamKind::parse("friend_handshake"),
        Some(StreamKind::FriendHandshake)
    );
    assert_eq!(StreamKind::parse("group_dm"), None);
}
