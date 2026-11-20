//! A set's fanout over a record's replicas (plan E2.B slices S4 and S11;
//! plan VU2 items 1, 9 and 13), after veilid-core 0.5.8 (Rekindle's fork,
//! ADR 0017; `veilid-core/src`):
//!
//! - **lanes:** at most [`LANES`] set RPCs are in flight at once
//!   (`set_value_fanout`, `veilid_config.rs:1012`), and no more than
//!   [`SET_COUNT`] counting the accepts, until a lane in flight passes the
//!   slow threshold [`STRAGGLER_FLOOR`] (`set_value.rs:676`
//!   `ThrottleAtConsensus`, `fanout_queue.rs:500-552`), sent in closeness order
//!   over the replicas the node reaches; each arrives at its replica a
//!   [`latency::SET_RPC`] drawn from the set's own stream ([`set_stream`])
//!   after it left, and **the replica stores it then**
//!   (readers and watchers see it from that moment), and its answer is read
//!   then. An RPC whose replica the node no longer reaches at arrival, or
//!   arriving while the node is offline, is lost: no answer comes, and its
//!   lane waits [`RPC_TIMEOUT`] (`veilid_config.rs:944`), or the straggler
//!   cut below;
//! - **a replica's answer** (`storage_manager/set_value.rs:819-842`): it
//!   keeps the first value it receives at a seq, answers the same value
//!   (ciphertext included, [`super::replicas::Cipher`]) as a success and
//!   anything older or same-seq different with what it holds; a value past
//!   the size rule is an invalid message ([`super::size`]);
//! - **the fanout's side** (`set_value.rs:24-115`): a [`Tally`] counts the
//!   holders of each value at the fanout's seq. An answer older than the
//!   fanout's value is stale; a newer one restarts the fanout on it (every
//!   accepted, conflicting and stale replica is queued again,
//!   `fanout_call.rs:567-573`); a same-seq rival is a conflict unless it now
//!   beats the fanout's value, when the fanout switches to it (the
//!   conflicting replicas are queued again and the accepted ones become
//!   conflicts, `fanout_call.rs:579-585`). An accept of a value the fanout
//!   has since left counts as a rival holder;
//! - **done** at [`SET_COUNT`] accepts (`set_value_count`,
//!   `veilid_config.rs:1011`) within the `consensus_width` closest nodes
//!   that did not time out or disqualify: those take no slot, so the window
//!   widens past a silent or invalid replica (`fanout_call.rs:401-441`; an
//!   invalid answer disqualifies, `set_value.rs:534-537`,
//!   `fanout_call.rs:586-588`). Here the queue starts as the window, the
//!   [`super::replicas::REPLICAS`] closest candidates the node reaches (an
//!   unreached store is a timeout before the call starts); each lane that
//!   times out and each invalid answer moves the next reached candidate
//!   from the reserve behind the window into the queue, so at most
//!   `consensus_width` candidates ever hold a slot and every accept counts.
//!   The reserve is asked only once a slot frees, never ahead of a slow
//!   window lane (0.5.8 `fanout_queue.rs:447-465` would dispatch it after
//!   the window's queued nodes, its accept counting only once a closer
//!   node times out): a reserve answer can come later than in Veilid,
//!   never earlier or counted where Veilid would not. The call returns then; RPCs still in flight
//!   land on their replicas, their answers unread. Otherwise it ends when
//!   every reached replica answered (exhausted), or at [`SET_TIMEOUT`] past
//!   the last accept (`fanout_call.rs:552-555`, `veilid_config.rs:1010`).
//!   While no queued replica is left and the lanes in flight cannot bring the
//!   accepts to consensus, a silent lane is cut after the larger of
//!   [`STRAGGLER_FLOOR`] and three times the slowest answer
//!   (`fanout_queue.rs:11-13,662-751`, `fanout_call.rs:105-110`);
//! - **landed** at consensus, or at an end by timeout or exhaustion with
//!   at least [`GET_COUNT`] accepts (`storage_manager/mod.rs:990-1030`);
//!   then the settled value is the local copy and the call returns it when
//!   it is not the value asked for or some replica holds a rival at its seq
//!   (`set_value.rs:753-760`). Below that nothing is stored
//!   (`local_record_store_interface.rs:61-62`).
//!   A fanout nobody waits on (a rehydration push) runs in one step, bounded
//!   by [`max_rpcs`].

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{DhtCallError, DhtErrorKind, RecordSchema};
use tokio::time::Instant;

use super::replicas::{self, Cipher, Replica, Tally, GET_COUNT, REPLICAS, SET_COUNT};
use super::{candidates, reaches, size, stores, watches};
use crate::latency;
use crate::network::Network;

/// Set RPCs in flight at once: `set_value_fanout` (`veilid_config.rs:1012`).
pub(crate) const LANES: usize = 6;
/// An RPC nobody answers times out: `rpc.timeout_ms` (`veilid_config.rs:944`).
pub(crate) const RPC_TIMEOUT: Duration = Duration::from_secs(5);
/// A set's deadline past its last accept: `set_value_timeout_ms`
/// (`veilid_config.rs:1010`).
pub(crate) const SET_TIMEOUT: Duration = Duration::from_secs(10);
/// The least a straggler that can no longer complete consensus is waited
/// for: 33 % of [`RPC_TIMEOUT`] (`fanout_call.rs:105-110`).
pub(crate) const STRAGGLER_FLOOR: Duration = Duration::from_millis(1_650);
/// The multiple of the slowest answer a straggler is waited for
/// (`fanout_queue.rs:11-13`).
const STRAGGLER_RTT_FACTOR: u32 = 3;

/// The RPCs one unawaited fanout holds: [`LANES`] × [`SET_TIMEOUT`] over
/// [`latency::SET_RPC`]'s min.
pub(crate) fn max_rpcs() -> usize {
    let rounds = SET_TIMEOUT.as_millis() / latency::SET_RPC.min.as_millis();
    LANES.saturating_mul(usize::try_from(rounds).unwrap_or(usize::MAX))
}

/// What a set's fanout did.
pub(crate) struct SetFanout {
    /// The value the fanout ended on: the writer's own, or one it switched
    /// or restarted to.
    pub value: Cipher,
    /// Whether [`SET_COUNT`] replicas accepted `value`.
    pub consensus: bool,
    /// How many replicas accepted `value`.
    pub accepted: usize,
    /// Whether a replica holds another value at `value`'s seq.
    pub conflicted: bool,
    /// The replicas that stored a value in this fanout, in order (one may
    /// appear twice: a restart sent it another value).
    pub stored: Vec<usize>,
}

impl SetFanout {
    /// Whether the set landed (see the module notes).
    pub(crate) fn landed(&self) -> bool {
        self.consensus || self.accepted >= GET_COUNT
    }
}

/// A replica's answer to a set (`set_value.rs:819-842`).
enum Offer {
    Stored,
    Same,
    Ignored(Cipher),
}

fn offer(replica: &mut Replica, subkey: usize, value: &Cipher) -> Offer {
    match replica.cipher(subkey) {
        Some(held) if held == *value => Offer::Same,
        Some(held) if held.value.seq >= value.value.seq => Offer::Ignored(held),
        _ => {
            replica.put(subkey, value);
            Offer::Stored
        }
    }
}

/// `value` reaches replica `r`: its answer, or `None` for an invalid
/// message (past the size rule).
fn deliver(
    schema: &RecordSchema,
    replicas: &mut [Replica],
    r: usize,
    subkey: usize,
    value: &Cipher,
) -> Option<Offer> {
    let replica = replicas
        .get_mut(r)
        .filter(|x| size::admits(schema, &x.values, subkey, &value.value))?;
    Some(offer(replica, subkey, value))
}

/// One fanout's state (see the module notes).
pub(crate) struct Fanout {
    ctx: Cipher,
    queue: VecDeque<usize>,
    /// The reached candidates past the window, closest first.
    reserve: VecDeque<usize>,
    accepted: Vec<usize>,
    conflict: Vec<usize>,
    stale: Vec<usize>,
    tally: Tally,
    stored: Vec<usize>,
    sent: usize,
}

impl Fanout {
    /// A fanout of `ours` over the `reach` candidates, closest first: the
    /// first [`REPLICAS`] are the window, the rest its reserve.
    pub(crate) fn new(reach: &[usize], ours: &Cipher) -> Self {
        let split = reach.len().min(REPLICAS);
        Self {
            ctx: ours.clone(),
            queue: reach[..split].iter().copied().collect(),
            reserve: reach[split..].iter().copied().collect(),
            accepted: Vec::new(),
            conflict: Vec::new(),
            stale: Vec::new(),
            tally: Tally::default(),
            stored: Vec::new(),
            sent: 0,
        }
    }

    fn done(&self) -> bool {
        self.accepted.len() >= SET_COUNT
    }

    /// The next replica to send to and the value it carries.
    pub(crate) fn next(&mut self) -> Option<(usize, Cipher)> {
        if self.done() {
            return None;
        }
        let r = self.queue.pop_front()?;
        self.sent += 1;
        Some((r, self.ctx.clone()))
    }

    /// A lane timed out or an answer was invalid: its node takes no slot,
    /// so the next reached candidate takes one (`fanout_call.rs:399-405`,
    /// `fanout_queue.rs:771-813`).
    pub(crate) fn slot_freed(&mut self) {
        if let Some(r) = self.reserve.pop_front() {
            self.queue.push_back(r);
        }
    }

    /// Queue `rs` again, in closeness order.
    fn requeue(&mut self, rs: Vec<usize>) {
        self.queue.extend(rs);
        self.queue.make_contiguous().sort_unstable();
    }

    fn accept(&mut self, r: usize) {
        self.tally.add(&self.ctx, r);
        self.accepted.push(r);
    }

    /// `value`, a rival of the fanout's at its seq, is held by `r`
    /// (0.5.8 `set_value.rs:94-109`).
    fn rival(&mut self, value: Cipher, r: usize) {
        self.tally.add(&value, r);
        if !self.tally.beats(&value, &self.ctx) {
            self.conflict.push(r);
            return;
        }
        self.ctx = value;
        let conflict = std::mem::take(&mut self.conflict);
        self.requeue(conflict);
        self.conflict = std::mem::take(&mut self.accepted);
        self.accepted.push(r);
    }

    /// `r` answered the set of `sent` with `answer` (0.5.8 `set_value.rs:24-90`).
    fn classify(&mut self, r: usize, sent: &Cipher, answer: Offer) {
        let held = match answer {
            Offer::Stored | Offer::Same => sent.clone(),
            Offer::Ignored(held) => held,
        };
        if held == self.ctx {
            self.accept(r);
        } else if held.value.seq < self.ctx.value.seq {
            self.stale.push(r);
        } else if held.value.seq > self.ctx.value.seq {
            self.ctx = held;
            self.tally.clear();
            let again: Vec<usize> = self
                .accepted
                .drain(..)
                .chain(self.conflict.drain(..))
                .chain(self.stale.drain(..))
                .collect();
            self.requeue(again);
            self.accept(r);
        } else {
            self.rival(held, r);
        }
    }

    /// `sent` arrives at replica `r`: its store takes it or answers what it
    /// holds. Whether the replica stored it.
    pub(crate) fn arrive(
        &mut self,
        schema: &RecordSchema,
        replicas: &mut [Replica],
        r: usize,
        subkey: usize,
        sent: &Cipher,
    ) -> bool {
        let Some(answer) = deliver(schema, replicas, r, subkey, sent) else {
            if !self.done() {
                self.slot_freed();
            }
            return false;
        };
        let stored = matches!(answer, Offer::Stored);
        if stored {
            self.stored.push(r);
        }
        if !self.done() {
            self.classify(r, sent, answer);
        }
        stored
    }

    /// Whether the straggler cut applies: no replica is left queued and the
    /// `in_flight` lanes cannot bring the accepts to consensus
    /// (`fanout_queue.rs:662-705`).
    fn cut(&self, in_flight: usize) -> bool {
        self.queue.is_empty() && self.accepted.len() + in_flight < SET_COUNT
    }

    pub(crate) fn finish(self) -> SetFanout {
        SetFanout {
            consensus: self.done(),
            accepted: self.accepted.len(),
            conflicted: self.tally.conflicted(&self.ctx),
            value: self.ctx,
            stored: self.stored,
        }
    }
}

/// A fanout nobody waits on, in one step (a rehydration push).
pub(crate) fn set_now(
    schema: &RecordSchema,
    replicas: &mut [Replica],
    reach: &[usize],
    subkey: usize,
    ours: &Cipher,
) -> SetFanout {
    let mut fanout = Fanout::new(reach, ours);
    let bound = max_rpcs();
    // Lane by lane in waves: a wave's RPCs all land, those after consensus
    // too, their answers unread.
    while fanout.sent < bound {
        let wave: Vec<(usize, Cipher)> = (0..LANES).map_while(|_| fanout.next()).collect();
        if wave.is_empty() {
            break;
        }
        for (r, value) in wave {
            fanout.arrive(schema, replicas, r, subkey, &value);
        }
    }
    fanout.finish()
}

/// One set RPC in flight.
struct Lane {
    sent: Instant,
    at: Instant,
    r: usize,
    value: Cipher,
    /// Whether an answer comes (else the lane waits for its timeout or cut).
    answers: bool,
}

/// `node`'s set of `ours` at `key`/`subkey`: each RPC lands as it arrives;
/// returns when the fanout ends (see the module notes). With `accepting`,
/// only that many of the closest replicas it reaches answer
/// ([`crate::Fault::WritePartial`]); the others never do.
pub(crate) async fn run(
    net: &Arc<Network>,
    node: usize,
    (key, subkey): (&str, u32),
    ours: &Cipher,
    accepting: Option<usize>,
) -> SetFanout {
    let (schema, reach) = {
        let inner = net.inner.lock();
        inner.records.get(key).map_or_else(
            || (None, Vec::new()),
            |r| {
                let reach = candidates(&r.replicas, &inner.faults.partitions, node);
                (Some(r.schema.clone()), reach)
            },
        )
    };
    let answering: Vec<usize> = reach
        .iter()
        .copied()
        .take(accepting.unwrap_or(usize::MAX))
        .collect();
    let mut fanout = Fanout::new(&reach, ours);
    let stream = set_stream(key, subkey, &ours.value);
    let mut lanes: Vec<Lane> = Vec::new();
    let mut last_accept = Instant::now();
    let mut slowest: Option<Duration> = None;
    while let Some(schema) = &schema {
        if fanout.done() {
            break;
        }
        let now = Instant::now();
        let counted = |lanes: &[Lane]| {
            lanes
                .iter()
                .filter(|l| now < l.sent + STRAGGLER_FLOOR)
                .count()
        };
        if watches::online(&net.inner.lock(), node) {
            while lanes.len() < LANES && fanout.accepted.len() + counted(&lanes) < SET_COUNT {
                let Some((r, value)) = fanout.next() else {
                    break;
                };
                let sent = Instant::now();
                let at = sent + net.latency.draw_stream(node, stream, latency::SET_RPC);
                let answers = answering.contains(&r);
                lanes.push(Lane {
                    sent,
                    at,
                    r,
                    value,
                    answers,
                });
            }
        }
        let wait = match slowest {
            Some(s) if fanout.cut(lanes.len()) => STRAGGLER_FLOOR.max(s * STRAGGLER_RTT_FACTOR),
            _ => RPC_TIMEOUT,
        };
        let due = |l: &Lane| if l.answers { l.at } else { l.sent + wait };
        let Some(i) = (0..lanes.len()).min_by_key(|i| (due(&lanes[*i]), lanes[*i].r)) else {
            break;
        };
        // A lane in flight past the slow threshold stops holding back the
        // next dispatch (`fanout_queue.rs:500-552`, `fanout_call.rs:105-110`).
        let slow = lanes
            .iter()
            .map(|l| l.sent + STRAGGLER_FLOOR)
            .filter(|t| *t > now && !fanout.queue.is_empty())
            .min();
        let deadline = last_accept + SET_TIMEOUT;
        let next = slow.map_or(due(&lanes[i]), |t| t.min(due(&lanes[i])));
        if next > deadline {
            tokio::time::sleep_until(deadline).await;
            break;
        }
        if next < due(&lanes[i]) {
            tokio::time::sleep_until(next).await;
            continue;
        }
        tokio::time::sleep_until(due(&lanes[i])).await;
        let mut lane = lanes.swap_remove(i);
        if !lane.answers {
            // Timed out or cut: it takes no slot.
            fanout.slot_freed();
            continue;
        }
        let mut guard = net.inner.lock();
        let inner = &mut *guard;
        if !arrives(inner, node, key, lane.r) {
            // Lost on the way: no answer comes.
            lane.answers = false;
            lanes.push(lane);
            continue;
        }
        let Some(record) = inner.records.get_mut(key) else {
            break;
        };
        let before = fanout.accepted.len();
        if fanout.arrive(
            schema,
            &mut record.replicas,
            lane.r,
            subkey as usize,
            &lane.value,
        ) {
            watches::mark(record, &[lane.r], subkey);
            stores::admit(inner, key, &[lane.r]);
        } else {
            stores::touch(inner, key, &[lane.r]);
        }
        slowest = slowest.max(Some(lane.at - lane.sent));
        if fanout.accepted.len() > before {
            last_accept = lane.at;
        }
    }
    let tail: Vec<Lane> = lanes.into_iter().filter(|l| l.answers).collect();
    if let (Some(schema), false) = (schema, tail.is_empty()) {
        tokio::spawn(land_tail(
            Arc::clone(net),
            node,
            (key.to_string(), subkey),
            schema,
            tail,
        ));
    }
    fanout.finish()
}

/// The latency stream of a set of `value` at `key`/`subkey`: its RPCs'
/// durations come from the seed and the write alone, never from what the
/// node's other calls drew before it. Tokio wakes tasks due at one instant
/// in an order the seed does not fix (`select!` polls its branches from an
/// unseeded generator), so one stream shared by every call of a node would
/// hand a set different durations, and its replicas' answers a different
/// order, from run to run on the same seed: which value a contested set
/// settles on is decided by that order (0.5.8 `set_value.rs:94-109`,
/// `value_tally.rs:37-45`).
fn set_stream(key: &str, subkey: u32, value: &rekindle_records::port::RecordValue) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"rekindle-netsim set stream");
    hasher.update(key.as_bytes());
    hasher.update(&subkey.to_le_bytes());
    hasher.update(&value.seq.to_le_bytes());
    hasher.update(value.writer.as_bytes());
    hasher.update(&value.data);
    let mut first = [0u8; 8];
    first.copy_from_slice(&hasher.finalize().as_bytes()[..8]);
    u64::from_le_bytes(first)
}

/// Whether an RPC from `node` arriving now at `key`'s replica `r` is
/// received: the node is online and still reaches it.
fn arrives(inner: &crate::network::Inner, node: usize, key: &str, r: usize) -> bool {
    let home = inner
        .records
        .get(key)
        .and_then(|record| record.replicas.get(r))
        .map(|x| x.home);
    watches::online(inner, node) && home.is_some_and(|h| reaches(&inner.faults.partitions, node, h))
}

/// The RPCs still in flight when a set returned land at their arrival,
/// their answers unread (0.5.8 `fanout_call.rs:466-471,624-626`).
async fn land_tail(
    net: Arc<Network>,
    node: usize,
    (key, subkey): (String, u32),
    schema: RecordSchema,
    mut tail: Vec<Lane>,
) {
    tail.sort_by_key(|l| (l.at, l.r));
    for lane in tail {
        tokio::time::sleep_until(lane.at).await;
        let mut guard = net.inner.lock();
        let inner = &mut *guard;
        if !arrives(inner, node, &key, lane.r) {
            continue;
        }
        let Some(record) = inner.records.get_mut(&key) else {
            return;
        };
        let answer = deliver(
            &schema,
            &mut record.replicas,
            lane.r,
            subkey as usize,
            &lane.value,
        );
        if matches!(answer, Some(Offer::Stored)) {
            watches::mark(record, &[lane.r], subkey);
            stores::admit(inner, &key, &[lane.r]);
        } else {
            stores::touch(inner, &key, &[lane.r]);
        }
    }
}

/// A landed fanout's value is stored in `node`'s local store, within the
/// record's total (`record_store/record.rs:105-129`); `Some` is that value
/// when it is not ours or some replica holds a rival at its seq
/// (`set_value.rs:753-760`). A set that did not land stores nothing and,
/// with the `AllowOffline(false)` every Rekindle set passes, answers
/// `TryAgain` (0.5.8 `set_value.rs:731-738` →
/// `local_record_store_interface.rs:60-62`), never `Ok(None)`.
pub(crate) fn commit(
    net: &Network,
    node: usize,
    (key, subkey): (&str, u32),
    ours: &Cipher,
    fanout: &SetFanout,
) -> Result<Option<rekindle_records::port::RecordValue>, DhtCallError> {
    if !fanout.landed() {
        return Err(DhtCallError {
            kind: DhtErrorKind::TryAgain,
            message: "offline, try again later".into(),
        });
    }
    let mut inner = net.inner.lock();
    let others: usize = inner.nodes[node]
        .local
        .iter()
        .filter(|((k, s), _)| k == key && *s != subkey)
        .map(|(_, v)| v.data.len())
        .sum();
    if others + fanout.value.value.data.len() > size::MAX_RECORD_DATA_SIZE {
        return Err(DhtCallError {
            kind: DhtErrorKind::Internal,
            message: format!("record exceeds maximum data size: {key}"),
        });
    }
    replicas::keep(&mut inner, node, key, subkey, &fanout.value);
    let settled = fanout.value.value.clone();
    Ok((fanout.conflicted || settled != ours.value).then_some(settled))
}
