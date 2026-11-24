//! Private routes on the simulated network (plan C9.3): our own routes run
//! through the real `rekindle_route::own_routes::OwnRoutes` (single flight,
//! ready gating, backoff) over [`FakeAllocator`]; peers' routes import from
//! their blobs.
//!
//! Veilid 0.5.8 (`routing_table/route_spec_store`):
//! - **import:** any well-formed blob imports, live or not
//!   (`route_remote.rs:28-53`); it enters the importer's remote route cache
//!   (re-importing touches it, and unexpires an expired one,
//!   `route_spec_store_cache.rs:371-394`), which holds 1024 entries
//!   (`mod.rs:42`); one pushed out by LRU is reported to the importer as
//!   `dead_remote` (`route_spec_store_cache.rs:293-311`), and so is one
//!   removed (`:397-415`). The owner's route dying is never told to an
//!   importer: Veilid has no message for it. The importer learns it from
//!   its own remote route test ([`crate::loopback`] `tick_remote_route_tests`,
//!   `private_route_management.rs:396-414`, `ping_validator.rs:719-744`): a
//!   failed test removes the entry, reported as `dead_remote`
//!   (`mod.rs:235-251` `send_route_update`).
//! - **resolve:** a send looks the route up in the cache, touching it; an
//!   entry unused for 5 minutes has expired and does not resolve
//!   (`mod.rs:44`, `remote_route_cache_entry.rs:131-135`,
//!   `route_spec_store_cache.rs:331-343`), nor does one never imported:
//!   that send fails `NoConnection` (`rpc_processor/destination.rs:362-371`,
//!   `rpc_processor/error.rs:81`), which our port maps to `RouteUnusable`
//!   (`rekindle-protocol` `ports.rs:233-245`).
//!   A send to a resolved route the owner released leaves and is lost.
//! - **own route death** (Veilid 0.5.8 at `/Users/kali/veilid-wt-058`):
//!   only evidence spanning the route kills it (`route_stats.rs:19-29`
//!   `means_route_dead`): here a run of failed loopback tests (3 on an
//!   Ordered route, 5 on an Unordered one, [`crate::loopback`]). Lost
//!   questions count on a call's safety and reply routes
//!   (`rpc_processor/mod.rs:1567-1640`), which are Veilid's pool routes for
//!   every send (plan RV1), so no own route is charged. A death is reported
//!   to the owner as `dead_routes` (`route_stats.rs:716-756`).
//! - **first-hop send failure:** a send that fails at the safety route's
//!   first hop answers `NoConnection` and records a send failure
//!   (`rpc_processor/mod.rs:1433-1447`), which only asks for a recompile
//!   (`route_stats.rs:31-35` `needs_recompile`, `route_compile.rs:672`): the
//!   route lives and is not reported ([`crate::Fault::RouteLinkFailure`]).
//!   A partition or drop loses a message in transit instead.
//! - **one ordering per route:** a custom private route carries exactly its
//!   spec's preferred ordering (`route_allocate.rs:897-936`
//!   `SequenceOrderingSet::only`), every hop pair proven on that ordering's
//!   own protocols from our own published peer info on (`:899-934`), and
//!   `allocate_route` has no fallback to the other ordering (`:99-105`;
//!   only pool selection retries, `route_select.rs`). So on a network that
//!   blocks UDP an Unordered route cannot be allocated and answers
//!   `TryAgain` ("unable to find any route at this time", `:268`)
//!   ([`crate::Fault::UdpBlocked`]).
//! - **allocation takes the route's test:** `new_custom_private_route`
//!   (`veilid_api/api.rs:812-842`) allocates locally (`route_allocate.rs:99-136`:
//!   a lock, keys, hop selection, no RPC) and then tests the route over the
//!   network before it returns the blob (`api.rs:831`,
//!   `route_test.rs:62-69`): one loopback status call over the route and
//!   up to three direct-target status calls, all at once, waiting for every
//!   probe (`route_test.rs:228-338`). So an allocation costs
//!   [`route_test_duration`]; a node gone offline or not ready by the
//!   test's end fails it, and so does [`crate::Fault::AllocationTestFails`]:
//!   Veilid releases the route and answers `TryAgain` ("allocated route
//!   failed to test", `api.rs:829-836`), leaving no route behind. It is traced: [`ALLOCATION_STARTS`],
//!   [`ALLOCATION_ENDS`], and [`ALLOCATION_CUT`] when the call is dropped
//!   mid-flight (a shutdown that cancels it, a node that stops), as
//!   Veilid's `DurationRecorder` dropped unfinished logs an error
//!   (`logging/duration_recorder.rs:84-90`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rekindle_records::port::{PeerTarget, PortError, PortEvent, RouteClass, RoutePort, RouteState};
use rekindle_route::own_routes::{AllocError, OwnRoutes, RouteAllocator};
use tokio::sync::watch;

use crate::flight::AllocationFlight;
use crate::network::{Network, RouteEntry};

/// The trace entry of a node forgetting a peer's route after a send to it
/// failed `RouteUnusable` (`RoutePort::invalidate_after_send_failure`).
pub const FORGETS_PEER_ROUTE: &str = "forgets a peer route";

/// The trace entry of a node's remote route test failing: Veilid removes
/// the peer's route from the node's cache and reports it as `dead_remote`
/// (Veilid 0.5.8 `ping_validator.rs:740-744` logs "Remote route test
/// failed (no response)"; [`crate::loopback`]).
pub const RELEASES_PEER_ROUTE: &str = "a remote route test releases a peer route";

/// The trace entry of an own-route allocation starting:
/// `"{ALLOCATION_STARTS}: {class:?}"` ([`allocation_entry`]).
pub const ALLOCATION_STARTS: &str = "own route allocation starts";
/// The trace entry of an own-route allocation returning (a route or an
/// error).
pub const ALLOCATION_ENDS: &str = "own route allocation ends";
/// The trace entry of an own-route allocation dropped before it returned.
pub const ALLOCATION_CUT: &str = "own route allocation cut";

/// The trace entry `event` (one of the `ALLOCATION_*`) of `class`.
#[must_use]
pub fn allocation_entry(event: &str, class: RouteClass) -> String {
    format!("{event}: {class:?}")
}

/// The probes of an allocated route's test: one loopback plus
/// `ROUTE_TEST_DIRECT_CANDIDATE_COUNT` (3) direct targets (Veilid 0.5.8
/// `route_test.rs:4`, `:289-304`); the simulated network always knows
/// enough peers for all three.
pub(crate) const ROUTE_TEST_PROBES: usize = 4;

/// The blob prefix: a blob is this, then the route id.
const BLOB_PREFIX: &str = "netsim-route:";

/// The remote route cache's size (Veilid 0.5.8 `route_spec_store/mod.rs:42`).
pub(crate) const REMOTE_ROUTE_CACHE_SIZE: usize = 1024;

/// An unused remote route expires after this (Veilid 0.5.8
/// `route_spec_store/mod.rs:44`).
pub(crate) const REMOTE_ROUTE_EXPIRY: Duration = Duration::from_secs(300);

/// The one ordering a route carries (Veilid's `SequenceOrdering`): Ordered
/// over TCP and WebSocket, Unordered over UDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteOrdering {
    Ordered,
    Unordered,
}

impl RouteOrdering {
    /// The ordering our allocator asks for `class`, as
    /// `rekindle-protocol` `own_routes.rs` (`media_private_spec`) asks it: General is `new_private_route`,
    /// whose default spec is PreferOrdered (Veilid 0.5.8
    /// `veilid_api/types/safety.rs:16-17`,
    /// `api.rs:763-765`); Media is PreferUnordered (plan VU decision 2). A preference names the route's one ordering
    /// (`safety.rs:24-29` `preferred_ordering`). The netsim cannot link that
    /// crate (ADR 0014), so this follows it by hand.
    #[must_use]
    pub fn of(class: RouteClass) -> Self {
        match class {
            RouteClass::General => Self::Ordered,
            RouteClass::Media => Self::Unordered,
        }
    }
}

/// An entry of a node's remote route cache.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CacheEntry {
    /// When it was last used (µs).
    pub touched: u64,
    /// LRU order.
    pub used: u64,
    /// When a remote route test last proved it (µs); `None` before its
    /// first: a new or unexpired entry starts with fresh stats
    /// (Veilid 0.5.8 `remote_route_cache_entry.rs:138-143` `unexpire`) and
    /// needs testing at once (`route_stats.rs:672-681` `remote_needs_testing`).
    pub valid: Option<u64>,
    /// A remote route test in flight that gets no answer: its answer wait
    /// (Veilid's RPC timeout) ends at this µs ([`crate::loopback`]).
    pub failing: Option<u64>,
}

impl CacheEntry {
    /// A new entry, or an expired one imported again: fresh stats.
    fn fresh(touched: u64, used: u64) -> Self {
        Self {
            touched,
            used,
            valid: None,
            failing: None,
        }
    }

    /// Whether it is idle past [`REMOTE_ROUTE_EXPIRY`] at `now` (µs).
    pub(crate) fn expired(&self, now: u64) -> bool {
        let expiry = u64::try_from(REMOTE_ROUTE_EXPIRY.as_micros()).unwrap_or(u64::MAX);
        now.saturating_sub(self.touched) >= expiry
    }
}

/// Allocates a node's routes on the simulated route table.
pub struct FakeAllocator {
    node: usize,
    net: Arc<Network>,
    next: AtomicU64,
}

impl FakeAllocator {
    pub(crate) fn new(node: usize, net: Arc<Network>) -> Self {
        Self {
            node,
            net,
            next: AtomicU64::new(0),
        }
    }
}

#[async_trait::async_trait]
impl RouteAllocator for FakeAllocator {
    type Id = String;

    async fn allocate(&self, class: RouteClass) -> Result<(String, Vec<u8>), AllocError> {
        // A suspended process allocates nothing ([`crate::sleep`]).
        self.net.awake(self.node).await;
        let mut flight = AllocationFlight::start(self.node, class, &self.net);
        let result = self.allocate_and_test(class).await;
        flight.end();
        result
    }

    fn release(&self, id: &String) {
        self.net.inner.lock().routes.remove(id);
    }

    /// The simulated network does not model a route's hops.
    fn hop_count(&self, _class: RouteClass) -> Option<usize> {
        None
    }
}

impl FakeAllocator {
    /// Veilid's `allocate_route` (local: it fails at once when no route can
    /// be built), then the route's test over the network.
    async fn allocate_and_test(&self, class: RouteClass) -> Result<(String, Vec<u8>), AllocError> {
        let ordering = RouteOrdering::of(class);
        {
            let inner = self.net.inner.lock();
            if inner.faults.dht_offline.contains(&self.node)
                || inner.faults.not_ready.contains(&self.node)
            {
                return Err(AllocError::TryAgain("offline".into()));
            }
            if ordering == RouteOrdering::Unordered && inner.faults.udp_blocked.contains(&self.node)
            {
                // No hop pair is viable on UDP alone (Veilid 0.5.8 `route_allocate.rs:897-934`).
                return Err(AllocError::TryAgain(
                    "unable to find any route at this time".into(),
                ));
            }
        }
        tokio::time::sleep(route_test_duration(&self.net, self.node, class)).await;
        let n = self.next.fetch_add(1, Ordering::SeqCst);
        let id = format!("n{}-{}{n}", self.node, class_tag(class));
        let mut inner = self.net.inner.lock();
        if inner.faults.dht_offline.contains(&self.node)
            || inner.faults.not_ready.contains(&self.node)
            || inner
                .faults
                .allocation_test_fails
                .contains(&(self.node, class))
        {
            // The test got no answer: Veilid releases the route it just
            // allocated (never inserted here) and answers `TryAgain`
            // (Veilid 0.5.8 `veilid_api/api.rs:829-836`).
            return Err(AllocError::TryAgain(
                "allocated route failed to test".into(),
            ));
        }
        inner.routes.insert(
            id.clone(),
            RouteEntry {
                node: self.node,
                ordering,
            },
        );
        let blob = blob_of(&id);
        Ok((id, blob))
    }
}

/// How long `node`'s route test takes: its [`ROUTE_TEST_PROBES`] status
/// calls run at once and the test waits for all of them
/// (Veilid 0.5.8 `route_test.rs:300-327`); each is a question and its
/// answer, two [`crate::latency::MESSAGE`] legs as an app call's
/// (`messages.rs` `call_over`). Drawn from the class's own stream: the
/// classes' allocations start together on node-ready, in the order tokio
/// wakes them.
pub(crate) fn route_test_duration(net: &Network, node: usize, class: RouteClass) -> Duration {
    let stream = u64::from(class_tag(class));
    let leg = || {
        net.latency
            .draw_stream(node, stream, crate::latency::MESSAGE)
    };
    (0..ROUTE_TEST_PROBES)
        .map(|_| leg() + leg())
        .max()
        .unwrap_or_default()
}

/// A node's routes: peers' imported from their blobs, our own through
/// `OwnRoutes`.
pub struct FakeRoutes {
    node: usize,
    net: Arc<Network>,
    own: Arc<OwnRoutes<FakeAllocator>>,
    /// The port's imports by route id, as production's `RouteImports`
    /// (`rekindle-protocol` `dht/route_imports.rs`): a blob imported once is
    /// not imported again until forgotten.
    imports: Mutex<HashMap<String, Vec<u8>>>,
}

impl FakeRoutes {
    pub(crate) fn new(node: usize, net: Arc<Network>, own: Arc<OwnRoutes<FakeAllocator>>) -> Self {
        Self {
            node,
            net,
            own,
            imports: Mutex::new(HashMap::new()),
        }
    }
}

/// The letter a route id names its class by: `n{node}-{tag}{n}`.
pub(crate) fn class_tag(class: RouteClass) -> char {
    match class {
        RouteClass::General => 'g',
        RouteClass::Media => 'm',
    }
}

/// Whether route `id` is `node`'s route of the class `tag` names.
pub(crate) fn is_class(id: &str, node: usize, tag: char) -> bool {
    id.strip_prefix(&format!("n{node}-"))
        .is_some_and(|rest| rest.starts_with(tag))
}

/// The blob of route `id`.
pub(crate) fn blob_of(id: &str) -> Vec<u8> {
    format!("{BLOB_PREFIX}{id}").into_bytes()
}

/// The route id a blob names.
pub(crate) fn route_id(blob: &[u8]) -> Option<String> {
    std::str::from_utf8(blob)
        .ok()?
        .strip_prefix(BLOB_PREFIX)
        .map(ToString::to_string)
}

#[async_trait::async_trait]
impl RoutePort for FakeRoutes {
    fn import(&self, blob: &[u8]) -> Result<PeerTarget, PortError> {
        let id = route_id(blob).ok_or_else(|| PortError::Routing("not a route blob".into()))?;
        let mut imports = self.imports.lock();
        if !imports.contains_key(&id) {
            self.net.import_route(self.node, &id);
            imports.insert(id.clone(), blob.to_vec());
        }
        Ok(PeerTarget::new(id))
    }

    fn on_dead_remote(&self, dead: &[PeerTarget]) -> Vec<Vec<u8>> {
        let mut imports = self.imports.lock();
        dead.iter()
            .filter_map(|t| imports.remove(t.as_str()))
            .collect()
    }

    fn invalidate_after_send_failure(&self, target: &PeerTarget) {
        // Forgotten, not released (as production). Traced, so a scenario
        // can tell a peer's route forgotten from one kept.
        self.imports.lock().remove(target.as_str());
        self.net
            .inner
            .lock()
            .trace
            .push(self.node, FORGETS_PEER_ROUTE.to_string());
    }

    fn own_blob(&self, class: RouteClass) -> Option<Vec<u8>> {
        self.own.blob(class)
    }

    fn own_id(&self, class: RouteClass) -> Option<String> {
        self.own.id(class)
    }

    fn own_state(&self, class: RouteClass) -> watch::Receiver<RouteState> {
        self.own.state(class)
    }

    async fn available_own(&self, class: RouteClass) -> Vec<u8> {
        self.own.available(class).await
    }

    fn own_age(&self, class: RouteClass) -> Option<Duration> {
        self.own.age(class)
    }

    fn want_own(&self) {
        self.own.want_all();
    }

    fn want_own_class(&self, class: RouteClass) {
        self.own.want(class);
    }

    fn release_own_class(&self, class: RouteClass) {
        self.own.release(class);
    }

    fn replace_own_class(&self, class: RouteClass) {
        self.own.replace(class);
    }

    fn release_retired_own(&self, class: RouteClass) {
        self.own.release_retired(class);
    }

    fn on_dead_own(&self, dead: &[String]) {
        self.own.on_dead(dead);
    }

    fn renew_own(&self) {
        self.own.renew();
    }

    fn release_own(&self) {
        self.own.release_all();
    }

    async fn shutdown_own(&self) {
        self.own.shutdown().await;
    }

    fn attach_budget(
        &self,
        state: rekindle_records::port::RouteBudgetState,
        wall: rekindle_records::port::WallMs,
        save: rekindle_records::port::SaveBudget,
    ) {
        self.own.attach_budget(state, wall, save);
    }

    fn attach_unreadable_budget(&self) {
        self.own.attach_unreadable_budget();
    }

    fn detach_budget(&self) {
        self.own.detach_budget();
    }

    fn spend_budget(&self) -> bool {
        self.own.spend_external()
    }

    fn own_paused(&self) -> watch::Receiver<Option<rekindle_records::port::RoutePauseCause>> {
        self.own.paused()
    }

    fn own_deaths(&self, class: RouteClass) -> rekindle_records::port::OwnDeaths {
        self.own.deaths(class)
    }
}

impl Network {
    /// `node` imports route `id` into its remote route cache: a new entry,
    /// or a touch that unexpires (Veilid 0.5.8 `route_spec_store_cache.rs:371-394`); a
    /// full cache pushes its least recently used entry out, reported to
    /// `node` as `dead_remote` (`:293-311`).
    pub(crate) fn import_route(&self, node: usize, id: &str) {
        let now = self.now_us();
        let evicted = {
            let mut inner = self.inner.lock();
            inner.lru += 1;
            let used = inner.lru;
            let Some(slot) = inner.nodes.get_mut(node) else {
                return;
            };
            let entry = slot
                .imported
                .entry(id.to_string())
                .or_insert_with(|| CacheEntry::fresh(now, used));
            if entry.expired(now) {
                *entry = CacheEntry::fresh(now, used);
            } else {
                entry.touched = now;
                entry.used = used;
            }
            if slot.imported.len() > REMOTE_ROUTE_CACHE_SIZE {
                let oldest = slot
                    .imported
                    .iter()
                    .min_by_key(|(_, e)| e.used)
                    .map(|(k, _)| k.clone());
                oldest.inspect(|k| {
                    slot.imported.remove(k);
                })
            } else {
                None
            }
        };
        if let Some(dead) = evicted {
            self.send(
                node,
                node,
                "RouteChange",
                PortEvent::RouteChange {
                    dead: Vec::new(),
                    dead_remote: vec![dead],
                },
                false,
            );
        }
    }

    /// Whether `node`'s cache resolves route `id` for a send, touching it:
    /// not when never imported or idle past [`REMOTE_ROUTE_EXPIRY`]
    /// (Veilid 0.5.8 `route_spec_store_cache.rs:331-343`).
    pub(crate) fn resolve_route(&self, node: usize, id: &str) -> bool {
        let now = self.now_us();
        let mut inner = self.inner.lock();
        inner.lru += 1;
        let used = inner.lru;
        let Some(entry) = inner
            .nodes
            .get_mut(node)
            .and_then(|s| s.imported.get_mut(id))
        else {
            return false;
        };
        if entry.expired(now) {
            return false;
        }
        entry.touched = now;
        entry.used = used;
        true
    }

    /// Whether `node`'s send fails at its safety route's first hop
    /// ([`crate::Fault::RouteLinkFailure`]): the fault is spent and the
    /// send answers `NoConnection`; the route lives and is recompiled for
    /// the next send, which may start at another first hop
    /// (Veilid 0.5.8 `route_stats.rs:31-35`, `route_compile.rs:672`).
    pub(crate) fn first_hop_failed(&self, node: usize) -> bool {
        self.inner.lock().faults.link_failure.remove(&node)
    }

    /// An answer reached `node`: the pool reply route it came over
    /// is known valid now.
    pub(crate) fn answered(&self, node: usize) {
        if let Some(slot) = self.inner.lock().nodes.get_mut(node) {
            slot.pool_reply_valid = Some(tokio::time::Instant::now());
        }
    }

    /// `node` asks an unpinned question: Veilid selects its pool reply
    /// route, allocating a new one when the last was released
    /// ([`ROUTE_MIN_IDLE`](crate::loopback::ROUTE_MIN_IDLE) past its last
    /// validity); a new route is tested on the next tick
    /// (Veilid 0.5.8 `private_route_management.rs:119-121`, never tested), so it counts
    /// as valid from now.
    pub(crate) fn select_pool_reply(&self, node: usize) {
        if let Some(slot) = self.inner.lock().nodes.get_mut(node) {
            let released = slot
                .pool_reply_valid
                .is_none_or(|valid| valid.elapsed() >= crate::loopback::ROUTE_MIN_IDLE);
            if released {
                slot.pool_reply_valid = Some(tokio::time::Instant::now());
            }
        }
    }
}
