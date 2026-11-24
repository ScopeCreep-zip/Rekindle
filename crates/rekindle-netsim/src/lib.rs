//! `rekindle-netsim` (plan C9.3): a seeded, deterministic simulation of the
//! Veilid network behind the record, route and message ports of
//! `rekindle_records::port`. Each simulated node gets a `PortSet`, its event
//! stream, a [`SimClock`], a [`SimRandom`] and a [`SimIdle`]; a real Host runs on them
//! (`TransportSource::Ports`), so the Host's adapters and engines run as in
//! production.
//!
//! Run it on a single-threaded, paused tokio runtime
//! (`#[tokio::test(flavor = "current_thread", start_paused = true)]`). The
//! pump ([`Netsim::start`]) lets every task run until it waits, then
//! delivers the events in flight in an order drawn from the seed, and
//! records each delivery in the [`Trace`]. Wire bytes never enter the trace:
//! they carry AEAD nonces from `OsRng` (plan C9.2).
//!
//! Veilid's latency is part of the model, always (plan E2.B, [`latency`]):
//! every DHT call and app message costs simulated time from a calibrated,
//! seeded distribution, with Veilid's per-subkey serialization and DHT
//! concurrency cap. The calibration table in [`latency`] names each
//! number's source (the pinned Veilid constant or the live log) and how to
//! recalibrate it from a live log.

mod attach;
mod busy;
mod clock;
mod faults;
mod flight;
mod idle;
pub mod latency;
mod local_store;
mod loopback;
mod messages;
mod network;
mod observe;
mod records;
mod restart;
mod routes;
mod sleep;
mod trace;
mod watch_census;

#[cfg(test)]
mod attach_tests;
#[cfg(test)]
mod member_record_tests;
#[cfg(test)]
mod message_tests;
#[cfg(test)]
mod remote_cache_tests;
#[cfg(test)]
mod replica_tests;
#[cfg(test)]
mod routes_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use rekindle_records::port::{PortEvent, PortSet, RecordValue};
use tokio::sync::mpsc;

pub use attach::ATTACH_STEP;
pub use busy::{db_call, db_queued, on_sim_thread, DbCall};
pub use clock::{SimClock, SimRandom, SIM_EPOCH_MS};
pub use faults::{Fault, Records, Writes};
pub use idle::SimIdle;
pub use local_store::LocalStore;
pub use messages::{call_timeout, FakeMessages, RPC_TIMEOUT};
pub use network::Network;
pub use records::FakeRecords;
pub use routes::{
    allocation_entry, FakeAllocator, FakeRoutes, ALLOCATION_CUT, ALLOCATION_ENDS,
    ALLOCATION_STARTS, FORGETS_PEER_ROUTE, RELEASES_PEER_ROUTE,
};
pub use trace::Trace;

use network::{pair, NodeSlot, EVENT_QUEUE};
use rekindle_route::ordered_watch::{Follow, OrderedWatch};
use rekindle_route::own_routes::OwnRoutes;

/// A simulated node's side: what a Host on it needs.
pub struct Node {
    pub index: usize,
    pub ports: PortSet,
    pub events: mpsc::Receiver<PortEvent>,
    pub clock: Arc<SimClock>,
    pub rng: Arc<SimRandom>,
    pub idle: Arc<SimIdle>,
}

/// The simulation: its network, seed, and nodes' clocks and idle ports.
pub struct Netsim {
    net: Arc<Network>,
    seed: u64,
    start: tokio::time::Instant,
    clocks: parking_lot::Mutex<Vec<Arc<SimClock>>>,
    idles: parking_lot::Mutex<Vec<Arc<SimIdle>>>,
    /// Each node's generator, which a restarted node keeps drawing from.
    rngs: parking_lot::Mutex<Vec<Arc<SimRandom>>>,
}

impl Netsim {
    /// An empty network for run `seed`. Call on the paused runtime's
    /// thread: the nodes' clocks start now, and the database calls queued
    /// from this thread count as the run's work ([`busy`]).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let net = Network::new(seed);
        busy::install(&net.busy);
        Self {
            net,
            seed,
            start: tokio::time::Instant::now(),
            clocks: parking_lot::Mutex::new(Vec::new()),
            idles: parking_lot::Mutex::new(Vec::new()),
            rngs: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// `n` nodes on a new network.
    #[must_use]
    pub fn nodes(n: usize, seed: u64) -> (Self, Vec<Node>) {
        let sim = Self::new(seed);
        let nodes = (0..n).map(|_| sim.add_node()).collect();
        (sim, nodes)
    }

    /// Add a node; it is not ready until [`start`](Self::start).
    #[must_use]
    pub fn add_node(&self) -> Node {
        let (events_tx, events) = mpsc::channel(EVENT_QUEUE);
        let ready = Arc::new(OrderedWatch::new(false));
        let ready_rx = ready.follower();
        let index = {
            let mut inner = self.net.inner.lock();
            inner.nodes.push(NodeSlot::new(events_tx, ready));
            inner.nodes.len() - 1
        };
        let rng = Arc::new(SimRandom::new(self.seed, index));
        self.rngs.lock().push(Arc::clone(&rng));
        let clock = Arc::new(SimClock::new(self.start));
        self.clocks.lock().push(Arc::clone(&clock));
        let idle = Arc::new(SimIdle::new(Arc::clone(&clock)));
        self.idles.lock().push(Arc::clone(&idle));
        let ports = self.ports(index, &rng, ready_rx, &clock);
        Node {
            index,
            ports,
            events,
            clock,
            rng,
            idle,
        }
    }

    /// Node `index`'s ports over the network, its route set following
    /// `ready`.
    fn ports(
        &self,
        index: usize,
        rng: &Arc<SimRandom>,
        ready: Follow<bool>,
        clock: &Arc<SimClock>,
    ) -> PortSet {
        let wall_clock = Arc::clone(clock);
        let own = OwnRoutes::new(
            FakeAllocator::new(index, Arc::clone(&self.net)),
            ready,
            Arc::new(move || rekindle_utils::Clock::now_ms(wall_clock.as_ref())),
        );
        PortSet {
            records: Arc::new(FakeRecords::new(
                index,
                Arc::clone(&self.net),
                Arc::clone(rng),
            )),
            routes: Arc::new(FakeRoutes::new(index, Arc::clone(&self.net), own)),
            messages: Arc::new(FakeMessages::new(index, Arc::clone(&self.net))),
            lifecycle: None,
        }
    }

    /// The user of `node` stops (`idle`, from now on the simulated clock)
    /// or resumes giving input; the node's Host reads it on its next idle
    /// tick.
    pub fn set_idle(&self, node: usize, idle: bool) {
        if let Some(port) = self.idles.lock().get(node) {
            if idle {
                port.set_idle();
            } else {
                port.set_active();
            }
        }
    }

    /// Attach every node (fully, public internet ready) and spawn the pump.
    /// The pump runs until the runtime ends.
    pub fn start(&self) -> tokio::task::JoinHandle<()> {
        let count = self.net.inner.lock().nodes.len();
        for node in 0..count {
            self.set_ready(node, true);
        }
        let net = Arc::clone(&self.net);
        tokio::spawn(async move {
            loop {
                settle(&net).await;
                if net.deliver() == 0 {
                    // Nothing in flight is due: wait for a send, the next
                    // arrival or the storage tick. Time moves only while
                    // every task waits, database calls included
                    // ([`busy`]), so each jump lands where the seed puts it.
                    let woken = net.wake.notified();
                    match net.next_wake() {
                        Some(due) => drop(tokio::time::timeout_at(due, woken).await),
                        None => woken.await,
                    }
                }
            }
        })
    }

    /// Let every task run until it waits, then deliver what is in flight;
    /// again until nothing is, waiting (in simulated time) for what is
    /// still on its way. For a scenario's explicit steps.
    pub async fn quiesce(&self) {
        loop {
            settle(&self.net).await;
            if self.net.deliver() == 0 {
                settle(&self.net).await;
                if self.net.deliver() == 0 {
                    match self.net.next_due() {
                        Some(due) => tokio::time::sleep_until(due).await,
                        None => return,
                    }
                }
            }
        }
    }

    /// A modified client's write: any writer the schema admits, bypassing
    /// the app (plan C9.3). It reaches every replica in the window and lands
    /// on each at the newest seq any of them holds + 1.
    ///
    /// # Errors
    /// The record or subkey is unknown, or the schema does not admit the
    /// writer.
    pub fn forge_write(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: &str,
    ) -> Result<(), rekindle_records::port::PortError> {
        self.write_outside(key, subkey, data, writer, true)
    }

    /// A same-seq conflict (plan C9.3): a racing writer's value at the
    /// newest seq the replicas hold, stored on every window replica that
    /// holds nothing at that seq yet (the replicas the first writer's fanout did
    /// not reach). A replica keeps the first value it got at a seq
    /// (Veilid 0.5.8 `set_value.rs:832-843`), and so does a node's local copy.
    ///
    /// # Errors
    /// As [`forge_write`](Self::forge_write); also when the subkey holds
    /// nothing yet, or every replica already holds that seq.
    pub fn conflict_write(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: &str,
    ) -> Result<(), rekindle_records::port::PortError> {
        self.write_outside(key, subkey, data, writer, false)
    }

    fn write_outside(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: &str,
        bump: bool,
    ) -> Result<(), rekindle_records::port::PortError> {
        use rekindle_records::port::PortError;
        let public = records::writer_public(writer)?;
        {
            let mut inner = self.net.inner.lock();
            let record = inner
                .records
                .get_mut(key)
                .ok_or_else(|| PortError::Unreachable(key.into()))?;
            if !records::admits(record, subkey, &public) {
                return Err(PortError::NotWritable(key.into()));
            }
            let at = subkey as usize;
            if at >= record.replicas.first().map_or(0, |r| r.values.len()) {
                return Err(PortError::Dht(format!("subkey {subkey} out of range")));
            }
            let newest = record
                .replicas
                .iter()
                .filter_map(|r| r.values[at].as_ref().map(|v| v.seq))
                .max();
            let seq = match (newest, bump) {
                (Some(seq), true) => seq.saturating_add(1),
                (None, true) => 0,
                (Some(seq), false) => seq,
                (None, false) => {
                    return Err(PortError::Dht("no value to conflict with".into()));
                }
            };
            let value = RecordValue {
                seq,
                data,
                writer: records::public_text(&public),
            };
            // Each replica checks the size rule (Veilid 0.5.8 `set_value.rs:880-891`).
            if !records::size::fits(&record.schema, &value) {
                return Err(PortError::Dht("value too big".into()));
            }
            let mut stored = Vec::new();
            // A set reaches the window, never the reserve past it
            // ([`records::REPLICAS`], 0.5.8 `fanout_call.rs:395-441`).
            for (r, replica) in record
                .replicas
                .iter_mut()
                .enumerate()
                .take(records::REPLICAS)
            {
                if !records::size::within_total(&replica.values, at, &value) {
                    continue;
                }
                if replica.values[at].as_ref().is_none_or(|v| v.seq < seq) {
                    replica.values[at] = Some(value.clone());
                    stored.push(r);
                }
            }
            if stored.is_empty() {
                return Err(PortError::Dht("every replica holds that seq".into()));
            }
            records::watches::mark(record, &stored, subkey);
        }
        Ok(())
    }

    /// The owner keypair `key` was created with, as its creator holds it.
    /// A record's owner key is not its creator's identity key, so whoever
    /// holds it (a thief, or the creator's modified client) can rewrite
    /// the record; a scenario writes with it through
    /// [`forge_write`](Self::forge_write) to prove that what the record
    /// carries is checked under the identity (plan E2.4's prekey record).
    #[must_use]
    pub fn owner_keypair(&self, key: &str) -> Option<String> {
        self.net.inner.lock().owners.get(key).cloned()
    }

    /// Whether `node` holds `key` open in its local store (Veilid's
    /// `opened_records`): a lease the app released, or a pool that ended,
    /// has closed it.
    #[must_use]
    pub fn is_open(&self, node: usize, key: &str) -> bool {
        self.net.inner.lock().nodes[node].opened.contains_key(key)
    }

    /// The node that created `key`: whose record it is (a chain's writer).
    #[must_use]
    pub fn creator(&self, key: &str) -> Option<usize> {
        self.net.inner.lock().records.get(key).map(|r| r.creator)
    }

    /// A kill -9 on `node`: its record pools discard the writes they hold
    /// for a re-push, which live only in memory (a graceful lock saves
    /// them for the next unlock). Nothing re-pushes them.
    pub fn lose_held(&self, node: usize) {
        for pool in self.net.pools(node) {
            drop(pool.take_held());
        }
    }

    /// A stray app message reaches `to` from `from` (any node may send any
    /// bytes to a route it holds): what the receiver makes of them.
    pub fn stray_message(&self, from: usize, to: usize, message: Vec<u8>) {
        self.net.send(
            from,
            to,
            "AppMessage",
            PortEvent::AppMessage {
                message,
                route: None,
            },
            true,
        );
    }

    /// A stray app call reaches `to` from `from` (any node may call any
    /// route it holds), no one waiting on its answer: the reply is ignored, as
    /// an expired call's is ([`FakeMessages::reply`](crate::messages)). Plan RV1.
    pub fn stray_call(&self, from: usize, to: usize, message: Vec<u8>) {
        let call_id = self.net.next_call_id();
        self.net.send(
            from,
            to,
            "AppCall",
            PortEvent::AppCall {
                call_id: rekindle_records::port::AppCallId(call_id),
                message,
                route: None,
            },
            true,
        );
    }

    /// Record a semantic event `node` saw (a scenario's or a Host's).
    pub fn mark(&self, node: usize, event: impl Into<String>) {
        self.net.inner.lock().trace.push(node, event.into());
    }

    /// The trace so far.
    #[must_use]
    pub fn trace(&self) -> Trace {
        self.net.inner.lock().trace.clone()
    }
}

pub(crate) fn toggle(set: &mut std::collections::HashSet<usize>, node: usize, on: bool) {
    if on {
        set.insert(node);
    } else {
        set.remove(&node);
    }
}

/// Let every runnable task run until it waits, its database calls
/// included: again while a settle began a call.
async fn settle(net: &Network) {
    loop {
        let begun = net.busy.begun();
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        if net.busy.begun() == begun {
            return;
        }
    }
}
