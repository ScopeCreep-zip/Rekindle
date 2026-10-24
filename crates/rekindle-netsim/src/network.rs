//! The simulated network every node's ports share: the DHT's records, the
//! private-route table, each node's desired watches and local record
//! copies, the
//! queue of events in flight, the faults in force, and the trace.
//!
//! Events are never delivered on send. They queue here, and the pump
//! ([`crate::Netsim::start`]) delivers each batch in an order drawn from the
//! run seed, so a seed fixes the interleaving.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rekindle_record_pool::RecordPool;
use rekindle_records::port::{PortEvent, RecordSchema, RecordValue};
use tokio::sync::{mpsc, oneshot, watch, Notify};

use crate::busy::Busy;

use crate::latency::Latency;
pub(crate) use crate::records::Replica;
use crate::trace::Trace;

/// The per-node event queue's bound, as the Veilid one (`UPDATE_QUEUE`).
pub(crate) const EVENT_QUEUE: usize = 4096;
/// A node's inbound RPC queue: `rpc.queue_size` (`veilid_config.rs:849`).
pub(crate) const RPC_QUEUE: usize = 1024;

/// A record on the simulated DHT.
#[derive(Debug, Clone)]
pub(crate) struct NetRecord {
    pub schema: RecordSchema,
    /// The owner's public key.
    pub owner: [u8; 32],
    /// The creating node: only it sees the record until its first write
    /// lands (Veilid's create is local; the first write publishes).
    pub creator: usize,
    /// The record's replicas (plan E2.B S4; [`crate::records`]'s
    /// `replicas` module places them and cites Veilid).
    pub replicas: Vec<Replica>,
}

/// A live private route: the node that owns it.
#[derive(Debug, Clone)]
pub(crate) struct RouteEntry {
    pub node: usize,
}

/// An event in flight.
#[derive(Debug)]
struct Pending {
    from: usize,
    to: usize,
    /// Arrival order within `(from, to)`.
    rank: u64,
    label: &'static str,
    event: PortEvent,
    /// When it arrives (simulated time; an app message's latency).
    due: tokio::time::Instant,
    /// An app message or call: lost when its receiver is detached on
    /// arrival.
    droppable: bool,
}

/// Faults in force (plan C9.3).
#[derive(Debug, Default)]
pub(crate) struct Faults {
    /// Per node: how many more DHT calls answer "try again".
    pub try_again: HashMap<usize, u32>,
    /// Nodes whose DHT calls all fail, as with no connection.
    pub dht_offline: HashSet<usize>,
    /// (node, record) pairs whose sets land below consensus.
    pub write_fails: HashSet<(usize, String)>,
    /// Detached nodes: no watch change, app message or call reaches them,
    /// and they send none.
    pub detached: HashSet<usize>,
    /// Attached nodes whose public internet is not ready: their DHT calls
    /// answer "try again" and no watch is placed (Veilid's storage manager
    /// gates on `RoutingDomainReadyEvent`, `storage_manager/mod.rs:887-898`).
    pub not_ready: HashSet<usize>,
    /// Unordered node pairs that cannot reach each other: no app message
    /// or call passes, and neither reaches the replicas beside the other.
    pub partitions: HashSet<(usize, usize)>,
    /// Per mille of app messages and calls dropped.
    pub drop_per_mille: u32,
    /// Per mille of the app messages, calls and call answers each node
    /// sends that are dropped ([`crate::Fault::DropFrom`]); read only by
    /// [`Network::lost_in_transit`], never by the DHT model.
    pub drop_from: HashMap<usize, u32>,
    /// Nodes whose app calls are never answered (the caller times out).
    pub unanswered: HashSet<usize>,
    /// Nodes whose next send fails at its own route's first hop.
    pub link_failure: HashSet<usize>,
}

/// Each node's side of the network.
pub(crate) struct NodeSlot {
    pub events: mpsc::Sender<PortEvent>,
    pub ready: watch::Sender<bool>,
    /// The node's local record copies, across its sessions (Veilid's
    /// local store).
    pub local: HashMap<(String, u32), RecordValue>,
    /// The records in the node's local store (created or opened here):
    /// opening one again needs no network (`open_record.rs:27-44`).
    pub known: HashSet<String>,
    /// The node's open records and the writer keypair each is open with
    /// (node-wide, as Veilid's `opened_records`).
    pub opened: HashMap<String, Option<String>>,
    /// The node's desired watches, by record key; `None` is cancelled and
    /// dies at the next watch tick ([`crate::records::watches`]).
    pub watches: HashMap<String, Option<crate::records::watches::Desired>>,
    /// The node's queued change inspections: record key → not before (µs)
    /// (Veilid's `needs_change_inspection`, [`crate::records`]' catch-up).
    pub inspects: HashMap<String, u64>,
    /// The inspects the node's app asked for (`DhtPort::inspect`), by
    /// record key, for a scenario that a record is never polled.
    pub app_inspects: HashMap<String, u64>,
    /// Whether the node's DHT was online at the last watch tick.
    pub was_online: bool,
    /// When a tick lag last inspected every watch (µs).
    pub last_lag_inspect: Option<u64>,
    /// The node's live record sessions (the production pool).
    pub pools: Vec<Weak<RecordPool>>,
    /// The attachment state last queued for the node ([`crate::attach`]).
    pub state: &'static str,
    /// When the node's next attachment update may arrive: one after
    /// another, in order.
    pub attach_next: tokio::time::Instant,
    /// The node's remote route cache ([`crate::routes`]).
    pub imported: HashMap<String, crate::routes::CacheEntry>,
    /// The node's sets in flight, by record key and subkey.
    pub setting: HashMap<(String, u32), u32>,
    /// The node's clock skew (ms): what its Veilid's own clock reads.
    pub skew_ms: i64,
    /// The generation of Veilid's own value-changed route for the node's
    /// watches: a route death starts a new one ([`crate::records::watches`]).
    pub route_epoch: u64,
    /// Per own route: the destinations of questions lost on it.
    pub lost: HashMap<String, HashSet<String>>,
    /// The node's queued rehydrations, by record key (Veilid's
    /// `rehydration_requests`, [`crate::records`]' rehydration).
    pub rehydrations: BTreeSet<String>,
    /// The node's queued subkey pushes (Veilid's `offline_subkey_writes`,
    /// fed only by rehydration): the reach of the last attempt that left
    /// the subkey queued, `None` before any.
    pub pushes: BTreeMap<(String, u32), Option<Vec<usize>>>,
}

impl NodeSlot {
    pub(crate) fn new(events: mpsc::Sender<PortEvent>, ready: watch::Sender<bool>) -> Self {
        Self {
            events,
            ready,
            local: HashMap::new(),
            known: HashSet::new(),
            opened: HashMap::new(),
            watches: HashMap::new(),
            inspects: HashMap::new(),
            app_inspects: HashMap::new(),
            was_online: true,
            last_lag_inspect: None,
            pools: Vec::new(),
            state: "detached",
            attach_next: tokio::time::Instant::now(),
            imported: HashMap::new(),
            lost: HashMap::new(),
            setting: HashMap::new(),
            skew_ms: 0,
            route_epoch: 0,
            rehydrations: BTreeSet::new(),
            pushes: BTreeMap::new(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Inner {
    pub records: HashMap<String, NetRecord>,
    /// The owner keypair each record was created with, as its creator
    /// holds it (a scenario's thief of that key writes with it).
    pub owners: HashMap<String, String>,
    pub routes: HashMap<String, RouteEntry>,
    pub nodes: Vec<NodeSlot>,
    pending: Vec<Pending>,
    ranks: HashMap<(usize, usize), u64>,
    pub calls: HashMap<u64, oneshot::Sender<Vec<u8>>>,
    pub next_call: u64,
    pub faults: Faults,
    pub trace: Trace,
    /// When the next watch tick runs.
    pub next_tick: Option<tokio::time::Instant>,
    /// The DHT stores' use counter (each store's record LRU,
    /// [`crate::records`]' `stores`).
    pub lru: u64,
    /// Per node: inbound RPCs its full queue dropped.
    pub rpc_dropped: HashMap<usize, u64>,
}

/// The network, shared by every node's ports.
pub struct Network {
    pub(crate) inner: Mutex<Inner>,
    /// The pump's generator: delivery order and dropped messages.
    order: Mutex<StdRng>,
    /// What each call costs (plan E2.B), always on.
    pub(crate) latency: Latency,
    /// Simulated time zero (Veilid timestamps are counted from it).
    pub(crate) epoch: tokio::time::Instant,
    /// The Hosts' database calls in flight ([`crate::busy`]).
    pub(crate) busy: Arc<Busy>,
    /// Wakes the pump when an event is queued.
    pub(crate) wake: Notify,
}

impl Network {
    pub(crate) fn new(seed: u64) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner::default()),
            order: Mutex::new(StdRng::seed_from_u64(seed ^ 0x6e65_7473_696d)),
            latency: Latency::new(seed),
            epoch: tokio::time::Instant::now(),
            busy: Arc::default(),
            wake: Notify::new(),
        })
    }

    /// Queue `event` from `from` to `to`, unless a partition, a detached
    /// receiver or the drop fault takes it. `droppable`: an app message or call (DHT
    /// notifications and attachment changes are never dropped).
    pub(crate) fn send(
        &self,
        from: usize,
        to: usize,
        label: &'static str,
        event: PortEvent,
        droppable: bool,
    ) -> bool {
        self.send_after(from, to, label, event, droppable, Duration::ZERO)
    }

    /// As [`send`](Self::send), arriving `delay` from now.
    pub(crate) fn send_after(
        &self,
        from: usize,
        to: usize,
        label: &'static str,
        event: PortEvent,
        droppable: bool,
        delay: Duration,
    ) -> bool {
        let mut inner = self.inner.lock();
        if droppable && self.lost_in_transit(&inner, from, to) {
            return false;
        }
        let rank = {
            let next = inner.ranks.entry((from, to)).or_default();
            *next += 1;
            *next
        };
        inner.pending.push(Pending {
            from,
            to,
            rank,
            label,
            event,
            due: tokio::time::Instant::now() + delay,
            droppable: droppable && from != to,
        });
        drop(inner);
        self.wake.notify_one();
        true
    }

    /// Whether a message from `from` to `to` is lost on the way: a
    /// partition between them, `to` detached, or the draw of the drop fault
    /// (the larger of the network's and the sender's own,
    /// [`crate::Fault::DropFrom`]).
    pub(crate) fn lost_in_transit(&self, inner: &Inner, from: usize, to: usize) -> bool {
        if from == to {
            return false;
        }
        if inner.faults.partitions.contains(&pair(from, to)) || inner.faults.detached.contains(&to)
        {
            return true;
        }
        let per_mille = inner
            .faults
            .drop_per_mille
            .max(inner.faults.drop_from.get(&from).copied().unwrap_or(0));
        per_mille > 0 && self.order.lock().gen_range(0..1000) < per_mille
    }

    /// Deliver what is in flight, in an order drawn from the seed. The
    /// batch is first put in a canonical order (sender, receiver, arrival
    /// within the pair), so the draw does not depend on which task queued
    /// first. Returns how many were delivered.
    pub(crate) fn deliver(&self) -> usize {
        let now = tokio::time::Instant::now();
        let tick = {
            let mut inner = self.inner.lock();
            let due = *inner
                .next_tick
                .get_or_insert(now + crate::records::watches::WATCH_TICK);
            if now >= due {
                inner.next_tick = Some(now + crate::records::watches::WATCH_TICK);
            }
            now >= due
        };
        if tick {
            // The storage manager's tick: offline writes and rehydration run
            // before the watch tasks (`tasks/mod.rs:119-125`).
            self.tick_rehydration();
            self.tick_watches();
        }
        let (batch, senders) = {
            let mut inner = self.inner.lock();
            let now = tokio::time::Instant::now();
            let (mut batch, later): (Vec<_>, Vec<_>) = std::mem::take(&mut inner.pending)
                .into_iter()
                .partition(|p| p.due <= now);
            inner.pending = later;
            // A message or call in flight to a node that detached meanwhile
            // is lost: a detached Veilid node has no network to take it.
            batch.retain(|p| !(p.droppable && inner.faults.detached.contains(&p.to)));
            batch.sort_by_key(|p| (p.from, p.to, p.rank));
            // A node's inbound RPC queue holds `rpc.queue_size` 1024
            // (`veilid_config.rs:849`); past it the enqueue fails and the RPC
            // is dropped (`rpc_worker.rs:158-163`).
            let mut queued: HashMap<usize, usize> = HashMap::new();
            let dropped = &mut inner.rpc_dropped;
            batch.retain(|p| {
                if p.from == p.to || matches!(p.event, PortEvent::Attachment { .. }) {
                    return true;
                }
                let n = queued.entry(p.to).or_default();
                *n += 1;
                if *n > RPC_QUEUE {
                    *dropped.entry(p.to).or_default() += 1;
                    return false;
                }
                true
            });
            batch.shuffle(&mut *self.order.lock());
            for p in &batch {
                // A value change names its record (keys come from the seeded
                // node generators); wire bytes never enter the trace.
                let entry = match &p.event {
                    PortEvent::ValueChange(change) => {
                        format!("{}<-{} {} {}", p.to, p.from, p.label, change.key)
                    }
                    _ => format!("{}<-{} {}", p.to, p.from, p.label),
                };
                inner.trace.push(p.to, entry);
                // The ready watch follows the attachment update, as
                // production's transport sets it from each one
                // (`rekindle-transport` `SharedState::set_attachment`).
                if let PortEvent::Attachment {
                    public_internet_ready: pir,
                    ..
                } = &p.event
                {
                    if let Some(slot) = inner.nodes.get(p.to) {
                        slot.ready
                            .send_if_modified(|ready| std::mem::replace(ready, *pir) != *pir);
                    }
                }
            }
            let senders: Vec<_> = inner.nodes.iter().map(|n| n.events.clone()).collect();
            (batch, senders)
        };
        let count = batch.len();
        for p in batch {
            if let Some(tx) = senders.get(p.to) {
                if tx.try_send(p.event).is_err() {
                    tracing::warn!(node = p.to, "netsim: event queue full, event dropped");
                }
            }
        }
        count
    }

    /// When the next event in flight arrives, if any is.
    pub(crate) fn next_due(&self) -> Option<tokio::time::Instant> {
        let inner = self.inner.lock();
        let tick = inner.next_tick.filter(|_| Self::watch_work(&inner));
        inner.pending.iter().map(|p| p.due).chain(tick).min()
    }

    /// When the pump next has work without being woken: the next event in
    /// flight or the next storage tick, whichever is first.
    pub(crate) fn next_wake(&self) -> Option<tokio::time::Instant> {
        let inner = self.inner.lock();
        inner
            .pending
            .iter()
            .map(|p| p.due)
            .chain(inner.next_tick)
            .min()
    }

    /// The node's live record sessions.
    pub(crate) fn pools(&self, node: usize) -> Vec<Arc<RecordPool>> {
        let mut inner = self.inner.lock();
        let Some(slot) = inner.nodes.get_mut(node) else {
            return Vec::new();
        };
        slot.pools.retain(|p| p.strong_count() > 0);
        slot.pools.iter().filter_map(Weak::upgrade).collect()
    }
}

/// An unordered node pair.
pub(crate) fn pair(a: usize, b: usize) -> (usize, usize) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}
