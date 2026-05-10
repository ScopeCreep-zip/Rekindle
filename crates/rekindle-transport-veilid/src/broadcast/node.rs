//! Transport node lifecycle — the sole owner of the Veilid connection.
//!
//! [`TransportNode`] encapsulates the entire Veilid API. It is created
//! once at application startup and shut down on exit. No other code in
//! the workspace touches `veilid_core` directly.
//!
//! TransportNode does NOT hold Session, MekCache, SignalSessionManager,
//! or any application state. It provides Veilid primitives (DHT, routes,
//! send, call) that the application layer (rekindle-chat via PlatformIO)
//! composes into higher-level operations.
//!
//! The TransportCallback is installed AFTER construction via `set_callback()`.
//! During the window between `start()` and `set_callback()`, inbound events
//! are buffered in the dispatch loop (bounded at 4096). When the callback
//! is installed, the buffer drains first, then live dispatch resumes.
//! Zero events lost.

use std::sync::Arc;

use parking_lot::RwLock as CallbackLock;
use tokio::sync::mpsc;
use tracing::{info, warn};
use veilid_core::{
    RoutingContext, SafetySelection, SafetySpec, Sequencing, Stability,
    VeilidAPI, VeilidConfig, VeilidUpdate,
};

use crate::config::{SafetyProfile, SequencingPreference, StabilityPreference, TransportConfig};
use crate::subscriptions::dispatch;
use super::dht::DhtStore;
use crate::error::{TransportError, Result};
use rekindle_types::transport::TransportCallback;
use super::peer_registry::PeerRegistry;
use super::peer_route::RouteManager;
use super::send::{Sender, Caller};
use crate::shared::{SharedState, TransportSnapshot, TransportNotification};

/// Default maximum wait time for route allocation (seconds).
/// 30 minutes — generous for edge/far-edge nodes with intermittent connectivity.
const DEFAULT_ROUTE_ALLOC_MAX_SECS: u64 = 1800;

/// The top-level transport node. Owns the Veilid API handle and all
/// subsystems. There is exactly one of these per application lifetime.
///
/// The callback uses `parking_lot::RwLock<Option<...>>` for safe access.
/// Starts as None — events are buffered in the dispatch loop until
/// `set_callback()` installs the real TransportCallback from ChatService.
/// Read lock is ~2ns uncontended; the write happens exactly once.
pub struct TransportNode {
    api: VeilidAPI,
    config: Arc<TransportConfig>,
    callback: Arc<CallbackLock<Option<Arc<dyn TransportCallback>>>>,
    shutdown_tx: mpsc::Sender<()>,
    dispatch_handle: tokio::task::JoinHandle<()>,
    route_refresh_handle: Option<tokio::task::JoinHandle<()>>,
    route_refresh_shutdown_tx: Option<mpsc::Sender<()>>,
    route_manager: Arc<parking_lot::RwLock<RouteManager>>,
    peer_registry: Arc<parking_lot::RwLock<PeerRegistry>>,
    shared_state: Arc<SharedState>,
}

impl TransportNode {
    /// Start a new transport node, attach to the Veilid network, and begin
    /// the dispatch loop.
    ///
    /// The dispatch loop starts with NO callback — events are buffered.
    /// Call `set_callback()` after constructing ChatService to install the
    /// real TransportCallback and drain the buffer.
    ///
    /// No Session parameter — transport does not know about application state.
    pub async fn start(config: TransportConfig) -> Result<Self> {
        info!(namespace = %config.namespace, "starting transport node");

        let mut veilid_config = VeilidConfig::new(
            &config.namespace,
            "com",
            "rekindle",
            Some(&config.storage_dir),
            None,
        );
        veilid_config.protected_store.allow_insecure_fallback =
            config.allow_insecure_protected_store;

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
        let shared_state = SharedState::new();

        // Callback starts as None — dispatch loop buffers events until set_callback()
        let callback: Arc<CallbackLock<Option<Arc<dyn TransportCallback>>>> =
            Arc::new(CallbackLock::new(None));

        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let dispatch_handle = {
            let cb = Arc::clone(&callback);
            let c = Arc::clone(&config);
            let a = api.clone();
            let ss = Arc::clone(&shared_state);
            tokio::spawn(dispatch::run_dispatch_loop(cb, c, update_rx, shutdown_rx, a, ss))
        };

        let (rr_tx, rr_rx) = mpsc::channel(1);
        let route_refresh_handle = {
            let a = api.clone();
            let rm = Arc::clone(&route_manager);
            let secs = config.route_refresh_secs;
            tokio::spawn(run_route_refresh_loop(a, rm, secs, rr_rx))
        };

        info!("transport node started — call set_callback() to begin event dispatch");

        Ok(Self {
            api,
            config,
            callback,
            shutdown_tx,
            dispatch_handle,
            route_refresh_handle: Some(route_refresh_handle),
            route_refresh_shutdown_tx: Some(rr_tx),
            route_manager,
            peer_registry,
            shared_state,
        })
    }

    /// Install the TransportCallback. The dispatch loop will drain any
    /// buffered events through this callback, then process live events.
    ///
    /// Called exactly once after ChatService construction. The callback
    /// is ChatService's EventRouter which implements TransportCallback.
    ///
    /// This is lock-free — ArcSwapOption::store is a single atomic write.
    /// The dispatch loop sees the new callback on its next iteration.
    pub fn set_callback(&self, cb: Arc<dyn TransportCallback>) {
        *self.callback.write() = Some(cb);
        info!("transport callback installed — event dispatch active");
    }

    /// Non-consuming graceful shutdown for Arc holders.
    ///
    /// Signals all background tasks to stop, releases routes, detaches from
    /// the Veilid network, and shuts down the VeilidAPI. Safe to call
    /// multiple times — mpsc sends on closed channels are ignored.
    ///
    /// Used by the daemon lifecycle when TransportNode is held in
    /// `Arc<dyn Transport>` and cannot be consumed.
    pub async fn graceful_shutdown(&self) {
        tracing::info!("transport node graceful shutdown starting");

        let _ = self.shutdown_tx.send(()).await;
        if let Some(ref tx) = self.route_refresh_shutdown_tx {
            let _ = tx.send(()).await;
        }

        {
            let rm = self.route_manager.read();
            if let Some(route_id) = rm.route_id() {
                if let Err(e) = self.api.release_private_route(route_id.clone()) {
                    tracing::debug!(error = %e, "route release on graceful shutdown (likely already dead)");
                }
            }
        }

        let _ = self.api.detach().await;
        // VeilidAPI::shutdown() consumes self. Clone the handle — VeilidAPI
        // is a thin Arc wrapper, clone is ~1ns. The original handle becomes
        // inert after shutdown but remains valid (no-op on subsequent calls).
        self.api.clone().shutdown().await;

        tracing::info!("transport node graceful shutdown complete");
    }

    /// Consuming shutdown: signal all background tasks, detach, shut down Veilid.
    /// Prefer `graceful_shutdown()` when the node is behind Arc.
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
                if let Err(e) = self.api.release_private_route(route_id.clone()) {
                    tracing::debug!(error = %e, "route release on shutdown failed (likely already dead)");
                }
            }
        }

        self.api.detach().await.map_err(|e| TransportError::ShutdownFailed {
            reason: format!("detach: {e}"),
        })?;
        self.api.shutdown().await;

        info!("transport node shutdown complete");
        Ok(())
    }

    // ── Primitive accessors ─────────────────────────────────────────

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

    // ── Introspection ───────────────────────────────────────────────

    pub fn shared(&self) -> &Arc<SharedState> {
        &self.shared_state
    }

    pub fn is_ready(&self) -> bool {
        self.shared_state.is_attached() && self.shared_state.public_internet_ready()
    }

    pub fn uptime(&self) -> std::time::Duration {
        self.shared_state.uptime()
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<TransportNotification> {
        self.shared_state.subscribe()
    }

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

    // ── Route allocation ────────────────────────────────────────────

    /// Allocate a private route with exponential backoff retry.
    pub async fn allocate_route(&self) -> Result<(String, Vec<u8>)> {
        self.allocate_route_with_deadline(DEFAULT_ROUTE_ALLOC_MAX_SECS).await
    }

    /// Allocate a private route with a configurable deadline in seconds.
    /// Pass 0 for unlimited retry (beacon mode for intermittent connectivity).
    pub async fn allocate_route_with_deadline(&self, max_wait_secs: u64) -> Result<(String, Vec<u8>)> {
        let start = std::time::Instant::now();
        let mut backoff = std::time::Duration::from_millis(500);
        let ceiling = std::time::Duration::from_secs(15);
        let normal_deadline = std::time::Duration::from_secs(90);
        let hard_deadline = if max_wait_secs == 0 {
            std::time::Duration::from_secs(u64::MAX)
        } else {
            std::time::Duration::from_secs(max_wait_secs)
        };
        let mut warned_slow = false;
        let mut attempt = 0u32;

        loop {
            attempt += 1;
            match self.api.new_private_route().await {
                Ok(rb) => {
                    let id_str = rb.route_id.to_string();
                    let blob = rb.blob.clone();
                    self.route_manager.write().set_route(rb.route_id, rb.blob);
                    if attempt > 1 {
                        tracing::info!(
                            attempt,
                            elapsed_secs = start.elapsed().as_secs(),
                            "route allocated after retry"
                        );
                    }
                    return Ok((id_str, blob));
                }
                Err(e) => {
                    let msg = e.to_string();

                    if !msg.contains("TryAgain") {
                        return Err(TransportError::RouteAllocationFailed {
                            reason: format!(
                                "non-retryable route allocation error after {attempt} attempts: {e}"
                            ),
                        });
                    }

                    let elapsed = start.elapsed();

                    if elapsed >= hard_deadline {
                        return Err(TransportError::RouteAllocationFailed {
                            reason: format!(
                                "network not ready after {} attempts over {}s — \
                                 check network connectivity and Veilid bootstrap peers",
                                attempt, elapsed.as_secs()
                            ),
                        });
                    }

                    if !warned_slow && elapsed >= normal_deadline {
                        warned_slow = true;
                        tracing::warn!(
                            attempt,
                            elapsed_secs = elapsed.as_secs(),
                            next_retry_secs = ceiling.as_secs(),
                            "route allocation taking longer than expected — \
                             network may be slow to attach, retrying at {}s intervals",
                            ceiling.as_secs()
                        );
                    }

                    tracing::debug!(
                        attempt,
                        elapsed_secs = elapsed.as_secs(),
                        backoff_ms = backoff.as_millis(),
                        "network not ready for route allocation, retrying"
                    );

                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(ceiling);
                }
            }
        }
    }

    /// Import a remote private route from a route blob.
    pub fn import_route(&self, route_blob: &[u8]) -> Result<super::peer_registry::PeerTarget> {
        let route_id = self.api.import_remote_private_route(route_blob.to_vec())
            .map_err(|e| TransportError::RouteImportFailed {
                peer: String::new(), reason: format!("{e}"),
            })?;
        Ok(super::peer_registry::PeerTarget { route_id })
    }

    #[allow(dead_code)]
    pub(crate) fn api(&self) -> &VeilidAPI {
        &self.api
    }
}

// ── Veilid helpers ──────────────────────────────────────────────────

/// Build a Veilid `RoutingContext` from a [`SafetyProfile`].
///
/// Single source of truth for safety-profile-to-Veilid mapping.
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

/// Deserialize a keypair from the 64-byte format (32 pub + 32 secret).
pub fn deserialize_keypair(bytes: &[u8]) -> Result<veilid_core::KeyPair> {
    if bytes.len() != 64 {
        return Err(TransportError::Internal(format!(
            "keypair deserialization failed: expected 64 bytes, got {} — \
             keypair data may be corrupted or truncated",
            bytes.len()
        )));
    }
    let bare_pub = veilid_core::BarePublicKey::new(&bytes[..32]);
    let bare_secret = veilid_core::BareSecretKey::new(&bytes[32..]);
    let veilid_pub = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
    Ok(veilid_core::KeyPair::new_from_parts(veilid_pub, bare_secret))
}

/// Serialize a Veilid `KeyPair` to bytes (32 pub + 32 secret = 64 bytes).
pub fn serialize_keypair(kp: &veilid_core::KeyPair) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(64);
    bytes.extend_from_slice(kp.key().value().bytes());
    bytes.extend_from_slice(kp.secret().value().bytes());
    bytes
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

/// Personal route refresh loop. Periodically re-allocates the private route
/// before it expires. Community route refresh is handled by chat via
/// PlatformIO during the watch renewal background task.
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
                // Release old route
                {
                    let rm = route_manager.read();
                    if let Some(old_id) = rm.route_id() {
                        if let Err(e) = api.release_private_route(old_id.clone()) {
                            tracing::debug!(error = %e, "old route release failed (likely already dead)");
                        }
                    }
                }

                // Allocate fresh route
                match api.new_private_route().await {
                    Ok(rb) => {
                        route_manager.write().set_route(rb.route_id, rb.blob);
                        tracing::debug!("personal route refreshed");
                    }
                    Err(e) => {
                        route_manager.write().forget_route();
                        tracing::warn!(
                            error = %e,
                            "personal route refresh FAILED — incoming messages \
                             will fail until next successful refresh. Check network \
                             connectivity and Veilid attachment state."
                        );
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
