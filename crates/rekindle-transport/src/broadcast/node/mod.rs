//! Transport node lifecycle — the sole owner of the Veilid connection.
//!
//! [`TransportNode`] encapsulates the entire Veilid API. It is created
//! once at application startup and shut down on exit. No other code in
//! the workspace touches `veilid_core` directly.

use std::sync::Arc;

use tokio::sync::mpsc;
use veilid_core::{RoutingContext, VeilidAPI};

use crate::config::{SafetyProfile, TransportConfig};

use super::peer_registry::PeerRegistry;
use super::send::{Caller, Sender};
use crate::error::{Result, TransportError};
use crate::shared::{SharedState, TransportNotification, TransportSnapshot};

mod keys;
mod lifecycle;
mod resume;
mod routes;

pub use keys::{deserialize_keypair, ed25519_to_keypair, serialize_keypair};

/// The top-level transport node. Owns the Veilid API handle and all
/// subsystems. There is exactly one of these per application lifetime.
pub struct TransportNode {
    api: VeilidAPI,
    config: Arc<TransportConfig>,
    shutdown_tx: mpsc::Sender<()>,
    /// `None` for outbound-only adoption (W16.9b) — host runs its own
    /// dispatch loop, transport provides senders/DHT/peer registry only.
    /// `Some` for full adoption with transport-owned dispatch.
    dispatch_handle: Option<tokio::task::JoinHandle<()>>,
    route_publisher_handle: Option<tokio::task::JoinHandle<()>>,
    route_publisher_shutdown_tx: Option<mpsc::Sender<()>>,
    /// Our own routes (plan C7.9d); `None` in outbound-only adoption,
    /// where the host owns its routes.
    own_routes: Option<
        Arc<
            rekindle_protocol::own_routes::OwnRoutes<
                rekindle_protocol::own_routes::VeilidRouteAllocator,
            >,
        >,
    >,
    peer_registry: Arc<parking_lot::RwLock<PeerRegistry>>,
    shared_state: Arc<SharedState>,
    /// The process's importer of peers' private routes (plan C7.3).
    route_imports: Arc<rekindle_protocol::dht::route_imports::RouteImports>,
    /// The process's closer of ended sessions' records (plan C7.6i).
    record_closer: Arc<rekindle_protocol::dht::pool::RecordCloser>,
}

impl TransportNode {
    pub fn sender(&self) -> Sender {
        Sender::new(self.api.clone())
    }

    pub fn caller(&self) -> Caller {
        Caller::new(
            self.api.clone(),
            Arc::clone(&self.config),
            Arc::clone(&self.route_imports),
        )
    }

    /// Our own routes, when this transport owns them (not in
    /// outbound-only adoption).
    pub fn own_routes(
        &self,
    ) -> Option<
        Arc<
            rekindle_protocol::own_routes::OwnRoutes<
                rekindle_protocol::own_routes::VeilidRouteAllocator,
            >,
        >,
    > {
        self.own_routes.clone()
    }

    /// Our personal (general) route blob, while one is live.
    pub fn personal_route_blob(&self) -> Option<Vec<u8>> {
        self.own_routes
            .as_ref()?
            .blob(rekindle_protocol::own_routes::RouteClass::General)
    }

    /// Our media-class route blob, while one is live. Never substituted
    /// by the personal route (plan C7.9c).
    pub fn media_route_blob(&self) -> Option<Vec<u8>> {
        self.own_routes
            .as_ref()?
            .blob(rekindle_protocol::own_routes::RouteClass::Media)
    }

    /// Unlock: want both route classes (plan C7.9d; nothing is allocated
    /// while locked).
    pub fn want_routes(&self) {
        if let Some(routes) = &self.own_routes {
            routes.want(rekindle_protocol::own_routes::RouteClass::General);
            routes.want(rekindle_protocol::own_routes::RouteClass::Media);
        }
    }

    /// Lock: release this session's routes, so no route outlives the
    /// unlock that published it.
    pub fn release_routes(&self) {
        if let Some(routes) = &self.own_routes {
            routes.release_all();
        }
    }

    /// The process's route importer: the only `import_remote_private_route`
    /// caller (plan C7.3, D4).
    pub fn route_imports(&self) -> Arc<rekindle_protocol::dht::route_imports::RouteImports> {
        Arc::clone(&self.route_imports)
    }

    /// The process's closer of ended sessions' records (plan C7.6i): every
    /// record pool of this node ends through it.
    pub fn record_closer(&self) -> Arc<rekindle_protocol::dht::pool::RecordCloser> {
        Arc::clone(&self.record_closer)
    }

    /// The unlocked session's record pool, when one is running.
    pub fn records(&self) -> Option<Arc<rekindle_protocol::dht::pool::RecordPool>> {
        self.shared_state.records()
    }

    /// The unlocked session's record pool.
    ///
    /// # Errors
    /// `NotStarted` when no session is unlocked.
    pub fn require_records(&self) -> Result<Arc<rekindle_protocol::dht::pool::RecordPool>> {
        self.shared_state
            .records()
            .ok_or(TransportError::NotStarted)
    }

    /// Start the session's record pool (unlock). Its Veilid calls run on
    /// its own scope, which outlives the unlock scope's shutdown so the
    /// pool can still close its records (plan C7.3).
    ///
    /// # Errors
    /// The routing context refused the DHT safety selection.
    pub fn start_records(&self) -> Result<Arc<rekindle_protocol::dht::pool::RecordPool>> {
        let rc = self
            .api
            .routing_context()
            .map_err(|e| TransportError::Internal(format!("routing context: {e}")))?;
        let scope = rekindle_lifecycle::SessionScope::new(
            "record pool",
            Arc::new(|task| tracing::error!(task, "record pool task panicked")),
        );
        let pool = rekindle_protocol::dht::pool::RecordPool::new(
            &rc,
            &self.config.safety.dht,
            scope,
            self.shared_state.subscribe_ready(),
            Arc::clone(&self.record_closer),
        )?;
        self.shared_state.set_records(Some(Arc::clone(&pool)));
        Ok(pool)
    }

    /// Install a pool the host built and runs itself (the desktop, whose
    /// adopted node has no dispatch loop to feed readiness), so the process
    /// has one pool (plan C7.3). `None` removes it.
    pub fn adopt_records(&self, pool: Option<Arc<rekindle_protocol::dht::pool::RecordPool>>) {
        self.shared_state.set_records(pool);
    }

    /// Begin lock (plan C7.6g): release every task waiting on the pool and
    /// refuse new calls, so the unlock scope stops at once. Calls in flight
    /// run on; held writes stay held.
    pub fn drain_records(&self) {
        if let Some(pool) = self.shared_state.records() {
            pool.drain();
        }
    }

    /// Admit the lock's own writes (the Offline status) on the drained pool,
    /// once the unlock's tasks stopped (plan C7.6g).
    pub fn admit_records_teardown(&self) {
        if let Some(pool) = self.shared_state.records() {
            pool.admit_teardown();
        }
    }

    /// End the session's records (lock): the node's closer closes them once
    /// the pool's calls in flight have finished, off the lock's path; no
    /// call is aborted (plan C7.6i, C7.6j).
    pub fn end_records(&self) {
        let Some(pool) = self.shared_state.records() else {
            return;
        };
        self.shared_state.set_records(None);
        let in_flight = pool.scope().len();
        let calls = pool.scope().running();
        let records = pool.end();
        tracing::info!(in_flight, ?calls, records, "record pool ended");
    }

    pub fn peers(&self) -> Arc<parking_lot::RwLock<PeerRegistry>> {
        Arc::clone(&self.peer_registry)
    }

    pub fn config(&self) -> &TransportConfig {
        &self.config
    }

    // ── Introspection (for CLI/TUI) ─────────────────────────────────

    /// Observable shared state (attachment, uptime, subscribers).
    pub fn shared(&self) -> &Arc<SharedState> {
        &self.shared_state
    }

    /// Whether the node is attached and public internet ready.
    pub fn is_ready(&self) -> bool {
        self.shared_state.is_attached() && self.shared_state.public_internet_ready()
    }

    /// Node uptime since `start()`.
    pub fn uptime(&self) -> std::time::Duration {
        self.shared_state.uptime()
    }

    /// Subscribe to transport notifications. Returns a receiver that gets
    /// a clone of every event the dispatch loop broadcasts. Multiple
    /// subscribers are supported.
    pub fn subscribe(&self) -> tokio::sync::mpsc::UnboundedReceiver<TransportNotification> {
        self.shared_state.subscribe()
    }

    /// Point-in-time snapshot of transport status.
    pub fn status_snapshot(&self) -> TransportSnapshot {
        let general = rekindle_protocol::own_routes::RouteClass::General;
        let route_age = self.own_routes.as_ref().and_then(|r| r.age(general));
        let peer_reg = self.peer_registry.read();
        TransportSnapshot {
            attachment: self.shared_state.attachment_state().to_string(),
            is_attached: self.shared_state.is_attached(),
            public_internet_ready: self.shared_state.public_internet_ready(),
            uptime_secs: self.shared_state.uptime().as_secs(),
            peer_count: peer_reg.route_count(),
            route_allocated: route_age.is_some(),
            route_age_secs: route_age.map(|d| d.as_secs()),
        }
    }

    /// Create a [`QueryEngine`](crate::query::QueryEngine) for high-level
    /// read operations. Requires a shared `MekCache` for message decryption
    /// and an unlocked session (its record pool).
    pub fn query(
        &self,
        mek_cache: Arc<parking_lot::RwLock<crate::crypto::mek::MekCache>>,
    ) -> Result<crate::query::QueryEngine> {
        Ok(crate::query::QueryEngine::new(
            self.require_records()?,
            mek_cache,
            Arc::clone(&self.peer_registry),
        ))
    }
}

/// Build a Veilid `RoutingContext` from a [`SafetyProfile`].
///
/// The mapping itself is `rekindle_protocol::dht::pool::safety_selection`,
/// the one profile-to-Veilid mapping (floor-clamped, tested).
/// Used by `TransportNode::dht()`, [`Sender`], and [`Caller`].
pub(crate) fn build_routing_context(
    api: &VeilidAPI,
    profile: &SafetyProfile,
) -> Result<RoutingContext> {
    let rc = api
        .routing_context()
        .map_err(|_| TransportError::NotStarted)?;

    // Every path is a Veilid Safe route — sender hidden behind an
    // ephemeral route id, never the node's real identity. There is no
    // `Unsafe` branch: it leaks the sender to the first relay and is
    // gated behind veilid-core's `footgun-nodeid-target` feature, which
    // we never enable. `hop_count` is floor-clamped to the anonymity floor so a
    // misconfigured profile can never route below 3-hop Tor-class.
    rc.with_safety(rekindle_protocol::dht::pool::safety_selection(profile))
        .map_err(|e| TransportError::Internal(format!("safety: {e}")))
}
