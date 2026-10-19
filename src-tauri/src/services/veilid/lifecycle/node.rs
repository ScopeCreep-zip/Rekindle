use std::sync::Arc;

use crate::state::{AppState, NodeHandle};
use tauri::Manager;
use tokio::sync::mpsc;
use veilid_core::VeilidUpdate;

/// Initialize the Veilid node (called once at app startup).
pub async fn initialize_node(
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<mpsc::Receiver<VeilidUpdate>, String> {
    // Created here, after the layout check at boot (`setup.rs`).
    let storage_dir = app_handle
        .state::<rekindle_db::paths::DataRoot>()
        .create_veilid_dir()
        .map_err(|e| format!("failed to create Veilid's storage directory: {e}"))?;

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
        friend_list_dht_key: None,
        mailbox_dht_key: None,
    };
    *state.node.write() = Some(node_handle);

    // Our routes. The transport wants them as soon as its ports are up, so
    // they are allocated at node-ready, before any login, and a login is
    // reachable at once (plan C7.9b, E1.1; owner: login and "Connected" are
    // paired).
    let own_routes = rekindle_route::own_routes::OwnRoutes::new(
        rekindle_protocol::own_routes::VeilidRouteAllocator::new(api.clone()),
        state.network_ready.subscribe(),
    );
    // The process's one route importer (plan C7.3): routes are imported
    // before login too (invites, inbound app_message).
    let route_imports = Arc::new(rekindle_protocol::dht::route_imports::RouteImports::new(
        api.clone(),
    ));

    // The desktop's ports (plan C9.1e), built once over the node it owns.
    // `lifecycle` is `None`: the desktop still starts and stops
    // `RekindleNode` itself until E5.7. The typed `RouteImports` and
    // `OwnRoutes` handles kept on `AppState` serve the direct Veilid sends
    // the E-ports move onto `AppMessagePort` (ADR 0014's desktop exception).
    let transport_config = rekindle_transport::config::TransportConfig {
        storage_dir: storage_dir.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let ports = rekindle_records::port::PortSet {
        records: Arc::new(rekindle_protocol::ports::VeilidRecords::new(
            api.clone(),
            transport_config.safety.dht.clone(),
            state.network_ready.subscribe(),
            rekindle_protocol::dht::pool::RecordCloser::new(),
        )),
        routes: Arc::new(rekindle_protocol::ports::VeilidRoutes::new(
            Arc::clone(&route_imports),
            Arc::clone(&own_routes),
        )),
        messages: Arc::new(rekindle_protocol::ports::VeilidMessages::new(api.clone())),
        lifecycle: None,
    };
    *state.own_routes.write() = Some(own_routes);
    *state.route_imports.write() = Some(route_imports);

    // W16.9b — an outbound-only TransportNode over the desktop's ports:
    // senders, record sessions, peer registry and the one route republisher
    // (over the Host's session projection), without consuming the host's
    // `update_rx` (the existing `lifecycle::dispatch::run_dispatch_loop`
    // keeps that) until E5.7.
    let tn = rekindle_transport::TransportNode::adopt_outbound(
        transport_config,
        ports,
        Arc::clone(&state.transport_session),
    );
    *state.transport.write() = Some(Arc::new(tn));
    tracing::info!("transport node adopted (outbound-only) on the desktop's ports");

    tracing::info!("rekindle node started and attached");
    Ok(update_rx)
}
