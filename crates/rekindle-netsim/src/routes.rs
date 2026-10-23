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
//! - **own route death from use:** a question lost on the way (a call that
//!   timed out) counts its destination on the asker's reply private route,
//!   its own general route (`rpc_processor/mod.rs:1354-1365`); 5 distinct
//!   destinations kill the route (`route_stats.rs:56-60`,
//!   `mod.rs:46`), an answer clears them (`route_stats.rs:300-312`), and
//!   the death is reported to the owner as `dead_routes` (`mod.rs:183-194`).
//!   A send that fails at the safety route's first hop is a send failure
//!   on that route (`rpc_processor/mod.rs:1255-1261`); one kills it
//!   (`mod.rs:48`, `route_stats.rs:56-60`), reported as `dead_routes`, and
//!   the send answers `NoConnection` ([`crate::Fault::RouteLinkFailure`]).
//!   A partition or drop loses a message in transit instead.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rekindle_records::port::{PeerTarget, PortError, PortEvent, RouteClass, RoutePort, RouteState};
use rekindle_route::own_routes::{AllocError, OwnRoutes, RouteAllocator};
use tokio::sync::watch;

use crate::network::{Network, RouteEntry};

/// The blob prefix: a blob is this, then the route id.
const BLOB_PREFIX: &str = "netsim-route:";

/// The remote route cache's size (`route_spec_store/mod.rs:40`).
pub(crate) const REMOTE_ROUTE_CACHE_SIZE: usize = 1024;

/// An unused remote route expires after this (`route_spec_store/mod.rs:42`).
pub(crate) const REMOTE_ROUTE_EXPIRY: Duration = Duration::from_secs(300);

/// Distinct lost-question destinations that kill an own route
/// (`route_spec_store/mod.rs:46`).
pub(crate) const LOST_DESTINATIONS_INVALIDATE: usize = 5;

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
        let n = self.next.fetch_add(1, Ordering::SeqCst);
        let tag = match class {
            RouteClass::General => "g",
            RouteClass::Media => "m",
            RouteClass::Sending => "s",
        };
        let id = format!("n{}-{tag}{n}", self.node);
        let blob = blob_of(&id);
        let mut inner = self.net.inner.lock();
        if inner.faults.dht_offline.contains(&self.node)
            || inner.faults.not_ready.contains(&self.node)
        {
            return Err(AllocError::TryAgain("offline".into()));
        }
        inner
            .routes
            .insert(id.clone(), RouteEntry { node: self.node });
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
        // Forgotten, not released (as production).
        self.imports.lock().remove(target.as_str());
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

    /// A question `node` asked of `destination` was lost: counted on its
    /// reply private route, the `pinned` route when the call was pinned
    /// (`rpc_processor/destination.rs:477-497`), else its own general
    /// routes; the fifth distinct destination kills a route, reported to
    /// `node` as `dead_routes`.
    pub(crate) fn lost_question(&self, node: usize, destination: &str, pinned: Option<&str>) {
        let dead: Vec<String> = {
            let mut guard = self.inner.lock();
            let inner = &mut *guard;
            let own: Vec<String> = sent_over(&inner.routes, node, pinned);
            let Some(slot) = inner.nodes.get_mut(node) else {
                return;
            };
            let dead: Vec<String> = own
                .into_iter()
                .filter(|id| {
                    let lost = slot.lost.entry(id.clone()).or_default();
                    lost.insert(destination.to_string());
                    lost.len() >= LOST_DESTINATIONS_INVALIDATE
                })
                .collect();
            for id in &dead {
                slot.lost.remove(id);
                inner.routes.remove(id);
            }
            dead
        };
        if !dead.is_empty() {
            self.send(
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

    /// Whether `node`'s send fails at its own route's first hop
    /// ([`crate::Fault::RouteLinkFailure`]): the fault is spent, and the
    /// route it went out on (the `pinned` route, else the first live
    /// general route) dies after this one failure, reported to `node` as
    /// `dead_routes`.
    pub(crate) fn first_hop_failed(&self, node: usize, pinned: Option<&str>) -> bool {
        let dead: Vec<String> = {
            let mut guard = self.inner.lock();
            let inner = &mut *guard;
            if !inner.faults.link_failure.remove(&node) {
                return false;
            }
            let dead: Vec<String> = sent_over(&inner.routes, node, pinned)
                .into_iter()
                .take(1)
                .collect();
            for id in &dead {
                inner.routes.remove(id);
                if let Some(slot) = inner.nodes.get_mut(node) {
                    slot.lost.remove(id);
                }
            }
            dead
        };
        if !dead.is_empty() {
            self.send(
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
        true
    }

    /// An answer reached `node`: its own routes' lost destinations clear.
    pub(crate) fn answered(&self, node: usize) {
        if let Some(slot) = self.inner.lock().nodes.get_mut(node) {
            slot.lost.clear();
        }
    }
}

/// The live route a send of `node`'s went out on: `pinned` while live,
/// else its general routes.
fn sent_over(
    routes: &HashMap<String, RouteEntry>,
    node: usize,
    pinned: Option<&str>,
) -> Vec<String> {
    match pinned {
        Some(id) => routes
            .get(id)
            .filter(|r| r.node == node)
            .map(|_| vec![id.to_string()])
            .unwrap_or_default(),
        None => own_general(routes, node),
    }
}

/// `node`'s live general routes, sorted.
fn own_general(routes: &HashMap<String, RouteEntry>, node: usize) -> Vec<String> {
    let mut ids: Vec<String> = routes
        .iter()
        .filter(|(id, r)| r.node == node && id.contains("-g"))
        .map(|(id, _)| id.clone())
        .collect();
    ids.sort();
    ids
}
