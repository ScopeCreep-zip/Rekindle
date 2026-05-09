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
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;
    let storage_dir = app_data_dir.join("veilid");
    std::fs::create_dir_all(&storage_dir)
        .map_err(|e| format!("failed to create veilid storage dir: {e}"))?;

    let config = rekindle_protocol::node::NodeConfig {
        storage_dir: storage_dir.to_string_lossy().into_owned(),
        app_namespace: "rekindle".into(),
        qualifier: "rekindle".into(),
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
        api: api.clone(),
        routing_context: routing_context.clone(),
        route_blob: None,
        profile_dht_key: None,
        profile_owner_keypair: None,
        friend_list_dht_key: None,
        friend_list_owner_keypair: None,
        account_dht_key: None,
        mailbox_dht_key: None,
    };
    *state.node.write() = Some(node_handle);

    let dht_handle = DHTManagerHandle::new(routing_context);
    *state.dht_manager.write() = Some(dht_handle);

    let routing_manager = rekindle_protocol::routing::RoutingManager::new(
        api,
        rekindle_protocol::routing::SafetyMode::default(),
    );
    *state.routing_manager.write() = Some(RoutingManagerHandle {
        manager: routing_manager,
        peer_route_cache: rekindle_route::cache::RouteCache::new(
            rekindle_route::lifecycle::ROUTE_REFRESH_INTERVAL,
        ),
        route_lifecycle: rekindle_route::lifecycle::RouteLifecycle::new(std::time::Instant::now()),
    });

    // W16.9b — adopt the running VeilidAPI into a TransportNode in
    // outbound-only mode. Provides Sender / Caller / DhtStore / peer
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
    *state.transport.write() = Some(Arc::new(tn));
    tracing::info!("transport node adopted (outbound-only) for W16 send paths");

    tracing::info!("rekindle node started and attached");
    Ok(update_rx)
}
