//! Transport node lifecycle — the sole owner of the Veilid connection.
//!
//! [`TransportNode`] encapsulates the entire Veilid API. It is created
//! once at application startup and shut down on exit. No other code in
//! the workspace touches `veilid_core` directly.

use std::sync::Arc;

use tokio::sync::mpsc;
use veilid_core::{RoutingContext, SafetySelection, SafetySpec, Sequencing, Stability, VeilidAPI};

use crate::config::{
    SafetyProfile, SequencingPreference, StabilityPreference, TransportConfig, ANONYMITY_HOP_FLOOR,
};

use super::dht::DhtStore;
use super::peer_registry::PeerRegistry;
use super::peer_route::RouteManager;
use super::send::{Caller, Sender};
use crate::error::{Result, TransportError};
use crate::shared::{SharedState, TransportNotification, TransportSnapshot};

mod keys;
mod lifecycle;
mod resume;
mod routes;
mod updates;

pub use keys::{deserialize_keypair, ed25519_to_keypair, serialize_keypair};
pub(crate) use routes::RouteAuthorityEvent;

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
    route_authority_handle: Option<tokio::task::JoinHandle<()>>,
    route_authority_shutdown_tx: Option<mpsc::Sender<()>>,
    route_manager: Arc<parking_lot::RwLock<RouteManager>>,
    peer_registry: Arc<parking_lot::RwLock<PeerRegistry>>,
    shared_state: Arc<SharedState>,
}

impl TransportNode {
    pub fn sender(&self) -> Sender {
        Sender::new(self.api.clone(), Arc::clone(&self.config))
    }

    pub fn caller(&self) -> Caller {
        Caller::new(self.api.clone(), Arc::clone(&self.config))
    }

    pub fn dht(&self) -> Result<DhtStore> {
        let rc = build_routing_context(&self.api, &self.config.safety.dht)?;
        Ok(DhtStore::new(rc))
    }

    pub fn routes(&self) -> Arc<parking_lot::RwLock<RouteManager>> {
        Arc::clone(&self.route_manager)
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
        let route_mgr = self.route_manager.read();
        let peer_reg = self.peer_registry.read();
        TransportSnapshot {
            attachment: self.shared_state.attachment_state().to_string(),
            is_attached: self.shared_state.is_attached(),
            public_internet_ready: self.shared_state.public_internet_ready(),
            uptime_secs: self.shared_state.uptime().as_secs(),
            peer_count: peer_reg.route_count(),
            route_allocated: route_mgr.has_route(),
            route_age_secs: route_mgr.route_age().map(|d| d.as_secs()),
        }
    }

    /// Create a [`QueryEngine`](crate::query::QueryEngine) for high-level
    /// read operations. Requires a shared `MekCache` for message decryption.
    pub fn query(
        &self,
        mek_cache: Arc<parking_lot::RwLock<crate::crypto::mek::MekCache>>,
    ) -> Result<crate::query::QueryEngine> {
        let dht = self.dht()?;
        Ok(crate::query::QueryEngine::new(
            dht,
            mek_cache,
            Arc::clone(&self.peer_registry),
        ))
    }
}

/// Build a Veilid `RoutingContext` from a [`SafetyProfile`].
///
/// Single source of truth for safety-profile-to-Veilid mapping.
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
    rc.with_safety(SafetySelection::Safe(SafetySpec {
        preferred_route: None,
        hop_count: profile.hop_count.max(ANONYMITY_HOP_FLOOR) as usize,
        stability: match profile.stability {
            StabilityPreference::LowLatency => Stability::LowLatency,
            StabilityPreference::Reliable => Stability::Reliable,
        },
        sequencing: map_sequencing(profile.sequencing),
    }))
    .map_err(|e| TransportError::Internal(format!("safety: {e}")))
}

pub(crate) fn map_sequencing(pref: SequencingPreference) -> Sequencing {
    match pref {
        SequencingPreference::NoPreference => Sequencing::PreferUnordered,
        SequencingPreference::PreferOrdered => Sequencing::PreferOrdered,
        SequencingPreference::EnsureOrdered => Sequencing::EnsureOrdered,
    }
}
