//! Private routes on the simulated network (plan C9.3): our own routes run
//! through the real `rekindle_route::own_routes::OwnRoutes` (single flight,
//! ready gating, backoff) over [`FakeAllocator`]; peers' routes import from
//! their blobs.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{PeerTarget, PortError, RouteClass, RoutePort, RouteState};
use rekindle_route::own_routes::{AllocError, OwnRoutes, RouteAllocator};
use tokio::sync::watch;

use crate::network::{Network, RouteEntry};

/// The blob prefix: a blob is this, then the route id.
const BLOB_PREFIX: &str = "netsim-route:";

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
}

impl FakeRoutes {
    pub(crate) fn new(node: usize, net: Arc<Network>, ready: watch::Receiver<bool>) -> Self {
        let own = OwnRoutes::new(FakeAllocator::new(node, Arc::clone(&net)), ready);
        Self { node, net, own }
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
        let mut inner = self.net.inner.lock();
        if !inner.routes.contains_key(&id) {
            return Err(PortError::Routing(format!("route {id} is not alive")));
        }
        if let Some(slot) = inner.nodes.get_mut(self.node) {
            slot.imported.insert(id.clone());
        }
        Ok(PeerTarget::new(id))
    }

    fn on_dead_remote(&self, dead: &[PeerTarget]) -> Vec<Vec<u8>> {
        let mut inner = self.net.inner.lock();
        let Some(slot) = inner.nodes.get_mut(self.node) else {
            return Vec::new();
        };
        dead.iter()
            .filter(|t| slot.imported.remove(t.as_str()))
            .map(|t| blob_of(t.as_str()))
            .collect()
    }

    fn invalidate_after_send_failure(&self, target: &PeerTarget) {
        if let Some(slot) = self.net.inner.lock().nodes.get_mut(self.node) {
            slot.imported.remove(target.as_str());
        }
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
