//! A set's fanout over a record's replicas (plan E2.B slices S4 and S11,
//! MED-1 and MED-2), after the pinned veilid-core (`~/veilid/veilid-core/src`):
//!
//! - **lanes:** at most [`LANES`] set RPCs are in flight at once
//!   (`set_value_fanout`, `veilid_config.rs:915`; `fanout_call.rs:630-640`
//!   spins up one task per lane), sent in placement order over the replicas
//!   the node reaches; each arrives at its replica a drawn
//!   [`latency::SET_RPC`] after it left, and **the replica stores it then**
//!   (readers and watchers see it from that moment, not when the call
//!   returns). A replica the node no longer reaches at arrival, or an RPC
//!   arriving while the node is offline, is lost;
//! - **a replica's answer** (`storage_manager/set_value.rs:700-719`): it keeps
//!   the first value it receives at a seq, answers the same value as a
//!   success and anything older or same-seq different with what it holds;
//!   a value past the size rule is an invalid message ([`super::size`]);
//! - **the fanout's side** (`set_value.rs:465-500`): an answer older than
//!   the fanout's value is stale; a newer one, or a same-seq different one,
//!   restarts the fanout on it ("accept all conflicts in an attempt to
//!   resolve them"): every accepted replica is queued again
//!   (`fanout_call.rs:438-442`, `fanout_queue.rs:491-498`) and the answering
//!   one counts as accepted. An accept of a value the fanout has since left
//!   queues that replica again. Concurrent fanouts at one seq therefore
//!   interleave replica by replica, in arrival order, and can split the
//!   replicas with neither side at consensus;
//! - **done** at [`SET_COUNT`] accepts (`set_value_count`,
//!   `veilid_config.rs:914`); RPCs still in flight then land on their
//!   replicas, their answers unread. Before consensus Veilid's deadline stays
//!   a full timeout past the last accept (`fanout_call.rs:423-446,642-656`),
//!   so a split keeps restarting; the model's bound is the call's drawn span
//!   ([`latency::SET`], the measured set), and the call returns at its end.
//!   A fanout nobody waits on (a rehydration push) runs in one step, bounded
//!   by [`max_rpcs`], the RPCs that span holds.

use std::collections::VecDeque;
use std::time::Duration;

use rekindle_records::port::{DhtCallError, DhtErrorKind, RecordSchema, RecordValue};
use tokio::time::Instant;

use super::replicas::{Replica, SET_COUNT};
use super::{reachable, reaches, size, stores, watches};
use crate::latency;
use crate::network::Network;

/// Set RPCs in flight at once: `set_value_fanout` (`veilid_config.rs:915`).
pub(crate) const LANES: usize = 6;

/// The RPCs one set's span holds: [`LANES`] × [`latency::SET`]'s max over
/// [`latency::SET_RPC`]'s min.
pub(crate) fn max_rpcs() -> usize {
    let rounds = latency::SET.max.as_millis() / latency::SET_RPC.min.as_millis();
    LANES.saturating_mul(usize::try_from(rounds).unwrap_or(usize::MAX))
}

/// What a set's fanout did.
pub(crate) struct SetFanout {
    /// The value the fanout ended on: the writer's own, or one a replica
    /// restarted it on.
    pub value: RecordValue,
    /// Whether [`SET_COUNT`] replicas accepted `value`.
    pub consensus: bool,
    /// How many replicas accepted `value`.
    pub accepted: usize,
    /// The replicas that stored a value in this fanout, in order (one may
    /// appear twice: a restart sent it another value).
    pub stored: Vec<usize>,
}

/// A replica's answer to a set (`set_value.rs:700-719`).
enum Offer {
    Stored,
    Same,
    Ignored(RecordValue),
}

fn offer(slot: &mut Option<RecordValue>, value: &RecordValue) -> Offer {
    match slot {
        Some(held) if held == value => Offer::Same,
        Some(held) if held.seq >= value.seq => Offer::Ignored(held.clone()),
        _ => {
            *slot = Some(value.clone());
            Offer::Stored
        }
    }
}

/// One fanout's state (see the module notes).
pub(crate) struct Fanout {
    ctx: RecordValue,
    queue: VecDeque<usize>,
    accepted: Vec<usize>,
    stored: Vec<usize>,
    sent: usize,
}

impl Fanout {
    pub(crate) fn new(reach: &[usize], ours: &RecordValue) -> Self {
        Self {
            ctx: ours.clone(),
            queue: reach.iter().copied().collect(),
            accepted: Vec::new(),
            stored: Vec::new(),
            sent: 0,
        }
    }

    fn done(&self) -> bool {
        self.accepted.len() >= SET_COUNT
    }

    /// The next replica to send to and the value it carries.
    pub(crate) fn next(&mut self) -> Option<(usize, RecordValue)> {
        if self.done() {
            return None;
        }
        let r = self.queue.pop_front()?;
        self.sent += 1;
        Some((r, self.ctx.clone()))
    }

    fn accept(&mut self, r: usize, sent: &RecordValue) {
        if *sent == self.ctx {
            self.accepted.push(r);
        } else {
            self.queue.push_back(r);
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
        sent: &RecordValue,
    ) -> bool {
        let Some(slot) = replicas
            .get_mut(r)
            .filter(|x| size::admits(schema, &x.values, subkey, sent))
            .and_then(|x| x.values.get_mut(subkey))
        else {
            return false;
        };
        let answer = offer(slot, sent);
        let stored = matches!(answer, Offer::Stored);
        if stored {
            self.stored.push(r);
        }
        if self.done() {
            return stored;
        }
        match answer {
            Offer::Stored | Offer::Same => self.accept(r, sent),
            Offer::Ignored(held) if held == self.ctx => self.accepted.push(r),
            Offer::Ignored(held) if held.seq >= self.ctx.seq => {
                self.ctx = held;
                for a in self.accepted.drain(..).rev() {
                    self.queue.push_front(a);
                }
                self.accepted.push(r);
            }
            Offer::Ignored(_) => {}
        }
        stored
    }

    pub(crate) fn finish(self) -> SetFanout {
        SetFanout {
            consensus: self.done(),
            accepted: self.accepted.len(),
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
    ours: &RecordValue,
) -> SetFanout {
    let mut fanout = Fanout::new(reach, ours);
    let bound = max_rpcs();
    // Lane by lane in waves: a wave's RPCs all land, those after consensus
    // too, their answers unread.
    while fanout.sent < bound {
        let wave: Vec<(usize, RecordValue)> = (0..LANES).map_while(|_| fanout.next()).collect();
        if wave.is_empty() {
            break;
        }
        for (r, value) in wave {
            fanout.arrive(schema, replicas, r, subkey, &value);
        }
    }
    fanout.finish()
}

/// `node`'s set of `ours` at `key`/`subkey`, over `span` from now: each RPC
/// lands as it arrives; returns at the span's end.
pub(crate) async fn run(
    net: &Network,
    node: usize,
    (key, subkey): (&str, u32),
    ours: &RecordValue,
    span: Duration,
) -> SetFanout {
    let end = Instant::now() + span;
    let (schema, reach) = {
        let inner = net.inner.lock();
        inner.records.get(key).map_or_else(
            || (None, Vec::new()),
            |r| {
                let reach = reachable(&r.replicas, &inner.faults.partitions, node);
                (Some(r.schema.clone()), reach)
            },
        )
    };
    let mut fanout = Fanout::new(&reach, ours);
    let mut flight: Vec<(Instant, usize, RecordValue)> = Vec::new();
    while let Some(schema) = &schema {
        if watches::online(&net.inner.lock(), node) {
            while flight.len() < LANES {
                let Some((r, value)) = fanout.next() else {
                    break;
                };
                let at = Instant::now() + net.latency.draw(node, latency::SET_RPC);
                flight.push((at, r, value));
            }
        }
        let Some(i) = (0..flight.len()).min_by_key(|i| (flight[*i].0, flight[*i].1)) else {
            break;
        };
        if flight[i].0 > end {
            break;
        }
        let (at, r, value) = flight.swap_remove(i);
        tokio::time::sleep_until(at).await;
        let mut guard = net.inner.lock();
        let inner = &mut *guard;
        if !watches::online(inner, node) {
            continue;
        }
        let Some(record) = inner.records.get_mut(key) else {
            break;
        };
        let home = record.replicas.get(r).map(|x| x.home);
        if !home.is_some_and(|h| reaches(&inner.faults.partitions, node, h)) {
            continue;
        }
        if fanout.arrive(schema, &mut record.replicas, r, subkey as usize, &value) {
            watches::mark(record, &[r], subkey, node);
            stores::admit(inner, key, &[r]);
        } else {
            stores::touch(inner, key, &[r]);
        }
    }
    tokio::time::sleep_until(end).await;
    fanout.finish()
}

/// At consensus the fanout's value is stored in `node`'s local store,
/// within the record's total (`record_store/record.rs:105-129`); `Some` is
/// that value when it is not ours. Below consensus nothing is stored
/// (`local_record_store_interface.rs:61-62`).
pub(crate) fn commit(
    net: &Network,
    node: usize,
    (key, subkey): (&str, u32),
    ours: &RecordValue,
    fanout: &SetFanout,
) -> Result<Option<RecordValue>, DhtCallError> {
    if !fanout.consensus {
        return Ok(None);
    }
    let mut inner = net.inner.lock();
    let local = &mut inner.nodes[node].local;
    let others: usize = local
        .iter()
        .filter(|((k, s), _)| k == key && *s != subkey)
        .map(|(_, v)| v.data.len())
        .sum();
    if others + fanout.value.data.len() > size::MAX_RECORD_DATA_SIZE {
        return Err(DhtCallError {
            kind: DhtErrorKind::Internal,
            message: format!("record exceeds maximum data size: {key}"),
        });
    }
    local.insert((key.to_string(), subkey), fanout.value.clone());
    Ok(Some(fanout.value.clone()).filter(|value| value != ours))
}
