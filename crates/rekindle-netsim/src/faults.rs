//! The faults a scenario puts in force (plan C9.3) and how the sim
//! applies them; split from `lib.rs` along its seam.

use super::{pair, toggle, Netsim, PortEvent};

/// A fault to put in force (plan C9.3).
#[derive(Debug, Clone)]
pub enum Fault {
    /// The node's next `calls` DHT calls answer "try again".
    TryAgain { node: usize, calls: u32 },
    /// Every DHT call of the node fails (or stops failing): no connection.
    DhtOffline { node: usize, offline: bool },
    /// Every set of `record` by the node lands below consensus (no replica
    /// accepts it and nothing is stored, as a Veilid `set_value` can),
    /// until [`Fault::Heal`]; its other records' writes are unaffected.
    WriteFails { node: usize, record: String },
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

impl Netsim {
    /// Put `fault` in force.
    pub fn fault(&self, fault: Fault) {
        match fault {
            Fault::TryAgain { node, calls } => {
                self.net.inner.lock().faults.try_again.insert(node, calls);
            }
            Fault::DhtOffline { node, offline } => {
                toggle(&mut self.net.inner.lock().faults.dht_offline, node, offline);
            }
            Fault::WriteFails { node, record } => {
                self.net
                    .inner
                    .lock()
                    .faults
                    .write_fails
                    .insert((node, record));
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
            inner.faults.write_fails.clear();
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
}
