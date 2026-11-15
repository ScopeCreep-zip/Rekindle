//! Route tests on the simulated network (plan VU2 items 3 and 11). Veilid
//! 0.5.8 tests each of a node's own routes by a loopback question sent out
//! over the route and back in through it (`routing_table/tasks/ping_validator.rs`,
//! `route_spec_store/route_stats.rs:523-564` `record_loopback_result`). A
//! route that carries traffic end to end passes; one that does not
//! ([`crate::Fault::LoopbackFailures`], or an Unordered route once its node's
//! network blocks UDP, [`crate::Fault::UdpBlocked`]) fails, and the run of
//! failures kills it:
//! - **when tested:** a published route is tested once it needs testing
//!   (`routing_domains/public_internet/controller/private_route_management.rs:113-121`): idle for
//!   [`ROUTE_MIN_IDLE`] since it was last known valid, or at once after a
//!   failure (`route_stats.rs:144-158`) or a question lost on it, which
//!   enqueues an out-of-band test (`rpc_processor/mod.rs:1586-1600`). The
//!   management task runs every [`ROUTE_TEST_TICK`]
//!   (`routing_domains/public_internet/controller/mod.rs:23`). A broken
//!   route receives nothing, so it was last valid when it broke: its first
//!   test comes [`ROUTE_MIN_IDLE`] later, or sooner after a lost question.
//! - **how long a failing test takes:** its answer wait, Veilid's RPC
//!   timeout ([`crate::RPC_TIMEOUT`]; the routed allowance of 10 % per hop
//!   past 4, `rpc_processor/mod.rs:100-117`, is left out), then the next
//!   tick.
//! - **death:** a run of 3 failures on an Ordered route, 5 on an
//!   Unordered one (`route_spec_store/mod.rs:51-63`), kills it
//!   (`route_stats.rs:19-29` `means_route_dead`); it is released and the
//!   owner hears `dead_routes` (`route_stats.rs:716-756`). A send failure
//!   at the first hop is not death ([`crate::Fault::RouteLinkFailure`]).
//! - **offline never blames:** while the node is Offline a failure is
//!   deferred and retested (`ping_validator.rs:813-829`); here a node that
//!   is detached, DHT-offline or not ready runs no test.

use std::time::Duration;

use rekindle_records::port::{PortEvent, RouteClass};

use crate::network::Network;
use crate::routes::RouteOrdering;

/// A published route is tested after this long without being known valid
/// (`route_spec_store/mod.rs:46` `ROUTE_MIN_IDLE_TIME_MS`).
pub(crate) const ROUTE_MIN_IDLE: Duration = Duration::from_secs(30);

/// The private route management task's period
/// (`routing_domains/public_internet/controller/mod.rs:23`).
pub(crate) const ROUTE_TEST_TICK: Duration = Duration::from_secs(1);

/// A run of failing loopback tests on one own route.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LoopbackRun {
    /// Consecutive failed tests so far.
    pub failures: usize,
    /// When the next test runs.
    pub next_test: tokio::time::Instant,
    /// The run comes from [`crate::Fault::UdpBlocked`], and ends when UDP
    /// is unblocked: the next test passes.
    pub udp: bool,
}

impl LoopbackRun {
    /// A run on a route that broke now: first tested [`ROUTE_MIN_IDLE`]
    /// later.
    pub(crate) fn new(udp: bool) -> Self {
        Self {
            failures: 0,
            next_test: tokio::time::Instant::now() + ROUTE_MIN_IDLE,
            udp,
        }
    }
}

/// The run of loopback failures that kills a route of `ordering`
/// (`route_spec_store/mod.rs:51-63`).
pub(crate) fn threshold(ordering: RouteOrdering) -> usize {
    match ordering {
        RouteOrdering::Ordered => 3,
        RouteOrdering::Unordered => 5,
    }
}

impl Network {
    /// Run the route tests that are due: each fails, and a route whose run
    /// reaches its threshold dies, reported to its owner as `dead_routes`.
    pub(crate) fn tick_route_tests(&self) {
        let now = tokio::time::Instant::now();
        let mut dead: Vec<(usize, String)> = Vec::new();
        {
            let mut guard = self.inner.lock();
            let inner = &mut *guard;
            let mut ids: Vec<String> = inner.faults.loopback.keys().cloned().collect();
            ids.sort();
            for id in ids {
                let Some(route) = inner.routes.get(&id).cloned() else {
                    inner.faults.loopback.remove(&id);
                    continue;
                };
                let offline = inner.faults.detached.contains(&route.node)
                    || inner.faults.dht_offline.contains(&route.node)
                    || inner.faults.not_ready.contains(&route.node);
                let Some(run) = inner.faults.loopback.get_mut(&id) else {
                    continue;
                };
                if offline || now < run.next_test {
                    continue;
                }
                run.failures += 1;
                run.next_test = now + crate::RPC_TIMEOUT + ROUTE_TEST_TICK;
                if run.failures >= threshold(route.ordering) {
                    inner.faults.loopback.remove(&id);
                    inner.routes.remove(&id);
                    if let Some(slot) = inner.nodes.get_mut(route.node) {
                        slot.lost.remove(&id);
                    }
                    dead.push((route.node, id));
                }
            }
        }
        for (node, id) in dead {
            self.send(
                node,
                node,
                "RouteChange",
                PortEvent::RouteChange {
                    dead: vec![id],
                    dead_remote: Vec::new(),
                },
                false,
            );
        }
    }

    /// [`crate::Fault::LoopbackFailures`]: `node`'s live route of `class`
    /// breaks now.
    pub(crate) fn break_route(&self, node: usize, class: RouteClass) {
        let tag = crate::routes::class_tag(class);
        let mut inner = self.inner.lock();
        let mut ids: Vec<String> = inner
            .routes
            .iter()
            .filter(|(id, r)| r.node == node && crate::routes::is_class(id, node, tag))
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        for id in ids {
            inner.faults.loopback.insert(id, LoopbackRun::new(false));
        }
    }

    /// [`crate::Fault::UdpBlocked`]: `node`'s live Unordered routes break
    /// (or, unblocked, the runs UDP caused end: their next test passes).
    pub(crate) fn block_udp(&self, node: usize, blocked: bool) {
        let mut guard = self.inner.lock();
        let inner = &mut *guard;
        crate::toggle(&mut inner.faults.udp_blocked, node, blocked);
        let mut ids: Vec<String> = inner
            .routes
            .iter()
            .filter(|(_, r)| r.node == node && r.ordering == RouteOrdering::Unordered)
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        for id in ids {
            if blocked {
                inner
                    .faults
                    .loopback
                    .entry(id)
                    .or_insert_with(|| LoopbackRun::new(true));
            } else if inner.faults.loopback.get(&id).is_some_and(|r| r.udp) {
                inner.faults.loopback.remove(&id);
            }
        }
    }

    /// Whether route `id` fails end to end (its loopback tests fail):
    /// nothing sent over it or to it arrives.
    pub(crate) fn route_broken(&self, id: &str) -> bool {
        self.inner.lock().faults.loopback.contains_key(id)
    }

    /// A question lost on broken route `id` enqueues an out-of-band test
    /// (`rpc_processor/mod.rs:1586-1600`): it runs on the next tick.
    pub(crate) fn test_soon(inner: &mut crate::network::Inner, id: &str) {
        if let Some(run) = inner.faults.loopback.get_mut(id) {
            run.next_test = run
                .next_test
                .min(tokio::time::Instant::now() + ROUTE_TEST_TICK);
        }
    }
}
