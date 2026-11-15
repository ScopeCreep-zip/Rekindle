//! Private routes on the simulated network (plan C9.3): our own routes run
//! through the real `rekindle_route::own_routes::OwnRoutes` (single flight,
//! ready gating, backoff) over [`FakeAllocator`]; peers' routes import from
//! their blobs.
//!
//! The pinned veilid-core (`routing_table/route_spec_store`):
//! - **import:** any well-formed blob imports, live or not
//!   (`route_remote.rs:28-53`); it enters the importer's remote route cache
//!   (re-importing touches it, and unexpires an expired one,
//!   `route_spec_store_cache.rs:371-394`), which holds 1024 entries
//!   (`mod.rs:40`); one pushed out by LRU is reported to the importer as
//!   `dead_remote` (`route_spec_store_cache.rs:293-311`), and so is one
//!   removed (`:398-414`). The owner's route dying is never told to an
//!   importer: Veilid has no message for it (`mod.rs:183-194` reports only
//!   the node's own cache).
//! - **resolve:** a send looks the route up in the cache, touching it; an
//!   entry unused for 5 minutes has expired and does not resolve
//!   (`mod.rs:42`, `remote_route_cache_entry.rs:131-135`,
//!   `route_spec_store_cache.rs:331-343`), nor does one never imported:
//!   that send fails `NoConnection` (`routing_context.rs:368-373`), which
//!   our port maps to `RouteUnusable` (`rekindle-protocol` `ports.rs:179`).
//!   A send to a resolved route the owner released leaves and is lost.
//! - **own route death** (Veilid 0.5.8 at `/Users/kali/veilid-wt-058`):
//!   only evidence spanning the route kills it (`route_stats.rs:19-29`
//!   `means_route_dead`): a run of failed loopback tests (3 on an Ordered
//!   route, 5 on an Unordered one, [`crate::loopback`]), or questions lost
//!   to [`LOST_DESTINATIONS_INVALIDATE`] distinct destinations
//!   (`route_stats.rs:134-141`). A lost question counts on the call's
//!   safety and reply routes (`rpc_processor/mod.rs:1567-1640`): for a
//!   pinned call that is the pinned route, once (the reply route equal to
//!   the safety route is not counted twice, `:1621-1624`); for
//!   an unpinned call they are Veilid's automatic pool routes
//!   (`route_select.rs:131-136` asks our 3-hop length, which no own route
//!   of the default 1-hop length matches), so no own route is charged. A
//!   loss on a safety route enqueues an out-of-band loopback test
//!   (`rpc_processor/mod.rs:1586-1600`) whose pass clears the lost
//!   destinations (`route_stats.rs:536-543`): on a route that works, only
//!   losses inside one test's span ([`OUT_OF_BAND_TEST`]) add up. An answer
//!   clears them too (`route_stats.rs:489-508`). A death is reported to
//!   the owner as `dead_routes` (`route_stats.rs:716-756`).
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

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rekindle_records::port::{PeerTarget, PortError, PortEvent, RouteClass, RoutePort, RouteState};
use rekindle_route::own_routes::{AllocError, OwnRoutes, RouteAllocator};
use tokio::sync::watch;

use crate::network::{Network, RouteEntry};

/// The trace entry of a node forgetting a peer's route after a send to it
/// failed `RouteUnusable` (`RoutePort::invalidate_after_send_failure`).
pub const FORGETS_PEER_ROUTE: &str = "forgets a peer route";

/// The blob prefix: a blob is this, then the route id.
const BLOB_PREFIX: &str = "netsim-route:";

/// The remote route cache's size (`route_spec_store/mod.rs:40`).
pub(crate) const REMOTE_ROUTE_CACHE_SIZE: usize = 1024;

/// An unused remote route expires after this (`route_spec_store/mod.rs:42`).
pub(crate) const REMOTE_ROUTE_EXPIRY: Duration = Duration::from_secs(300);

/// Distinct lost-question destinations that kill an own route
/// (`route_spec_store/mod.rs:48`), each at full blame weight
/// (`route_stats.rs:134-141`).
pub(crate) const LOST_DESTINATIONS_INVALIDATE: usize = 5;

/// How long after a loss on a working route its out-of-band loopback test
/// passes and clears the lost destinations: the next management tick
/// ([`crate::loopback::ROUTE_TEST_TICK`]) plus the loopback's round trip,
/// two [`crate::latency::MESSAGE`] legs at their longest.
pub(crate) const OUT_OF_BAND_TEST: Duration = Duration::from_millis(1_000 + 2 * 800);

/// The one ordering a route carries (Veilid's `SequenceOrdering`): Ordered
/// over TCP and WebSocket, Unordered over UDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteOrdering {
    Ordered,
    Unordered,
}

impl RouteOrdering {
    /// The ordering our allocator asks for `class`, as
    /// `rekindle-protocol` `own_routes.rs` (`media_private_spec`,
    /// `sending_private_spec`) asks it: General is `new_private_route`,
    /// whose default spec is PreferOrdered (`veilid_api/types/safety.rs:16-17`,
    /// `api.rs:763-765`); Sending is PreferOrdered (plan VU decision 2) and
    /// Media PreferUnordered. A preference names the route's one ordering
    /// (`safety.rs:24-29` `preferred_ordering`). The netsim cannot link that
    /// crate (ADR 0014), so this follows it by hand.
    #[must_use]
    pub fn of(class: RouteClass) -> Self {
        match class {
            RouteClass::General | RouteClass::Sending => Self::Ordered,
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
        let n = self.next.fetch_add(1, Ordering::SeqCst);
        let id = format!("n{}-{}{n}", self.node, class_tag(class));
        let blob = blob_of(&id);
        let ordering = RouteOrdering::of(class);
        let mut inner = self.net.inner.lock();
        if inner.faults.dht_offline.contains(&self.node)
            || inner.faults.not_ready.contains(&self.node)
        {
            return Err(AllocError::TryAgain("offline".into()));
        }
        if ordering == RouteOrdering::Unordered && inner.faults.udp_blocked.contains(&self.node) {
            // No hop pair is viable on UDP alone (`route_allocate.rs:897-934`).
            return Err(AllocError::TryAgain(
                "unable to find any route at this time".into(),
            ));
        }
        inner.routes.insert(
            id.clone(),
            RouteEntry {
                node: self.node,
                ordering,
            },
        );
        Ok((id, blob))
    }

    fn release(&self, id: &String) {
        self.net.inner.lock().routes.remove(id);
    }
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
        RouteClass::Sending => 's',
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
}

impl Network {
    /// `node` imports route `id` into its remote route cache: a new entry,
    /// or a touch that unexpires (`route_spec_store_cache.rs:371-394`); a
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
            slot.imported
                .insert(id.to_string(), CacheEntry { touched: now, used });
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
    /// (`route_spec_store_cache.rs:331-343`).
    pub(crate) fn resolve_route(&self, node: usize, id: &str) -> bool {
        let now = self.now_us();
        let mut inner = self.inner.lock();
        inner.lru += 1;
        let used = inner.lru;
        let expiry = u64::try_from(REMOTE_ROUTE_EXPIRY.as_micros()).unwrap_or(u64::MAX);
        let Some(entry) = inner
            .nodes
            .get_mut(node)
            .and_then(|s| s.imported.get_mut(id))
        else {
            return false;
        };
        if now.saturating_sub(entry.touched) >= expiry {
            return false;
        }
        *entry = CacheEntry { touched: now, used };
        true
    }

    /// A question `node` asked of `destination` was lost: counted on the
    /// `pinned` route when the call was pinned, on no own route otherwise
    /// ([`routes`](crate::routes)). On a working route the losses older than
    /// [`OUT_OF_BAND_TEST`] were cleared by the test the first of them
    /// enqueued; on a broken one that test fails, run on the next tick
    /// ([`crate::loopback`]). The fifth distinct destination kills the route,
    /// reported to `node` as `dead_routes`.
    pub(crate) fn lost_question(&self, node: usize, destination: &str, pinned: Option<&str>) {
        let Some(id) = pinned else {
            return;
        };
        let now = tokio::time::Instant::now();
        let dead = {
            let mut guard = self.inner.lock();
            let inner = &mut *guard;
            if inner.routes.get(id).is_none_or(|r| r.node != node) {
                return;
            }
            let broken = inner.faults.loopback.contains_key(id);
            Network::test_soon(inner, id);
            let Some(slot) = inner.nodes.get_mut(node) else {
                return;
            };
            let lost = slot.lost.entry(id.to_string()).or_default();
            if !broken {
                lost.retain(|_, at| now.duration_since(*at) < OUT_OF_BAND_TEST);
            }
            lost.insert(destination.to_string(), now);
            let dead = lost.len() >= LOST_DESTINATIONS_INVALIDATE;
            if dead {
                slot.lost.remove(id);
                inner.routes.remove(id);
                inner.faults.loopback.remove(id);
            }
            dead
        };
        if dead {
            self.send(
                node,
                node,
                "RouteChange",
                PortEvent::RouteChange {
                    dead: vec![id.to_string()],
                    dead_remote: Vec::new(),
                },
                false,
            );
        }
    }

    /// Whether `node`'s send fails at its safety route's first hop
    /// ([`crate::Fault::RouteLinkFailure`]): the fault is spent and the
    /// send answers `NoConnection`; the route lives and is recompiled for
    /// the next send, which may start at another first hop
    /// (`route_stats.rs:31-35`, `route_compile.rs:672`).
    pub(crate) fn first_hop_failed(&self, node: usize) -> bool {
        self.inner.lock().faults.link_failure.remove(&node)
    }

    /// An answer reached `node`: its own routes' lost destinations clear
    /// (`route_stats.rs:489-508`), and the pool reply route it came over
    /// (an unpinned call's, `pool`) is known valid now.
    pub(crate) fn answered(&self, node: usize, pool: bool) {
        if let Some(slot) = self.inner.lock().nodes.get_mut(node) {
            slot.lost.clear();
            if pool {
                slot.pool_reply_valid = Some(tokio::time::Instant::now());
            }
        }
    }

    /// `node` asks an unpinned question: Veilid selects its pool reply
    /// route, allocating a new one when the last was released
    /// ([`ROUTE_MIN_IDLE`](crate::loopback::ROUTE_MIN_IDLE) past its last
    /// validity); a new route is tested on the next tick
    /// (`private_route_management.rs:119-121`, never tested), so it counts
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
