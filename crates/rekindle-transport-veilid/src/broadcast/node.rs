//! Transport node lifecycle — the sole owner of the Veilid connection
//! and all delivery subsystems.
//!
//! [`TransportNode`] encapsulates the entire Veilid API and constructs
//! all delivery subsystems (RouteResolver, DeliveryEngine, MeshManager,
//! BroadcastManager, TransferRegistry, BulkSender) from primitives at
//! construction time. No circular `Arc<Self>` dependencies. No lazy
//! installation. No callback RwLock.
//!
//! Inbound data flows through `mpsc::Sender<InboundEvent>` — the dispatch
//! loop sends typed events, the chat layer reads from the receiver returned
//! by `start()`. No `TransportCallback` trait.
//!
//! Construction order (each step has all deps from prior steps):
//! 1. VeilidAPI → attach
//! 2. PeerRegistry, RouteManager, SharedState (standalone)
//! 3. RouteResolver(PeerRegistry, VeilidAPI, Config)
//! 4. BroadcastManager(VeilidAPI, Config)
//! 5. DeliveryEngine(RouteResolver, BroadcastManager)
//! 6. MeshManager(RouteResolver, BroadcastManager)
//! 7. TransferRegistry, BulkSender
//! 8. (inbound_tx, inbound_rx) mpsc channel
//! 9. dispatch_loop spawned with ALL deps — no buffering, no lazy

use std::sync::Arc;

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
use super::peer_registry::PeerRegistry;
use super::peer_route::RouteManager;
use super::send::{Sender, Caller};
use crate::shared::{SharedState, TransportSnapshot, TransportNotification};
use rekindle_types::transport::InboundEvent;

const DEFAULT_ROUTE_ALLOC_MAX_SECS: u64 = 1800;

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
    resolver: Arc<crate::resolver::RouteResolver>,
    broadcast_mgr: Arc<super::BroadcastManager>,
    delivery: Arc<crate::delivery::DeliveryEngine>,
    mesh: Arc<crate::mesh_manager::MeshManager>,
    transfers: Arc<crate::bulk_transfer::TransferRegistry>,
    bulk_sender: Arc<crate::bulk_transfer::BulkSender>,
}

impl TransportNode {
    /// Start a new transport node, attach to the Veilid network, construct
    /// all delivery subsystems, and begin the dispatch loop.
    ///
    /// Returns `(Self, Receiver<InboundEvent>)`. The receiver goes to the
    /// chat layer which spawns a read loop. No callback trait needed.
    pub async fn start(config: TransportConfig) -> Result<(Self, mpsc::Receiver<InboundEvent>)> {
        info!(namespace = %config.namespace, "starting transport node");

        let mut veilid_config = VeilidConfig::new(
            &config.namespace, "com", "rekindle", Some(&config.storage_dir), None,
        );
        veilid_config.protected_store.allow_insecure_fallback = config.allow_insecure_protected_store;

        // ── VeilidNetworkConfig mapping ─────────────────────────────
        {
            let v = &config.veilid;
            let net = &mut veilid_config.network;
            if !v.tcp_listen_address.is_empty() { net.protocol.tcp.listen_address.clone_from(&v.tcp_listen_address); }
            net.protocol.tcp.listen = v.tcp_listen;
            net.protocol.tcp.connect = v.tcp_connect;
            net.protocol.tcp.max_connections = v.tcp_max_connections;
            if !v.udp_listen_address.is_empty() { net.protocol.udp.listen_address.clone_from(&v.udp_listen_address); }
            net.protocol.udp.enabled = v.udp_enabled;
            net.protocol.udp.socket_pool_size = v.udp_socket_pool_size;
            if !v.ws_listen_address.is_empty() { net.protocol.ws.listen_address.clone_from(&v.ws_listen_address); }
            net.protocol.ws.listen = v.ws_listen;
            net.protocol.ws.connect = v.ws_connect;
            net.protocol.ws.max_connections = v.ws_max_connections;
            net.protocol.ws.path.clone_from(&v.ws_path);
            if v.tcp_public_address.is_some() { net.protocol.tcp.public_address.clone_from(&v.tcp_public_address); }
            if v.udp_public_address.is_some() { net.protocol.udp.public_address.clone_from(&v.udp_public_address); }
            net.max_connections_per_ip4 = v.max_connections_per_ip4;
            net.max_connections_per_ip6_prefix = v.max_connections_per_ip6_prefix;
            net.max_connections_per_ip6_prefix_size = v.max_connections_per_ip6_prefix_size;
            net.max_connection_frequency_per_min = v.max_connection_frequency_per_min;
            net.client_allowlist_timeout_ms = v.client_allowlist_timeout_ms;
            net.reverse_connection_receipt_time_ms = v.reverse_connection_receipt_time_ms;
            net.hole_punch_receipt_time_ms = v.hole_punch_receipt_time_ms;
            net.connection_initial_timeout_ms = v.connection_initial_timeout_ms;
            net.connection_inactivity_timeout_ms = v.connection_inactivity_timeout_ms;
            net.upnp = v.upnp;
            net.detect_address_changes = v.detect_address_changes;
            net.restricted_nat_retries = v.restricted_nat_retries;
            net.privacy.require_inbound_relay = v.require_inbound_relay;
            net.network_key_password.clone_from(&v.network_key_password);
            if !v.bootstrap.is_empty() { net.routing_table.bootstrap.clone_from(&v.bootstrap); }
            if !v.bootstrap_keys.is_empty() {
                net.routing_table.bootstrap_keys = v.bootstrap_keys.iter()
                    .filter_map(|s| s.parse::<veilid_core::PublicKey>().ok()).collect();
            }
            net.routing_table.limit_over_attached = v.limit_over_attached;
            net.routing_table.limit_fully_attached = v.limit_fully_attached;
            net.routing_table.limit_attached_strong = v.limit_attached_strong;
            net.routing_table.limit_attached_good = v.limit_attached_good;
            net.routing_table.limit_attached_weak = v.limit_attached_weak;
            net.rpc.concurrency = v.rpc_concurrency;
            net.rpc.queue_size = v.rpc_queue_size;
            net.rpc.timeout_ms = v.rpc_timeout_ms;
            net.rpc.max_timestamp_behind_ms = v.rpc_max_timestamp_behind_ms;
            net.rpc.max_timestamp_ahead_ms = v.rpc_max_timestamp_ahead_ms;
            net.rpc.max_route_hop_count = v.rpc_max_route_hop_count;
            net.rpc.default_route_hop_count = v.rpc_default_route_hop_count;
            net.dht.max_find_node_count = v.dht_max_find_node_count;
            net.dht.resolve_node_timeout_ms = v.dht_resolve_node_timeout_ms;
            net.dht.resolve_node_count = v.dht_resolve_node_count;
            net.dht.resolve_node_fanout = v.dht_resolve_node_fanout;
            net.dht.get_value_timeout_ms = v.dht_get_value_timeout_ms;
            net.dht.set_value_timeout_ms = v.dht_set_value_timeout_ms;
            net.dht.min_peer_count = v.dht_min_peer_count;
            net.dht.min_peer_refresh_time_ms = v.dht_min_peer_refresh_time_ms;
            net.dht.validate_dial_info_receipt_time_ms = v.dht_validate_dial_info_receipt_time_ms;
            net.dht.local_subkey_cache_size = v.dht_local_subkey_cache_size;
            net.dht.local_max_subkey_cache_memory_mb = v.dht_local_max_subkey_cache_memory_mb;
            net.dht.remote_subkey_cache_size = v.dht_remote_subkey_cache_size;
            net.dht.public_watch_limit = v.dht_public_watch_limit;
            net.dht.member_watch_limit = v.dht_member_watch_limit;
            net.dht.max_watch_expiration_ms = v.dht_max_watch_expiration_ms;
            net.dht.public_transaction_limit = v.dht_public_transaction_limit;
            net.dht.member_transaction_limit = v.dht_member_transaction_limit;
            net.dht.remote_max_records = v.dht_remote_max_records;
            net.dht.remote_max_subkey_cache_memory_mb = v.dht_remote_max_subkey_cache_memory_mb;
            net.dht.remote_max_storage_space_mb = v.dht_remote_max_storage_space_mb;
            net.dht.set_value_fanout = v.dht_set_value_fanout;
            net.dht.get_value_fanout = v.dht_get_value_fanout;
            net.dht.set_value_count = v.dht_set_value_count;
            net.dht.get_value_count = v.dht_get_value_count;
            net.dht.consensus_width = v.dht_consensus_width;
            veilid_config.protected_store.always_use_insecure_storage = v.always_use_insecure_storage;
            veilid_config.protected_store.delete = v.protected_store_delete;
            veilid_config.protected_store.device_encryption_key_password
                .clone_from(&v.protected_store_device_encryption_key_password);
            veilid_config.table_store.delete = v.table_store_delete;
            veilid_config.block_store.delete = v.block_store_delete;
            if !v.disable_capabilities.is_empty() {
                veilid_config.capabilities.disable = v.disable_capabilities.iter()
                    .filter_map(|s| s.parse().ok()).collect();
            }
        }

        let (update_tx, update_rx) = mpsc::channel::<VeilidUpdate>(4096);
        let update_callback: veilid_core::UpdateCallback = Arc::new(move |update| {
            if let Err(e) = update_tx.try_send(update) {
                let label = match &e {
                    mpsc::error::TrySendError::Full(u) | mpsc::error::TrySendError::Closed(u) => veilid_update_label(u),
                };
                if label != "Other" { tracing::warn!(event = label, "veilid update dropped"); }
            }
        });

        let api = veilid_core::api_startup(update_callback, veilid_config).await
            .map_err(|e| TransportError::AttachFailed { reason: format!("api_startup: {e}") })?;
        api.attach().await.map_err(|e| TransportError::AttachFailed { reason: format!("attach: {e}") })?;

        let config = Arc::new(config);
        let route_manager = Arc::new(parking_lot::RwLock::new(RouteManager::new()));
        let peer_registry = Arc::new(parking_lot::RwLock::new(PeerRegistry::new(
            config.route_cache_ttl_secs, config.circuit_breaker_threshold, config.circuit_breaker_cooldown_secs,
        )));
        let shared_state = SharedState::new();

        // ── Construct delivery subsystems from primitives ────────────
        let resolver = Arc::new(crate::resolver::RouteResolver::new(
            Arc::clone(&peer_registry), api.clone(), Arc::clone(&config),
        ));
        let broadcast_mgr = Arc::new(super::BroadcastManager::new(api.clone(), Arc::clone(&config)));
        let delivery = Arc::new(crate::delivery::DeliveryEngine::new(
            Arc::clone(&resolver), Arc::clone(&broadcast_mgr), api.clone(), Arc::clone(&config),
        ));
        let mesh = Arc::new(crate::mesh_manager::MeshManager::new(
            Arc::clone(&resolver), Arc::clone(&broadcast_mgr), Arc::clone(&peer_registry),
        ));
        let download_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("rekindle").join("downloads");
        let transfers = Arc::new(crate::bulk_transfer::TransferRegistry::new(download_dir));
        let bulk_sender = Arc::new(crate::bulk_transfer::BulkSender::new(
            Arc::clone(&resolver), api.clone(), Arc::clone(&config),
        ));

        // ── Inbound event channel — replaces TransportCallback ──────
        let (inbound_tx, inbound_rx) = mpsc::channel::<InboundEvent>(4096);

        // ── Spawn dispatch loop with ALL deps at spawn time ─────────
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let dispatch_handle = {
            let itx = inbound_tx;
            let c = Arc::clone(&config);
            let a = api.clone();
            let ss = Arc::clone(&shared_state);
            let tr = Arc::clone(&transfers);
            let de = Arc::clone(&delivery);
            tokio::spawn(dispatch::run_dispatch_loop(itx, c, update_rx, shutdown_rx, a, ss, tr, de))
        };

        let (rr_tx, rr_rx) = mpsc::channel(1);
        let route_refresh_handle = {
            let a = api.clone();
            let rm = Arc::clone(&route_manager);
            let secs = config.route_refresh_secs;
            tokio::spawn(run_route_refresh_loop(a, rm, secs, rr_rx))
        };

        info!("transport node started — all subsystems constructed, dispatch active");

        Ok((Self {
            api, config, shutdown_tx, dispatch_handle,
            route_refresh_handle: Some(route_refresh_handle),
            route_refresh_shutdown_tx: Some(rr_tx),
            route_manager, peer_registry, shared_state,
            resolver, broadcast_mgr, delivery, mesh, transfers, bulk_sender,
        }, inbound_rx))
    }

    // ── Shutdown ────────────────────────────────────────────────────

    pub async fn graceful_shutdown(&self) {
        tracing::info!("transport node graceful shutdown starting");
        let _ = self.shutdown_tx.send(()).await;
        if let Some(ref tx) = self.route_refresh_shutdown_tx { let _ = tx.send(()).await; }
        {
            let rm = self.route_manager.read();
            if let Some(route_id) = rm.route_id() {
                let _ = self.api.release_private_route(route_id.clone());
            }
        }
        let _ = self.api.detach().await;
        self.api.clone().shutdown().await;
        tracing::info!("transport node graceful shutdown complete");
    }

    pub async fn shutdown(mut self) -> Result<()> {
        info!("transport node shutting down");
        if let Some(tx) = self.route_refresh_shutdown_tx.take() { let _ = tx.send(()).await; }
        if let Some(h) = self.route_refresh_handle.take() { let _ = h.await; }
        let _ = self.shutdown_tx.send(()).await;
        if let Err(e) = self.dispatch_handle.await { warn!(error = %e, "dispatch join failed"); }
        {
            let rm = self.route_manager.read();
            if let Some(route_id) = rm.route_id() {
                let _ = self.api.release_private_route(route_id.clone());
            }
        }
        self.api.detach().await.map_err(|e| TransportError::ShutdownFailed { reason: format!("detach: {e}") })?;
        self.api.shutdown().await;
        info!("transport node shutdown complete");
        Ok(())
    }

    // ── Primitive accessors ────────────────────────────────────────

    pub fn sender(&self) -> Sender { Sender::new(self.api.clone(), Arc::clone(&self.config)) }
    pub fn caller(&self) -> Caller { Caller::new(self.api.clone(), Arc::clone(&self.config)) }
    pub fn dht(&self) -> Result<DhtStore> {
        let rc = build_routing_context(&self.api, &self.config.safety.dht)?;
        Ok(DhtStore::new(rc))
    }
    pub fn routes(&self) -> Arc<parking_lot::RwLock<RouteManager>> { Arc::clone(&self.route_manager) }
    pub fn peers(&self) -> Arc<parking_lot::RwLock<PeerRegistry>> { Arc::clone(&self.peer_registry) }
    pub fn config(&self) -> &TransportConfig { &self.config }

    // ── Delivery subsystem accessors ───────────────────────────────

    pub fn resolver(&self) -> &Arc<crate::resolver::RouteResolver> { &self.resolver }
    pub fn broadcast_mgr(&self) -> &Arc<super::BroadcastManager> { &self.broadcast_mgr }
    pub fn delivery(&self) -> &Arc<crate::delivery::DeliveryEngine> { &self.delivery }
    pub fn mesh_manager(&self) -> &Arc<crate::mesh_manager::MeshManager> { &self.mesh }
    pub fn transfer_registry(&self) -> &Arc<crate::bulk_transfer::TransferRegistry> { &self.transfers }
    pub fn bulk_sender(&self) -> &Arc<crate::bulk_transfer::BulkSender> { &self.bulk_sender }

    // ── Introspection ──────────────────────────────────────────────

    pub fn shared(&self) -> &Arc<SharedState> { &self.shared_state }
    pub fn is_ready(&self) -> bool { self.shared_state.is_attached() && self.shared_state.public_internet_ready() }
    pub fn uptime(&self) -> std::time::Duration { self.shared_state.uptime() }
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<TransportNotification> { self.shared_state.subscribe() }

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

    // ── Route allocation ───────────────────────────────────────────

    pub async fn allocate_route(&self) -> Result<(String, Vec<u8>)> {
        self.allocate_route_with_deadline(DEFAULT_ROUTE_ALLOC_MAX_SECS).await
    }

    pub async fn allocate_route_with_deadline(&self, max_wait_secs: u64) -> Result<(String, Vec<u8>)> {
        let start = std::time::Instant::now();
        let mut backoff = std::time::Duration::from_millis(500);
        let ceiling = std::time::Duration::from_secs(15);
        let normal_deadline = std::time::Duration::from_secs(90);
        let hard_deadline = if max_wait_secs == 0 { std::time::Duration::from_secs(u64::MAX) } else { std::time::Duration::from_secs(max_wait_secs) };
        let mut warned_slow = false;
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            match self.api.new_private_route().await {
                Ok(rb) => {
                    let id_str = rb.route_id.to_string();
                    let blob = rb.blob.clone();
                    self.route_manager.write().set_route(rb.route_id, rb.blob);
                    if attempt > 1 { tracing::info!(attempt, elapsed_secs = start.elapsed().as_secs(), "route allocated after retry"); }
                    return Ok((id_str, blob));
                }
                Err(e) => {
                    if !e.to_string().contains("TryAgain") {
                        return Err(TransportError::RouteAllocationFailed { reason: format!("non-retryable after {attempt}: {e}") });
                    }
                    let elapsed = start.elapsed();
                    if elapsed >= hard_deadline {
                        return Err(TransportError::RouteAllocationFailed { reason: format!("not ready after {attempt} attempts over {}s", elapsed.as_secs()) });
                    }
                    if !warned_slow && elapsed >= normal_deadline {
                        warned_slow = true;
                        tracing::warn!(attempt, "route allocation slow — retrying at {}s intervals", ceiling.as_secs());
                    }
                    tracing::debug!(attempt, backoff_ms = backoff.as_millis(), "retrying route allocation");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(ceiling);
                }
            }
        }
    }

    pub fn import_route(&self, route_blob: &[u8]) -> Result<super::peer_registry::PeerTarget> {
        let route_id = self.api.import_remote_private_route(route_blob.to_vec())
            .map_err(|e| TransportError::RouteImportFailed { peer: String::new(), reason: format!("{e}") })?;
        Ok(super::peer_registry::PeerTarget { route_id })
    }

    #[allow(dead_code)]
    pub(crate) fn api(&self) -> &VeilidAPI { &self.api }
}

// ── Veilid helpers ──────────────────────────────────────────────────

pub(crate) fn build_routing_context(api: &VeilidAPI, profile: &SafetyProfile) -> Result<RoutingContext> {
    let rc = api.routing_context().map_err(|_| TransportError::NotStarted)?;
    if profile.hop_count == 0 {
        return rc.with_safety(SafetySelection::Unsafe(map_sequencing(profile.sequencing)))
            .map_err(|e| TransportError::Internal(format!("safety: {e}")));
    }
    rc.with_safety(SafetySelection::Safe(SafetySpec {
        preferred_route: None,
        hop_count: profile.hop_count as usize,
        stability: match profile.stability { StabilityPreference::LowLatency => Stability::LowLatency, StabilityPreference::Reliable => Stability::Reliable },
        sequencing: map_sequencing(profile.sequencing),
    })).map_err(|e| TransportError::Internal(format!("safety: {e}")))
}

pub(crate) fn map_sequencing(pref: SequencingPreference) -> Sequencing {
    match pref {
        SequencingPreference::NoPreference => Sequencing::NoPreference,
        SequencingPreference::PreferOrdered => Sequencing::PreferOrdered,
        SequencingPreference::EnsureOrdered => Sequencing::EnsureOrdered,
    }
}

pub fn deserialize_keypair(bytes: &[u8]) -> Result<veilid_core::KeyPair> {
    if bytes.len() != 64 { return Err(TransportError::Internal(format!("keypair wrong length: {} (expected 64)", bytes.len()))); }
    let bare_pub = veilid_core::BarePublicKey::new(&bytes[..32]);
    let bare_secret = veilid_core::BareSecretKey::new(&bytes[32..]);
    let veilid_pub = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
    Ok(veilid_core::KeyPair::new_from_parts(veilid_pub, bare_secret))
}

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

async fn run_route_refresh_loop(
    api: VeilidAPI,
    route_manager: Arc<parking_lot::RwLock<RouteManager>>,
    interval_secs: u64,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
    interval.tick().await;
    loop {
        tokio::select! {
            _ = interval.tick() => {
                { let rm = route_manager.read(); if let Some(old_id) = rm.route_id() { let _ = api.release_private_route(old_id.clone()); } }
                match api.new_private_route().await {
                    Ok(rb) => { route_manager.write().set_route(rb.route_id, rb.blob); tracing::debug!("personal route refreshed"); }
                    Err(e) => { route_manager.write().forget_route(); tracing::warn!(error = %e, "route refresh FAILED"); }
                }
            }
            _ = shutdown_rx.recv() => { tracing::info!("route refresh loop shutting down"); break; }
        }
    }
}
