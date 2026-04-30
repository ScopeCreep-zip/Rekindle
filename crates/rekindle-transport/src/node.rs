//! Transport node lifecycle — the sole owner of the Veilid connection.
//!
//! [`TransportNode`] encapsulates the entire Veilid API. It is created
//! once at application startup and shut down on exit. No other code in
//! the workspace touches `veilid_core` directly.

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{info, warn};
use veilid_core::{
    RoutingContext, SafetySelection, SafetySpec, Sequencing, Stability,
    VeilidAPI, VeilidConfig, VeilidUpdate,
};

use crate::config::{SafetyProfile, SequencingPreference, StabilityPreference, TransportConfig};
use crate::dispatch;
use crate::dht::DhtStore;
use crate::error::{TransportError, Result};
use crate::handler::InboundHandler;
use crate::peer::PeerRegistry;
use crate::route::RouteManager;
use crate::send::{Sender, Caller};

/// The top-level transport node. Owns the Veilid API handle and all
/// subsystems. There is exactly one of these per application lifetime.
pub struct TransportNode {
    api: VeilidAPI,
    config: Arc<TransportConfig>,
    shutdown_tx: mpsc::Sender<()>,
    dispatch_handle: tokio::task::JoinHandle<()>,
    route_refresh_handle: Option<tokio::task::JoinHandle<()>>,
    route_refresh_shutdown_tx: Option<mpsc::Sender<()>>,
    route_manager: Arc<parking_lot::RwLock<RouteManager>>,
    peer_registry: Arc<parking_lot::RwLock<PeerRegistry>>,
}

impl TransportNode {
    /// Start a new transport node, attach to the Veilid network, and begin
    /// dispatching inbound events to `handler`.
    pub async fn start<H: InboundHandler>(
        config: TransportConfig,
        handler: Arc<H>,
    ) -> Result<Self> {
        info!(namespace = %config.namespace, "starting transport node");

        let veilid_config = VeilidConfig::new(
            &config.namespace,
            "com",
            "rekindle",
            Some(&config.storage_dir),
            None,
        );

        let (update_tx, update_rx) = mpsc::channel::<VeilidUpdate>(4096);
        let update_callback: veilid_core::UpdateCallback = Arc::new(move |update| {
            if let Err(e) = update_tx.try_send(update) {
                let label = match &e {
                    mpsc::error::TrySendError::Full(u) | mpsc::error::TrySendError::Closed(u) => {
                        veilid_update_label(u)
                    }
                };
                if label == "Other" {
                    tracing::debug!("veilid update channel full — dropped non-critical event");
                } else {
                    tracing::warn!(event = label, "veilid update channel full — dropped event");
                }
            }
        });

        let api = veilid_core::api_startup(update_callback, veilid_config)
            .await
            .map_err(|e| TransportError::AttachFailed {
                reason: format!("api_startup: {e}"),
            })?;

        api.attach().await.map_err(|e| TransportError::AttachFailed {
            reason: format!("attach: {e}"),
        })?;

        let config = Arc::new(config);
        let route_manager = Arc::new(parking_lot::RwLock::new(RouteManager::new()));
        let peer_registry = Arc::new(parking_lot::RwLock::new(PeerRegistry::new(
            config.route_cache_ttl_secs,
            config.circuit_breaker_threshold,
            config.circuit_breaker_cooldown_secs,
        )));

        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let dispatch_handle = {
            let h = Arc::clone(&handler);
            let c = Arc::clone(&config);
            let a = api.clone();
            tokio::spawn(dispatch::run_dispatch_loop(h, c, update_rx, shutdown_rx, a))
        };

        let (rr_tx, rr_rx) = mpsc::channel(1);
        let route_refresh_handle = {
            let a = api.clone();
            let rm = Arc::clone(&route_manager);
            let secs = config.route_refresh_secs;
            tokio::spawn(run_route_refresh_loop(a, rm, secs, rr_rx))
        };

        info!("transport node started");

        Ok(Self {
            api,
            config,
            shutdown_tx,
            dispatch_handle,
            route_refresh_handle: Some(route_refresh_handle),
            route_refresh_shutdown_tx: Some(rr_tx),
            route_manager,
            peer_registry,
        })
    }

    /// Graceful shutdown: signal all background tasks, detach, shut down Veilid.
    pub async fn shutdown(mut self) -> Result<()> {
        info!("transport node shutting down");

        if let Some(tx) = self.route_refresh_shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        if let Some(h) = self.route_refresh_handle.take() {
            let _ = h.await;
        }

        let _ = self.shutdown_tx.send(()).await;
        if let Err(e) = self.dispatch_handle.await {
            warn!(error = %e, "dispatch loop join failed");
        }

        {
            let rm = self.route_manager.read();
            if let Some(route_id) = rm.route_id() {
                let _ = self.api.release_private_route(route_id.clone());
            }
        }

        self.api.detach().await.map_err(|e| TransportError::ShutdownFailed {
            reason: format!("detach: {e}"),
        })?;
        self.api.shutdown().await;

        info!("transport node shutdown complete");
        Ok(())
    }

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

    pub async fn allocate_route(&self) -> Result<(String, Vec<u8>)> {
        let rb = self.api.new_private_route().await.map_err(|e| {
            TransportError::RouteAllocationFailed { reason: format!("{e}") }
        })?;
        let id_str = rb.route_id.to_string();
        let blob = rb.blob.clone();
        self.route_manager.write().set_route(rb.route_id, rb.blob);
        Ok((id_str, blob))
    }

    pub fn import_route(&self, route_blob: &[u8]) -> Result<crate::peer::PeerTarget> {
        let route_id = self.api.import_remote_private_route(route_blob.to_vec())
            .map_err(|e| TransportError::RouteImportFailed {
                peer: String::new(), reason: format!("{e}"),
            })?;
        Ok(crate::peer::PeerTarget { route_id })
    }

    #[allow(dead_code)]
    pub(crate) fn api(&self) -> &VeilidAPI {
        &self.api
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
    let rc = api.routing_context().map_err(|_| TransportError::NotStarted)?;

    if profile.hop_count == 0 {
        return rc
            .with_safety(SafetySelection::Unsafe(map_sequencing(profile.sequencing)))
            .map_err(|e| TransportError::Internal(format!("safety: {e}")));
    }

    rc.with_safety(SafetySelection::Safe(SafetySpec {
        preferred_route: None,
        hop_count: profile.hop_count as usize,
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
        SequencingPreference::NoPreference => Sequencing::NoPreference,
        SequencingPreference::PreferOrdered => Sequencing::PreferOrdered,
        SequencingPreference::EnsureOrdered => Sequencing::EnsureOrdered,
    }
}

fn veilid_update_label(update: &VeilidUpdate) -> &'static str {
    match update {
        VeilidUpdate::AppCall(_) => "AppCall",
        VeilidUpdate::AppMessage(_) => "AppMessage",
        VeilidUpdate::RouteChange(_) => "RouteChange",
        VeilidUpdate::Attachment(_) => "Attachment",
        VeilidUpdate::ValueChange(_) => "ValueChange",
        VeilidUpdate::Shutdown => "Shutdown",
        _ => "Other",
    }
}

/// Periodically re-allocate the private route before it expires.
async fn run_route_refresh_loop(
    api: VeilidAPI,
    route_manager: Arc<parking_lot::RwLock<RouteManager>>,
    interval_secs: u64,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
    interval.tick().await; // skip immediate first tick

    loop {
        tokio::select! {
            _ = interval.tick() => {
                {
                    let rm = route_manager.read();
                    if let Some(old_id) = rm.route_id() {
                        let _ = api.release_private_route(old_id.clone());
                    }
                }
                match api.new_private_route().await {
                    Ok(rb) => {
                        route_manager.write().set_route(rb.route_id, rb.blob);
                        tracing::debug!("private route refreshed");
                    }
                    Err(e) => {
                        route_manager.write().forget_route();
                        tracing::warn!(error = %e, "route refresh failed — retry next tick");
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("route refresh loop shutting down");
                break;
            }
        }
    }
}
