//! The simulated network every node's ports share: the DHT's records, the
//! private-route table, each node's watches and local record copies, the
//! queue of events in flight, the faults in force, and the trace.
//!
//! Events are never delivered on send. They queue here, and the pump
//! ([`crate::Netsim::start`]) delivers each batch in an order drawn from the
//! run seed, so a seed fixes the interleaving.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rekindle_record_pool::RecordPool;
use rekindle_records::port::{PortEvent, RecordSchema, RecordValue, ValueChange};
use tokio::sync::{mpsc, oneshot, watch};

use crate::latency::Latency;
use crate::trace::Trace;

/// The per-node event queue's bound, as the Veilid one (`UPDATE_QUEUE`).
pub(crate) const EVENT_QUEUE: usize = 4096;

/// A record on the simulated DHT.
#[derive(Debug, Clone)]
pub(crate) struct NetRecord {
    pub schema: RecordSchema,
    /// The owner's public key.
    pub owner: [u8; 32],
    /// The creating node: only it sees the record until its first write
    /// lands (Veilid's create is local; the first write publishes).
    pub creator: usize,
    pub published: bool,
    pub values: Vec<Option<RecordValue>>,
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
}

/// Faults in force (plan C9.3).
#[derive(Debug, Default)]
pub(crate) struct Faults {
    /// Per node: how many more DHT calls answer "try again".
    pub try_again: HashMap<usize, u32>,
    /// Nodes whose DHT calls all fail, as with no connection.
    pub dht_offline: HashSet<usize>,
    /// Detached nodes: no watch change reaches them.
    pub detached: HashSet<usize>,
    /// Nodes whose reads return their local copy without asking the
    /// network (a stale value).
    pub stale_reads: HashSet<usize>,
    /// Unordered node pairs that cannot reach each other.
    pub partitions: HashSet<(usize, usize)>,
    /// Per mille of app messages and calls dropped.
    pub drop_per_mille: u32,
    /// Nodes whose app calls are never answered (the caller times out).
    pub unanswered: HashSet<usize>,
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
    /// What the node watches, by record key.
    pub watches: HashMap<String, BTreeSet<u32>>,
    /// The node's live record sessions (the production pool).
    pub pools: Vec<Weak<RecordPool>>,
    /// Route ids this node imported.
    pub imported: HashSet<String>,
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
}

/// The network, shared by every node's ports.
pub struct Network {
    pub(crate) inner: Mutex<Inner>,
    /// The pump's generator: delivery order and dropped messages.
    order: Mutex<StdRng>,
    /// What each call costs (plan E2.B), always on.
    pub(crate) latency: Latency,
}

impl Network {
    pub(crate) fn new(seed: u64) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner::default()),
            order: Mutex::new(StdRng::seed_from_u64(seed ^ 0x6e65_7473_696d)),
            latency: Latency::new(seed),
        })
    }

    /// Queue `event` from `from` to `to`, unless a partition or the drop
    /// fault takes it. `droppable`: an app message or call (DHT
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
        if droppable && from != to {
            if inner.faults.partitions.contains(&pair(from, to)) {
                return false;
            }
            let per_mille = inner.faults.drop_per_mille;
            if per_mille > 0 && self.order.lock().gen_range(0..1000) < per_mille {
                return false;
            }
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
        });
        true
    }

    /// Deliver what is in flight, in an order drawn from the seed. The
    /// batch is first put in a canonical order (sender, receiver, arrival
    /// within the pair), so the draw does not depend on which task queued
    /// first. Returns how many were delivered.
    pub(crate) fn deliver(&self) -> usize {
        let (batch, senders) = {
            let mut inner = self.inner.lock();
            let now = tokio::time::Instant::now();
            let (mut batch, later): (Vec<_>, Vec<_>) = std::mem::take(&mut inner.pending)
                .into_iter()
                .partition(|p| p.due <= now);
            inner.pending = later;
            batch.sort_by_key(|p| (p.from, p.to, p.rank));
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
        self.inner.lock().pending.iter().map(|p| p.due).min()
    }

    /// Notify every node but `writer` that watches `subkey` of `key`. Each
    /// watcher's storage manager stores the value it is sent when it is
    /// newer than its local copy (the inbound `ValueChanged`).
    pub(crate) fn notify_watchers(
        &self,
        writer: usize,
        key: &str,
        subkey: u32,
        value: &RecordValue,
    ) {
        let watchers: Vec<usize> = {
            let mut inner = self.inner.lock();
            let watchers: Vec<usize> = inner
                .nodes
                .iter()
                .enumerate()
                .filter(|(i, node)| {
                    *i != writer
                        && !inner.faults.detached.contains(i)
                        && node
                            .watches
                            .get(key)
                            .is_some_and(|subkeys| subkeys.contains(&subkey))
                })
                .map(|(i, _)| i)
                .collect();
            for to in &watchers {
                let local = &mut inner.nodes[*to].local;
                let entry = local
                    .entry((key.to_string(), subkey))
                    .or_insert_with(|| value.clone());
                if value.seq > entry.seq {
                    *entry = value.clone();
                }
            }
            watchers
        };
        for to in watchers {
            self.send(
                writer,
                to,
                "ValueChange",
                PortEvent::ValueChange(ValueChange {
                    key: key.to_string(),
                    subkeys: vec![subkey],
                    count: u32::MAX,
                    value: Some(value.clone()),
                }),
                false,
            );
        }
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
