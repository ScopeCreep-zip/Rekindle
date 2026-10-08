use std::sync::Arc;

use crate::state::{AppState, DHTManagerHandle, NodeHandle, RoutingManagerHandle};
use tauri::Manager;
use tokio::sync::mpsc;
use veilid_core::VeilidUpdate;

/// Initialize the Veilid node (called once at app startup).
pub async fn initialize_node(
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<mpsc::Receiver<VeilidUpdate>, String> {
    // Created with the rest of the data root at boot (`setup.rs`).
    let storage_dir = app_handle.state::<rekindle_db::paths::DataRoot>().veilid();

    let config = rekindle_protocol::node::NodeConfig {
        storage_dir: storage_dir.to_string_lossy().into_owned(),
        app_namespace: "rekindle".into(),
        qualifier: "rekindle".into(),
        // Defaults preserve pre-0.5.7 behavior (UPnP off, 1-hop inbound
        // routes, file-backed ProtectedStore, DHT concurrency 16). Flip
        // protocols: docs/contributor/veilid-0.5.7-migration-plan.md.
        veilid: rekindle_types::config::VeilidStartupOptions::default(),
    };

    let mut node = rekindle_protocol::RekindleNode::start(config)
        .await
        .map_err(|e| format!("failed to start veilid node: {e}"))?;

    let update_rx = node.take_update_receiver();
    let api = node.api().clone();
    let routing_context = node.routing_context().clone();

    let node_handle = NodeHandle {
        attachment_state: "detached".to_string(),
        is_attached: false,
        public_internet_ready: false,
        reliable_peer_count: 0,
        live_peer_count: 0,
        estimated_network_size: 0,
        median_latency_us: 0,
        api: api.clone(),
        routing_context: routing_context.clone(),
        profile_dht_key: None,
        profile_owner_keypair: None,
        profile_lease: None,
        friend_list_dht_key: None,
        friend_list_owner_keypair: None,
        account_dht_key: None,
        mailbox_dht_key: None,
    };
    *state.node.write() = Some(node_handle);

    let dht_handle = DHTManagerHandle::default();
    *state.dht_manager.write() = Some(dht_handle);

    *state.routing_manager.write() = Some(RoutingManagerHandle {
        peer_route_cache: rekindle_route::cache::RouteCache::new(
            rekindle_route::lifecycle::PEER_ROUTE_CACHE_MAX_AGE,
        ),
    });
    // Our routes, allocated as soon as the node is ready, before any login,
    // so a login is reachable at once (plan C7.9b; owner: login and
    // "Connected" are paired).
    let own_routes = rekindle_protocol::own_routes::OwnRoutes::new(
        rekindle_protocol::own_routes::VeilidRouteAllocator::new(api.clone()),
        state.network_ready.subscribe(),
    );
    own_routes.want(rekindle_protocol::own_routes::RouteClass::General);
    own_routes.want(rekindle_protocol::own_routes::RouteClass::Media);
    *state.own_routes.write() = Some(own_routes);

    // W16.9b — adopt the running VeilidAPI into a TransportNode in
    // outbound-only mode. Provides Sender / Caller / record pool / peer
    // registry to W16.10's `operations::friend::send_friend_request`
    // (and future W16.10c/.10b/.10d migrations) without consuming the
    // host's `update_rx` (the existing `lifecycle::dispatch::run_dispatch_loop`
    // keeps that). The transport's own dispatch is dormant — incoming
    // events stay on the legacy code path until each flow migrates.
    let transport_session = Arc::clone(&state.transport_session);
    let transport_api = state
        .node
        .read()
        .as_ref()
        .map(|nh| nh.api.clone())
        .ok_or("node handle missing immediately after creation — internal bug")?;
    let transport_config = rekindle_transport::config::TransportConfig {
        storage_dir: storage_dir.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let tn = rekindle_transport::TransportNode::adopt_outbound(
        transport_config,
        transport_api,
        &transport_session,
    );
    // The process's one route importer is the transport node's (plan
    // C7.3): routes are imported before login too (invites, inbound
    // app_message).
    *state.route_imports.write() = Some(tn.route_imports());
    *state.transport.write() = Some(Arc::new(tn));
    tracing::info!("transport node adopted (outbound-only) for W16 send paths");

    tracing::info!("rekindle node started and attached");
    Ok(update_rx)
}
