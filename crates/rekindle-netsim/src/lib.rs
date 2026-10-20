//! `rekindle-netsim` (plan C9.3): a seeded, deterministic simulation of the
//! Veilid network behind the record, route and message ports of
//! `rekindle_records::port`. Each simulated node gets a `PortSet`, its event
//! stream, a [`SimClock`] and a [`SimRandom`]; a real Host runs on them
//! (`TransportSource::Ports`), so the Host's adapters and engines run as in
//! production.
//!
//! Run it on a single-threaded, paused tokio runtime
//! (`#[tokio::test(flavor = "current_thread", start_paused = true)]`). The
//! pump ([`Netsim::start`]) lets every task run until it waits, then
//! delivers the events in flight in an order drawn from the seed, and
//! records each delivery in the [`Trace`]. Wire bytes never enter the trace:
//! they carry AEAD nonces from `OsRng` (plan C9.2).

mod clock;
mod messages;
mod network;
mod records;
mod routes;
mod trace;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{PortEvent, PortSet, RecordValue};
use tokio::sync::{mpsc, watch};

pub use clock::{SimClock, SimRandom, SIM_EPOCH_MS};
pub use messages::{FakeMessages, CALL_TIMEOUT};
pub use network::Network;
pub use records::{FakePool, FakeRecords};
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
    /// The node's reads return its local copy without asking the network.
    StaleReads { node: usize, stale: bool },
    /// The two groups cannot exchange app messages or calls.
    Partition { a: Vec<usize>, b: Vec<usize> },
    /// Every partition ends, and every held write is pushed again.
    Heal,
    /// This fraction (per mille) of app messages and calls is dropped.
    DropMessages { per_mille: u32 },
    /// The node never answers app calls; its callers time out.
    Unanswered { node: usize, unanswered: bool },
    /// The node's clock runs this far from simulated true time.
    ClockSkew { node: usize, ms: i64 },
    /// The node's watch on `key` dies: Veilid sends a last change with
    /// `count == 0`.
    WatchDeath { node: usize, key: String },
    /// The node's own routes die: the owner and every importer hear of it.
    RouteDeath { node: usize },
}

/// A simulated node's side: what a Host on it needs.
pub struct Node {
    pub index: usize,
    pub ports: PortSet,
    pub events: mpsc::Receiver<PortEvent>,
    pub clock: Arc<SimClock>,
    pub rng: Arc<SimRandom>,
}

/// The simulation: its network, seed, and nodes' clocks.
pub struct Netsim {
    net: Arc<Network>,
    seed: u64,
    start: tokio::time::Instant,
    clocks: parking_lot::Mutex<Vec<Arc<SimClock>>>,
}

impl Netsim {
    /// An empty network for run `seed`. Call on the paused runtime: the
    /// nodes' clocks start now.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            net: Network::new(seed),
            seed,
            start: tokio::time::Instant::now(),
            clocks: parking_lot::Mutex::new(Vec::new()),
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
            inner.nodes.push(NodeSlot {
                events: events_tx,
                ready,
                local: std::collections::HashMap::new(),
                watches: std::collections::HashMap::new(),
                pools: Vec::new(),
                imported: std::collections::HashSet::new(),
            });
            inner.nodes.len() - 1
        };
        let rng = Arc::new(SimRandom::new(self.seed, index));
        let clock = Arc::new(SimClock::new(self.start));
        self.clocks.lock().push(Arc::clone(&clock));
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
                settle().await;
                if net.deliver() == 0 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        })
    }

    /// Attach or detach `node`: its readiness, and the attachment event.
    pub fn set_ready(&self, node: usize, ready: bool) {
        if let Some(slot) = self.net.inner.lock().nodes.get(node) {
            slot.ready.send_replace(ready);
        }
        let state = if ready { "attached_full" } else { "detached" };
        self.net.send(
            node,
            node,
            "Attachment",
            PortEvent::Attachment {
                state: state.into(),
                is_attached: ready,
                public_internet_ready: ready,
                reliable_peers: 8,
                live_peers: 8,
                network_size: 64,
                median_latency_us: 80_000,
            },
            false,
        );
    }

    /// Let every task run until it waits, then deliver what is in flight;
    /// again until nothing is. For a scenario's explicit steps.
    pub async fn quiesce(&self) {
        loop {
            settle().await;
            if self.net.deliver() == 0 {
                settle().await;
                if self.net.deliver() == 0 {
                    return;
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
            Fault::StaleReads { node, stale } => {
                toggle(&mut self.net.inner.lock().faults.stale_reads, node, stale);
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
            }
            Fault::WatchDeath { node, key } => self.kill_watch(node, &key),
            Fault::RouteDeath { node } => self.kill_routes(node),
        }
    }

    fn heal(&self) {
        let count = {
            let mut inner = self.net.inner.lock();
            inner.faults.partitions.clear();
            inner.faults.dht_offline.clear();
            inner.nodes.len()
        };
        for node in 0..count {
            for pool in self.net.pools(node) {
                pool.retry_held();
            }
        }
    }

    fn kill_watch(&self, node: usize, key: &str) {
        let subkeys = records::watched_subkeys(&self.net, node, key);
        if let Some(slot) = self.net.inner.lock().nodes.get_mut(node) {
            slot.watches.remove(key);
        }
        self.net.send(
            node,
            node,
            "WatchDeath",
            PortEvent::ValueChange(rekindle_records::port::ValueChange {
                key: key.to_string(),
                subkeys: subkeys.into_iter().collect(),
                count: 0,
                value: None,
            }),
            false,
        );
    }

    fn kill_routes(&self, node: usize) {
        let (dead, importers) = {
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
            let importers: Vec<(usize, Vec<String>)> = inner
                .nodes
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != node)
                .map(|(i, slot)| {
                    let theirs: Vec<String> = dead
                        .iter()
                        .filter(|id| slot.imported.contains(*id))
                        .cloned()
                        .collect();
                    (i, theirs)
                })
                .filter(|(_, theirs)| !theirs.is_empty())
                .collect();
            (dead, importers)
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
        for (to, theirs) in importers {
            self.net.send(
                node,
                to,
                "RouteChange",
                PortEvent::RouteChange {
                    dead: Vec::new(),
                    dead_remote: theirs,
                },
                false,
            );
        }
    }

    /// A modified client's write: any writer the schema admits, bypassing
    /// the app (plan C9.3). Lands at the network's seq + 1.
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

    /// A same-seq conflict (plan C9.3): another value at the seq the
    /// network already holds, as two writers racing to one seq leave the
    /// DHT's replicas. A node holding the first value at that seq keeps it:
    /// only a higher seq replaces a local copy.
    ///
    /// # Errors
    /// As [`forge_write`](Self::forge_write); also when the subkey holds
    /// nothing yet.
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
        let value = {
            let mut inner = self.net.inner.lock();
            let record = inner
                .records
                .get_mut(key)
                .ok_or_else(|| PortError::Unreachable(key.into()))?;
            if !records::admits(record, subkey, &public) {
                return Err(PortError::NotWritable(key.into()));
            }
            let held = record
                .values
                .get(subkey as usize)
                .ok_or_else(|| PortError::Dht(format!("subkey {subkey} out of range")))?
                .clone();
            let seq = match (held, bump) {
                (Some(v), true) => v.seq.saturating_add(1),
                (None, true) => 0,
                (Some(v), false) => v.seq,
                (None, false) => {
                    return Err(PortError::Dht("no value to conflict with".into()));
                }
            };
            let value = RecordValue {
                seq,
                data,
                writer: records::public_text(&public),
            };
            record.values[subkey as usize] = Some(value.clone());
            record.published = true;
            value
        };
        self.net.notify_watchers(usize::MAX, key, subkey, &value);
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

    /// A kill -9 on `node`: its record sessions lose the writes they hold
    /// for a re-push, which live only in memory (a graceful lock saves
    /// them for the next unlock). Nothing re-pushes them, not even
    /// [`Fault::Heal`].
    pub fn lose_held(&self, node: usize) {
        for pool in self.net.pools(node) {
            pool.forget_held();
        }
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

    /// The value the network holds at `key`/`subkey`.
    #[must_use]
    pub fn network_value(&self, key: &str, subkey: u32) -> Option<RecordValue> {
        self.net
            .inner
            .lock()
            .records
            .get(key)?
            .values
            .get(subkey as usize)
            .cloned()
            .flatten()
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

/// Let every runnable task run until it waits.
async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}
