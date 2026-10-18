//! The transport node: senders, records, routes and dispatch over the
//! Veilid-free ports (plan C9.1). It is built from a [`PortSet`], started
//! by [`TransportNode::start`] or handed in by the host; only
//! `rekindle-protocol` implements the ports over Veilid.

use std::sync::Arc;

use rekindle_records::port::{PortSet, RecordPort, RouteClass, RoutePort, UnsentWrite};
use tokio::sync::mpsc;

use crate::config::TransportConfig;

use super::peer_registry::PeerRegistry;
use super::send::{Caller, Sender};
use crate::error::{Result, TransportError};
use crate::shared::{SharedState, TransportNotification, TransportSnapshot};

mod lifecycle;
mod resume;
mod routes;

/// The top-level transport node. Holds the ports and all subsystems. There
/// is exactly one of these per application lifetime.
pub struct TransportNode {
    ports: PortSet,
    config: Arc<TransportConfig>,
    shutdown_tx: mpsc::Sender<()>,
    /// `None` for outbound-only adoption (W16.9b) — host runs its own
    /// dispatch loop, transport provides senders/DHT/peer registry only.
    /// `Some` when this node dispatches its own events.
    dispatch_handle: Option<tokio::task::JoinHandle<()>>,
    route_publisher_handle: Option<tokio::task::JoinHandle<()>>,
    route_publisher_shutdown_tx: Option<mpsc::Sender<()>>,
    peer_registry: Arc<parking_lot::RwLock<PeerRegistry>>,
    shared_state: Arc<SharedState>,
}

impl TransportNode {
    pub fn sender(&self) -> Sender {
        Sender::new(
            Arc::clone(&self.ports.messages),
            Arc::clone(&self.ports.routes),
        )
    }

    pub fn caller(&self) -> Caller {
        Caller::new(
            Arc::clone(&self.ports.messages),
            Arc::clone(&self.ports.routes),
            Arc::clone(&self.config),
        )
    }

    /// Peers' routes and our own (plan C9.1).
    pub fn routes(&self) -> Arc<dyn RoutePort> {
        Arc::clone(&self.ports.routes)
    }

    /// Our personal (general) route blob, while one is live.
    pub fn personal_route_blob(&self) -> Option<Vec<u8>> {
        self.ports.routes.own_blob(RouteClass::General)
    }

    /// Our media-class route blob, while one is live. Never substituted
    /// by the personal route (plan C7.9c).
    pub fn media_route_blob(&self) -> Option<Vec<u8>> {
        self.ports.routes.own_blob(RouteClass::Media)
    }

    /// Unlock: want both route classes (plan C7.9d; nothing is allocated
    /// while locked).
    pub fn want_routes(&self) {
        self.ports.routes.want_own();
    }

    /// Lock: release this session's routes, so no route outlives the
    /// unlock that published it.
    pub fn release_routes(&self) {
        self.ports.routes.release_own();
    }

    /// The unlocked session's records, when a session is running.
    pub fn records(&self) -> Option<Arc<dyn RecordPort>> {
        self.shared_state.records()
    }

    /// The unlocked session's records.
    ///
    /// # Errors
    /// `NotStarted` when no session is unlocked.
    pub fn require_records(&self) -> Result<Arc<dyn RecordPort>> {
        self.shared_state
            .records()
            .ok_or(TransportError::NotStarted)
    }

    /// Start the session's records (unlock): one pool per unlock, whose
    /// calls outlive the unlock scope's shutdown so it can still close its
    /// records (plan C7.3).
    ///
    /// # Errors
    /// The node refused a session (not started, or the DHT safety
    /// selection was refused).
    pub fn start_records(&self) -> Result<Arc<dyn RecordPort>> {
        let pool = self.ports.records.start_session()?;
        self.shared_state.set_records(Some(Arc::clone(&pool)));
        Ok(pool)
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

    /// The writes the drained pool still holds, taken for the next unlock
    /// (lock, after the teardown's own writes): drained again first, so
    /// nothing is held after the take (plan C7.6h).
    #[must_use]
    pub fn take_unsent(&self) -> Vec<UnsentWrite> {
        self.shared_state.records().map_or_else(Vec::new, |pool| {
            pool.drain();
            pool.take_held()
        })
    }

    /// End the session's records (lock): the node's closer closes them once
    /// the pool's calls in flight have finished, off the lock's path; no
    /// call is aborted (plan C7.6i, C7.6j).
    pub fn end_records(&self) {
        if let Some(pool) = self.shared_state.records() {
            self.shared_state.set_records(None);
            pool.end();
        }
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
        let route_age = self.ports.routes.own_age(RouteClass::General);
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
