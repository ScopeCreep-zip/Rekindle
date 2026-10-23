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
mod idle;
pub mod latency;
mod messages;
mod network;
mod records;
mod routes;
mod trace;

#[cfg(test)]
mod attach_tests;
#[cfg(test)]
mod member_record_tests;
#[cfg(test)]
mod replica_tests;
#[cfg(test)]
mod routes_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use rekindle_records::port::{PortEvent, PortSet, RecordValue};
use tokio::sync::{mpsc, watch};

pub use attach::ATTACH_STEP;
pub use busy::{db_call, db_queued, on_sim_thread, DbCall};
pub use clock::{SimClock, SimRandom, SIM_EPOCH_MS};
pub use idle::SimIdle;
pub use messages::{call_timeout, FakeMessages, RPC_TIMEOUT};
pub use network::Network;
pub use records::FakeRecords;
pub use routes::{FakeAllocator, FakeRoutes};
pub use trace::Trace;

use network::{pair, NodeSlot, EVENT_QUEUE};

/// A fault to put in force (plan C9.3).
#[derive(Debug, Clone)]
pub enum Fault {
    /// The node's next `calls` DHT calls answer "try again".
    TryAgain { node: usize, calls: u32 },
    /// Every DHT call of the node fails (or stops failing): no connection.
    DhtOffline { node: usize, offline: bool },
    /// The two groups cannot exchange app messages or calls, and neither
    /// reaches the record replicas placed beside the other's nodes (a
    /// stale read or a set below consensus follows from which replicas a
    /// side reaches; `records` notes the placement).
    Partition { a: Vec<usize>, b: Vec<usize> },
    /// Every partition and offline DHT ends, and every attached node whose
    /// public internet was not ready is ready again; the record pools' own
    /// timers re-push their held writes.
    Heal,
    /// This fraction (per mille) of app messages and calls is dropped.
    DropMessages { per_mille: u32 },
    /// The node never answers app calls; its callers time out.
    Unanswered { node: usize, unanswered: bool },
    /// The node's clock runs this far from simulated true time.
    ClockSkew { node: usize, ms: i64 },
    /// The node's watch on `key` dies: Veilid sends a last change with
    /// empty subkeys, `count == 0` and no value (`watch_value.rs:697-703`).
    WatchDeath { node: usize, key: String },
    /// The node's own routes die: the owner hears of it, its importers do
    /// not (their sends are lost until they fetch the new route).
    RouteDeath { node: usize },
    /// The first hop of the node's own route fails on its next app message
    /// or call: the send fails `NoConnection` (our port's `RouteUnusable`),
    /// one send failure kills that route (`route_spec_store/mod.rs:48`,
    /// `route_stats.rs:56-60`), and the owner hears `dead_routes`. Its
    /// next route starts at another hop, so the fault ends with the route.
    RouteLinkFailure { node: usize },
    /// The node detaches (or reattaches): its DHT calls fail, it neither
    /// sends nor receives app messages or calls (a call to it times out),
    /// no watch change reaches it, and it hears Detaching then Detached
    /// (or Attaching up to AttachedGood); its ready watch follows
    /// ([`attach`]).
    Detach { node: usize, detached: bool },
    /// The node's public internet turns unready (or ready again) while it
    /// stays attached, as on a `RoutingDomainReadyEvent`: its DHT calls
    /// answer "try again", its ready watch follows, and it hears an
    /// attachment update with `is_attached` still true.
    InternetReady { node: usize, ready: bool },
    /// The node's storage-manager tick ran `ms` late (a suspended
    /// process): past 2 s, an online node inspects every watch, at most
    /// once per 30 s (`tasks/mod.rs:132-145`).
    TickLag { node: usize, ms: u64 },
    /// Nodes join closer to `key`: `count` of its replicas are pushed out
    /// for empty stores, their watches gone (`records/outside.rs`).
    ReplicaChurn { key: String, count: usize },
}

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
        let (ready, ready_rx) = watch::channel(false);
        let index = {
            let mut inner = self.net.inner.lock();
            inner.nodes.push(NodeSlot::new(events_tx, ready));
            inner.nodes.len() - 1
        };
        let rng = Arc::new(SimRandom::new(self.seed, index));
        let clock = Arc::new(SimClock::new(self.start));
        self.clocks.lock().push(Arc::clone(&clock));
        let idle = Arc::new(SimIdle::new(Arc::clone(&clock)));
        self.idles.lock().push(Arc::clone(&idle));
        let ports = PortSet {
            records: Arc::new(FakeRecords::new(
                index,
                Arc::clone(&self.net),
                Arc::clone(&rng),
            )),
            routes: Arc::new(FakeRoutes::new(index, Arc::clone(&self.net), ready_rx)),
            messages: Arc::new(FakeMessages::new(index, Arc::clone(&self.net))),
            lifecycle: None,
        };
        Node {
            index,
            ports,
            events,
            clock,
            rng,
            idle,
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

    /// Put `fault` in force.
    pub fn fault(&self, fault: Fault) {
        match fault {
            Fault::TryAgain { node, calls } => {
                self.net.inner.lock().faults.try_again.insert(node, calls);
            }
            Fault::DhtOffline { node, offline } => {
                toggle(&mut self.net.inner.lock().faults.dht_offline, node, offline);
            }
            Fault::Partition { a, b } => {
                let mut inner = self.net.inner.lock();
                for x in &a {
                    for y in &b {
                        inner.faults.partitions.insert(pair(*x, *y));
                    }
                }
            }
            Fault::Heal => self.heal(),
            Fault::DropMessages { per_mille } => {
                self.net.inner.lock().faults.drop_per_mille = per_mille.min(1000);
            }
            Fault::Unanswered { node, unanswered } => {
                toggle(
                    &mut self.net.inner.lock().faults.unanswered,
                    node,
                    unanswered,
                );
            }
            Fault::ClockSkew { node, ms } => {
                if let Some(clock) = self.clocks.lock().get(node) {
                    clock.set_skew_ms(ms);
                }
                if let Some(slot) = self.net.inner.lock().nodes.get_mut(node) {
                    slot.skew_ms = ms;
                }
            }
            Fault::WatchDeath { node, key } => self.net.kill_watch(node, &key),
            Fault::RouteDeath { node } => self.kill_routes(node),
            Fault::RouteLinkFailure { node } => {
                self.net.inner.lock().faults.link_failure.insert(node);
            }
            Fault::Detach { node, detached } => self.detach(node, detached),
            Fault::InternetReady { node, ready } => self.internet_ready(node, ready),
            Fault::ReplicaChurn { key, count } => self.churn(&key, count),
            Fault::TickLag { node, ms } => self
                .net
                .tick_lag(node, std::time::Duration::from_millis(ms)),
        }
    }

    fn heal(&self) {
        let unready: Vec<usize> = {
            let mut inner = self.net.inner.lock();
            inner.faults.partitions.clear();
            inner.faults.dht_offline.clear();
            let mut nodes: Vec<usize> = inner.faults.not_ready.iter().copied().collect();
            nodes.sort_unstable();
            nodes
        };
        for node in unready {
            self.internet_ready(node, true);
        }
    }

    /// The node's routes die: it hears `dead_routes`; no importer hears
    /// (Veilid tells an importer only of its own cache, [`routes`]).
    fn kill_routes(&self, node: usize) {
        let dead: Vec<String> = {
            let mut inner = self.net.inner.lock();
            let dead: Vec<String> = inner
                .routes
                .iter()
                .filter(|(_, r)| r.node == node)
                .map(|(id, _)| id.clone())
                .collect();
            for id in &dead {
                inner.routes.remove(id);
            }
            // Veilid's own value-changed route dies with them.
            if let Some(slot) = inner.nodes.get_mut(node) {
                slot.route_epoch += 1;
            }
            dead
        };
        self.net.send(
            node,
            node,
            "RouteChange",
            PortEvent::RouteChange {
                dead,
                dead_remote: Vec::new(),
            },
            false,
        );
    }

    /// A modified client's write: any writer the schema admits, bypassing
    /// the app (plan C9.3). It reaches every replica and lands on each at
    /// the newest seq any of them holds + 1.
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
    /// newest seq the replicas hold, stored on every replica that holds
    /// nothing at that seq yet (the replicas the first writer's fanout did
    /// not reach). A replica keeps the first value it got at a seq
    /// (`set_value.rs:709-719`), and so does a node's local copy.
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
            // Each replica checks the size rule (`set_value.rs:759`).
            if !records::size::fits(&record.schema, &value) {
                return Err(PortError::Dht("value too big".into()));
            }
            let mut stored = Vec::new();
            for (r, replica) in record.replicas.iter_mut().enumerate() {
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
            records::watches::mark(record, &stored, subkey, usize::MAX);
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

    /// A kill -9 on `node`: its record pools discard the writes they hold
    /// for a re-push, which live only in memory (a graceful lock saves
    /// them for the next unlock). Nothing re-pushes them.
    pub fn lose_held(&self, node: usize) {
        for pool in self.net.pools(node) {
            drop(pool.take_held());
        }
    }

    /// A stray app message reaches `to` from `from` (any node may send
    /// any bytes to a route it holds): what the receiver makes of bytes
    /// no Rekindle node sends.
    pub fn stray_message(&self, from: usize, to: usize, message: Vec<u8>) {
        self.net.send(
            from,
            to,
            "AppMessage",
            PortEvent::AppMessage { message },
            true,
        );
    }

    /// The blobs of `node`'s live general routes, as a peer who reached
    /// it would hold them (for an outsider in a scenario).
    #[must_use]
    pub fn route_blobs(&self, node: usize) -> Vec<Vec<u8>> {
        let inner = self.net.inner.lock();
        let mut ids: Vec<&String> = inner
            .routes
            .iter()
            .filter(|(id, r)| r.node == node && id.contains("-g"))
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        ids.into_iter().map(|id| routes::blob_of(id)).collect()
    }

    /// What a get from a node reaching every replica, with no local copy,
    /// answers at `key`/`subkey`: the newest value, stopping at
    /// `get_value_count` matching replicas (`records` cites Veilid).
    #[must_use]
    pub fn network_get(&self, key: &str, subkey: u32) -> Option<RecordValue> {
        let inner = self.net.inner.lock();
        let record = inner.records.get(key)?;
        let all: Vec<usize> = (0..record.replicas.len()).collect();
        records::get_from(record, &all, subkey)
    }

    /// How many inspects of `key` node `node`'s app has asked for, of any
    /// scope.
    #[must_use]
    pub fn app_inspects(&self, node: usize, key: &str) -> u64 {
        self.net.inner.lock().nodes[node]
            .app_inspects
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    /// Each replica's value at `key`/`subkey`, in placement order.
    #[must_use]
    pub fn replica_values(&self, key: &str, subkey: u32) -> Vec<Option<RecordValue>> {
        let inner = self.net.inner.lock();
        inner.records.get(key).map_or_else(Vec::new, |r| {
            r.replicas
                .iter()
                .map(|x| x.values.get(subkey as usize).cloned().flatten())
                .collect()
        })
    }

    /// The single-subkey records the network holds (sealed pages, among
    /// others): `(key, value bytes)` as [`network_get`](Self::network_get)
    /// answers, sorted by key.
    #[must_use]
    pub fn single_subkey_values(&self) -> Vec<(String, Vec<u8>)> {
        let keys: Vec<String> = self
            .net
            .inner
            .lock()
            .records
            .iter()
            .filter(|(_, r)| r.replicas.first().is_some_and(|x| x.values.len() == 1))
            .map(|(k, _)| k.clone())
            .collect();
        let mut values: Vec<(String, Vec<u8>)> = keys
            .into_iter()
            .filter_map(|k| {
                let data = self.network_get(&k, 0)?.data;
                Some((k, data))
            })
            .collect();
        values.sort();
        values
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

fn toggle(set: &mut std::collections::HashSet<usize>, node: usize, on: bool) {
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
