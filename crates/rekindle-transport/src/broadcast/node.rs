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

/// Default maximum wait time for route allocation (seconds).
/// 30 minutes — generous for edge/far-edge nodes with intermittent connectivity.
/// Override via `allocate_route_with_deadline()` for specific use cases.
/// Pass 0 for unlimited retry (beacon mode).
const DEFAULT_ROUTE_ALLOC_MAX_SECS: u64 = 1800;
use crate::subscriptions::dispatch;
use super::dht::DhtStore;
use crate::error::{TransportError, Result};
use crate::handler::InboundHandler;
use super::peer_registry::PeerRegistry;
use super::peer_route::RouteManager;
use super::send::{Sender, Caller};
use crate::shared::{SharedState, TransportSnapshot, TransportNotification};

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
    shared_state: Arc<SharedState>,
}

impl TransportNode {
    /// Start a new transport node, attach to the Veilid network, and begin
    /// dispatching inbound events to `handler`.
    ///
    /// The `session` Arc is shared with the route refresh loop so community
    /// routes can be refreshed alongside the personal route. The session
    /// may be `None` at startup (no identity yet) — it gets populated
    /// during `IdentityCreate` or session load.
    pub async fn start<H: InboundHandler>(
        config: TransportConfig,
        handler: Arc<H>,
        session: Arc<parking_lot::RwLock<Option<crate::session::Session>>>,
    ) -> Result<Self> {
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

        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let dispatch_handle = {
            let h = Arc::clone(&handler);
            let c = Arc::clone(&config);
            let a = api.clone();
            let ss = Arc::clone(&shared_state);
            tokio::spawn(dispatch::run_dispatch_loop(h, c, update_rx, shutdown_rx, a, ss))
        };

        let (rr_tx, rr_rx) = mpsc::channel(1);
        let route_refresh_handle = {
            let a = api.clone();
            let rm = Arc::clone(&route_manager);
            let secs = config.route_refresh_secs;
            let sess = Arc::clone(&session);
            let cfg = Arc::clone(&config);
            tokio::spawn(run_route_refresh_loop(a, rm, secs, rr_rx, sess, cfg))
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
            shared_state,
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

        // Best-effort release of our allocated route before detach.
        // May fail if the route already died — that's expected and harmless.
        // Veilid's Drop impl on VeilidAPIInner calls api_shutdown which
        // cleans up the context, but explicit release is still correct
        // practice when the route is still alive.
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

    /// Resume operational state from a persisted session.
    ///
    /// This is the Rekindle bootstrap — the complement to `start()` which
    /// only bootstraps Veilid. Every time the node starts (CLI one-shot,
    /// TUI launch, daemon restart), this method must be called with the
    /// session loaded from disk. It:
    ///
    /// 1. Allocates a private route so peers can reach us
    /// 2. Reopens our profile DHT record and publishes the new route blob
    /// 3. Reopens our mailbox DHT record and publishes the new route blob
    /// 4. Reopens our friend list DHT record (readonly)
    /// 5. Reopens all community governance and registry records (readonly)
    ///
    /// Without this, the node is a blank Veilid peer with no Rekindle
    /// identity. DHT reads fail with "record not open", friend requests
    /// fail with "no route allocated", and the TUI shows empty data.
    ///
    /// Errors in individual steps are logged but don't fail the resume —
    /// a partially resumed node is better than no node. The caller can
    /// check `status_snapshot()` to see what succeeded.
    pub async fn resume(
        &self,
        session: &crate::session::Session,
        signing_key_bytes: &[u8; 32],
    ) -> Result<()> {
        info!("resuming session for {}", &session.identity.display_name);

        // Step 1: Allocate private route via broadcast primitive
        let route_blob = match crate::broadcast::route::allocate_personal(self).await {
            Ok((route_id, blob)) => {
                info!(route = route_id, "private route allocated");
                blob
            }
            Err(e) => {
                warn!(error = %e, "route allocation failed — incoming messages will fail");
                Vec::new()
            }
        };

        // Step 2: Reopen profile and publish route blob
        if route_blob.is_empty() {
            let _ = crate::broadcast::dht_writes::open_readonly(self, &session.identity.profile_dht_key).await;
            let _ = crate::broadcast::dht_writes::open_readonly(self, &session.identity.mailbox_dht_key).await;
        } else {
            if let Some(ref keypair_bytes) = session.identity.profile_keypair_bytes {
                if let Ok(kp) = deserialize_keypair(keypair_bytes) {
                    match crate::broadcast::dht_writes::open_writable(self, &session.identity.profile_dht_key, kp).await {
                        Ok(()) => {
                            let _ = crate::broadcast::dht_writes::set(
                                self, &session.identity.profile_dht_key,
                                crate::payload::dht_types::PROFILE_SUBKEY_ROUTE_BLOB,
                                route_blob.clone(), None,
                            ).await;
                            info!(key = session.identity.profile_dht_key.as_str(), "profile reopened + route published");
                        }
                        Err(e) => {
                            warn!(error = %e, "profile reopen failed — falling back to readonly");
                            let _ = crate::broadcast::dht_writes::open_readonly(self, &session.identity.profile_dht_key).await;
                        }
                    }
                }
            } else {
                let _ = crate::broadcast::dht_writes::open_readonly(self, &session.identity.profile_dht_key).await;
            }

            // Step 3: Reopen mailbox and publish route blob
            let identity_keypair = {
                let sk = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);
                let pk = sk.verifying_key();
                let bare_pub = veilid_core::BarePublicKey::new(&pk.to_bytes());
                let bare_secret = veilid_core::BareSecretKey::new(signing_key_bytes);
                let veilid_pub = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
                veilid_core::KeyPair::new_from_parts(veilid_pub, bare_secret)
            };
            match crate::broadcast::dht_writes::open_writable(self, &session.identity.mailbox_dht_key, identity_keypair).await {
                Ok(()) => {
                    let dht = self.dht()?;
                    let _ = dht.mailbox().update_route(&session.identity.mailbox_dht_key, &route_blob).await;
                    info!(key = session.identity.mailbox_dht_key.as_str(), "mailbox reopened + route published");
                }
                Err(e) => {
                    warn!(error = %e, "mailbox reopen failed");
                    let _ = crate::broadcast::dht_writes::open_readonly(self, &session.identity.mailbox_dht_key).await;
                }
            }
        }

        // Step 4: Reopen friend list
        if let Some(ref kp_bytes) = session.identity.friend_list_keypair_bytes {
            if let Ok(kp) = deserialize_keypair(kp_bytes) {
                match crate::broadcast::dht_writes::open_writable(self, &session.identity.friend_list_dht_key, kp).await {
                    Ok(()) => info!(key = session.identity.friend_list_dht_key.as_str(), "friend list reopened writable"),
                    Err(e) => {
                        warn!(error = %e, "friend list writable reopen failed, falling back to readonly");
                        let _ = crate::broadcast::dht_writes::open_readonly(self, &session.identity.friend_list_dht_key).await;
                    }
                }
            } else {
                let _ = crate::broadcast::dht_writes::open_readonly(self, &session.identity.friend_list_dht_key).await;
            }
        } else if let Err(e) = crate::broadcast::dht_writes::open_readonly(self, &session.identity.friend_list_dht_key).await {
            warn!(error = %e, "friend list reopen failed");
        } else {
            info!(key = session.identity.friend_list_dht_key.as_str(), "friend list reopened readonly (no keypair)");
        }

        // Step 5: Reopen community governance + registry records (readonly)
        for membership in session.communities.values() {
            if let Err(e) = crate::broadcast::dht_writes::open_readonly(self, &membership.governance_key).await {
                warn!(error = %e, community = membership.community_name.as_str(), "governance reopen failed");
            }
            if let Err(e) = crate::broadcast::dht_writes::open_readonly(self, &membership.registry_key).await {
                warn!(error = %e, community = membership.community_name.as_str(), "registry reopen failed");
            }
        }

        // Step 6: For operator communities, allocate community routes and publish to mailbox
        for membership in session.communities.values() {
            if !membership.is_operator || membership.community_mailbox_key.is_empty() {
                continue;
            }
            match crate::broadcast::route::allocate_community(self).await {
                Ok((_route_id, community_route_blob)) => {
                    match crate::broadcast::route::publish_to_community_mailbox(
                        self, &membership.community_mailbox_key, &community_route_blob,
                    ).await {
                        Ok(()) => info!(community = membership.community_name.as_str(), "community route refreshed"),
                        Err(e) => warn!(community = membership.community_name.as_str(), error = %e, "community route publish failed"),
                    }
                }
                Err(e) => warn!(community = membership.community_name.as_str(), error = %e, "community route allocation failed"),
            }
        }

        let community_count = session.communities.len();
        info!(communities = community_count, "session resumed");
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
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<TransportNotification> {
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
    /// read operations. Requires shared `MekCache` and `SignalSessionManager`
    /// for channel and DM message decryption respectively.
    pub fn query(
        &self,
        mek_cache: Arc<parking_lot::RwLock<crate::crypto::mek::MekCache>>,
        signal: Arc<parking_lot::RwLock<Option<crate::crypto::signal_session::SignalSessionManager>>>,
    ) -> Result<crate::query::QueryEngine> {
        let dht = self.dht()?;
        Ok(crate::query::QueryEngine::new(
            dht,
            mek_cache,
            Arc::clone(&self.peer_registry),
            signal,
        ))
    }

    /// Allocate a private route, retrying with exponential backoff until
    /// the network is ready or the deadline expires.
    ///
    /// Veilid returns `TryAgain` when the network hasn't reached a state
    /// where routes can be allocated (e.g., no PublicInternet network class).
    /// This is normal during startup — the Veilid node needs time to discover
    /// peers, establish NAT mappings, and confirm reachability.
    ///
    /// Retry strategy:
    /// - Initial backoff: 500ms (fast first retries for quick connections)
    /// - Exponential growth: 500ms → 1s → 2s → 4s → 8s → 15s (ceiling)
    /// - Ceiling: 15 seconds (prevents excessive gaps between attempts)
    /// - Normal deadline: 90 seconds (covers ~90% of network conditions)
    /// - After normal deadline: log advisory, continue retrying at ceiling
    /// - Maximum deadline: configurable, default 30 minutes (edge/far-edge)
    /// - Non-TryAgain errors: fail immediately (not a network readiness issue)
    ///
    /// The `max_wait_secs` parameter controls the hard deadline. Pass 0 for
    /// unlimited retry (beacon mode for intermittently connected nodes).
    pub async fn allocate_route(&self) -> Result<(String, Vec<u8>)> {
        self.allocate_route_with_deadline(DEFAULT_ROUTE_ALLOC_MAX_SECS).await
    }

    /// Allocate a private route with a configurable deadline in seconds.
    /// Pass 0 for unlimited retry (beacon mode).
    pub async fn allocate_route_with_deadline(&self, max_wait_secs: u64) -> Result<(String, Vec<u8>)> {
        let start = std::time::Instant::now();
        let mut backoff = std::time::Duration::from_millis(500);
        let ceiling = std::time::Duration::from_secs(15);
        let normal_deadline = std::time::Duration::from_secs(90);
        let hard_deadline = if max_wait_secs == 0 {
            std::time::Duration::from_secs(u64::MAX) // effectively unlimited
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

                    // Non-retryable errors fail immediately.
                    if !msg.contains("TryAgain") {
                        return Err(TransportError::RouteAllocationFailed {
                            reason: format!("{e}"),
                        });
                    }

                    let elapsed = start.elapsed();

                    // Hard deadline exceeded.
                    if elapsed >= hard_deadline {
                        return Err(TransportError::RouteAllocationFailed {
                            reason: format!(
                                "network not ready after {} attempts over {}s — \
                                 check network connectivity and Veilid bootstrap peers",
                                attempt, elapsed.as_secs()
                            ),
                        });
                    }

                    // Advisory after normal deadline — operation is taking longer than expected.
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

/// Deserialize a keypair from the 64-byte format used by `serialize_keypair`.
///
/// Format: public key bytes (32) + secret key bytes (32).
pub fn deserialize_keypair(bytes: &[u8]) -> Result<veilid_core::KeyPair> {
    if bytes.len() != 64 {
        return Err(TransportError::Internal(format!(
            "keypair bytes: expected 64, got {}", bytes.len()
        )));
    }
    let bare_pub = veilid_core::BarePublicKey::new(&bytes[..32]);
    let bare_secret = veilid_core::BareSecretKey::new(&bytes[32..]);
    let veilid_pub = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
    Ok(veilid_core::KeyPair::new_from_parts(veilid_pub, bare_secret))
}

/// Serialize a Veilid `KeyPair` to bytes for keyring storage.
/// Format: public key bytes (32) + secret key bytes (32) = 64 bytes.
pub fn serialize_keypair(kp: &veilid_core::KeyPair) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(64);
    bytes.extend_from_slice(kp.key().value().bytes());
    bytes.extend_from_slice(kp.secret().value().bytes());
    bytes
}

/// Convert an Ed25519 signing key to a Veilid `KeyPair`.
pub fn ed25519_to_keypair(signing_key: &ed25519_dalek::SigningKey) -> veilid_core::KeyPair {
    let pub_bytes = signing_key.verifying_key().to_bytes();
    let secret_bytes = signing_key.to_bytes();
    let bare_pub = veilid_core::BarePublicKey::new(&pub_bytes);
    let bare_secret = veilid_core::BareSecretKey::new(&secret_bytes);
    let veilid_pub = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
    veilid_core::KeyPair::new_from_parts(veilid_pub, bare_secret)
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

/// Periodically re-allocate routes before they expire.
///
/// Refreshes both the personal private route AND all community routes
/// for communities where this node is an operator. Community routes
/// are published to each community's mailbox DHT record so joiners
/// can reach the owner's daemon.
async fn run_route_refresh_loop(
    api: VeilidAPI,
    route_manager: Arc<parking_lot::RwLock<RouteManager>>,
    interval_secs: u64,
    mut shutdown_rx: mpsc::Receiver<()>,
    session: Arc<parking_lot::RwLock<Option<crate::session::Session>>>,
    config: Arc<TransportConfig>,
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
    interval.tick().await; // skip immediate first tick

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // ── Personal route refresh ─────────────────────────────
                {
                    let rm = route_manager.read();
                    if let Some(old_id) = rm.route_id() {
                        if let Err(e) = api.release_private_route(old_id.clone()) {
                            tracing::debug!(error = %e, "old route release failed (likely already dead)");
                        }
                    }
                }
                match api.new_private_route().await {
                    Ok(rb) => {
                        route_manager.write().set_route(rb.route_id, rb.blob);
                        tracing::debug!("personal route refreshed");
                    }
                    Err(e) => {
                        route_manager.write().forget_route();
                        tracing::warn!(error = %e, "personal route refresh failed — retry next tick");
                    }
                }

                // ── Community route refresh ────────────────────────────
                // For each community where we're an operator, allocate a
                // fresh route and publish it to the community mailbox.
                let operator_communities: Vec<(String, String)> = {
                    let guard = session.read();
                    guard.as_ref().map_or_else(Vec::new, |s| {
                        s.communities.values()
                            .filter(|m| m.is_operator && !m.community_mailbox_key.is_empty())
                            .map(|m| (m.community_name.clone(), m.community_mailbox_key.clone()))
                            .collect()
                    })
                };

                if !operator_communities.is_empty() {
                    let rc = match build_routing_context(&api, &config.safety.dht) {
                        Ok(rc) => rc,
                        Err(e) => {
                            tracing::warn!(error = %e, "cannot build routing context for community route refresh");
                            continue;
                        }
                    };
                    let dht = DhtStore::new(rc);

                    for (name, mailbox_key) in &operator_communities {
                        match api.new_private_route().await {
                            Ok(rb) => {
                                match dht.mailbox().update_community_route(mailbox_key, &rb.blob).await {
                                    Ok(()) => tracing::debug!(community = %name, "community route refreshed"),
                                    Err(e) => tracing::warn!(community = %name, error = %e, "community route publish failed"),
                                }
                            }
                            Err(e) => {
                                tracing::warn!(community = %name, error = %e, "community route allocation failed");
                            }
                        }
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
