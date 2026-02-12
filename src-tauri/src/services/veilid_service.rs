use std::sync::Arc;

use tauri::{Emitter, Manager};
use tokio::sync::mpsc;
use veilid_core::VeilidUpdate;

use crate::channels::{NetworkStatusEvent, NotificationEvent};
use crate::db::DbPool;
use crate::state::{
    AppState, DHTManagerHandle, NodeHandle, RoutingManagerHandle,
};

/// Build and emit a `NetworkStatusEvent` from current `NodeHandle` state.
///
/// Called from any code path that changes attachment, readiness, or route status
/// so the frontend's `NetworkIndicator` updates instantly.
pub fn emit_network_status(app_handle: &tauri::AppHandle, state: &AppState) {
    let event = {
        let node = state.node.read();
        match node.as_ref() {
            Some(nh) => NetworkStatusEvent {
                attachment_state: nh.attachment_state.clone(),
                is_attached: nh.is_attached,
                public_internet_ready: nh.public_internet_ready,
                has_route: nh.route_blob.is_some(),
            },
            None => NetworkStatusEvent {
                attachment_state: "detached".to_string(),
                is_attached: false,
                public_internet_ready: false,
                has_route: false,
            },
        }
    };
    let _ = app_handle.emit("network-status", &event);
}

/// Start the Veilid event dispatch loop.
///
/// This is the heartbeat of the application. It receives real `VeilidUpdate`
/// events from the node's internal callback channel and routes them to
/// the appropriate service handler.
pub async fn start_dispatch_loop(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    mut update_rx: mpsc::Receiver<VeilidUpdate>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    tracing::info!("veilid dispatch loop started");

    loop {
        tokio::select! {
            Some(update) = update_rx.recv() => {
                handle_veilid_update(&app_handle, &state, update).await;
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("veilid dispatch loop shutting down");
                break;
            }
        }
    }
}

/// Route a single `VeilidUpdate` to the appropriate handler.
async fn handle_veilid_update(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    update: VeilidUpdate,
) {
    match update {
        VeilidUpdate::AppMessage(msg) => handle_app_message(app_handle, state, *msg).await,
        VeilidUpdate::AppCall(call) => handle_app_call(app_handle, state, *call).await,
        VeilidUpdate::ValueChange(change) => {
            handle_value_change(app_handle, state, *change).await;
        }
        VeilidUpdate::Attachment(attachment) => {
            handle_attachment(app_handle, state, &attachment);
        }
        VeilidUpdate::RouteChange(change) => {
            handle_route_change(app_handle, state, &change).await;
        }
        VeilidUpdate::Shutdown => {
            tracing::info!("veilid core shutdown event received");
        }
        // Log, Network, Config updates are informational
        _ => {}
    }
}

/// Handle an incoming `AppMessage` by routing it through the message service.
async fn handle_app_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    msg: veilid_core::VeilidAppMessage,
) {
    // Sender identification comes from the MessageEnvelope, not from Veilid transport
    // (sender() returns None when received via private route)
    let message = msg.message().to_vec();
    tracing::debug!(msg_len = message.len(), "app_message received");
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    super::message_service::handle_incoming_message(
        app_handle,
        state,
        pool.inner(),
        &message,
    )
    .await;
}

/// Handle an incoming `AppCall` — process the message, then reply with ACK.
async fn handle_app_call(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    call: veilid_core::VeilidAppCall,
) {
    let call_id = call.id();
    tracing::debug!(call_id = %call_id, "app_call received");

    // Route the call through message handling (same as app_message)
    // then reply with an acknowledgment
    let message = call.message().to_vec();
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    super::message_service::handle_incoming_message(
        app_handle,
        state,
        pool.inner(),
        &message,
    )
    .await;

    // Reply with ACK so the caller's app_call future resolves.
    // Clone the API handle outside the lock (parking_lot guards are !Send).
    let api = {
        let node = state.node.read();
        node.as_ref().map(|nh| nh.api.clone())
    };
    if let Some(api) = api {
        if let Err(e) = api.app_call_reply(call_id, b"ACK".to_vec()).await {
            tracing::warn!(error = %e, "failed to reply to app_call");
        }
    }
}

/// Handle a DHT `ValueChange` notification by forwarding to the presence service.
async fn handle_value_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    change: veilid_core::VeilidValueChange,
) {
    let key = change.key.to_string();
    let subkeys: Vec<u32> = change.subkeys.iter().collect();
    let value = change
        .value
        .as_ref()
        .map_or_else(Vec::new, |v| v.data().to_vec());
    tracing::debug!(key = %key, subkeys = ?subkeys, "DHT value changed");
    super::presence_service::handle_value_change(app_handle, state, &key, &subkeys, &value).await;
}

/// Handle a network attachment state change — update node state and notify the frontend.
fn handle_attachment(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    attachment: &veilid_core::VeilidStateAttachment,
) {
    let attached = attachment.state.is_attached();
    let public_internet_ready = attachment.public_internet_ready;
    let state_str = attachment.state.to_string();
    tracing::info!(
        state = %state_str,
        public_internet_ready,
        "network attachment changed"
    );
    {
        if let Some(ref mut node) = *state.node.write() {
            node.attachment_state = state_str;
            node.is_attached = attached;
            node.public_internet_ready = public_internet_ready;
        }
    }
    // Propagate readiness via watch channel — never loses signals, no TOCTOU race
    let _ = state.network_ready_tx.send(public_internet_ready);

    // Push structured event so the frontend's NetworkIndicator can react immediately
    emit_network_status(app_handle, state);

    let status = if attached { "connected" } else { "disconnected" };
    let notification = NotificationEvent::SystemAlert {
        title: "Network".to_string(),
        body: format!("Veilid network {status}"),
    };
    let _ = app_handle.emit("notification-event", &notification);
}

/// Handle a route change — re-allocate our private route if it died, and
/// invalidate cached peer routes.
async fn handle_route_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    change: &veilid_core::VeilidRouteChange,
) {
    tracing::debug!(
        dead_routes = change.dead_routes.len(),
        dead_remote_routes = change.dead_remote_routes.len(),
        "route change event"
    );

    // Check if our specific private route died (not just any route)
    let our_route_died = {
        let rm = state.routing_manager.read();
        rm.as_ref().is_some_and(|handle| {
            handle
                .manager
                .route_id()
                .is_some_and(|our_id| change.dead_routes.contains(&our_id))
        })
    };

    if our_route_died {
        reallocate_private_route(app_handle, state).await;
    }

    // Invalidate cached peer routes that died (selective — only affected peers)
    if !change.dead_remote_routes.is_empty() {
        let api = {
            let node = state.node.read();
            node.as_ref().map(|nh| nh.api.clone())
        };
        if let Some(api) = api {
            let mut dht_mgr = state.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                mgr.manager
                    .invalidate_dead_routes(&api, &change.dead_remote_routes);
            }
        }
    }
}

/// Release the old private route, allocate a new one, and re-publish it to DHT.
async fn reallocate_private_route(app_handle: &tauri::AppHandle, state: &Arc<AppState>) {
    // Release the old route while holding the lock, then drop
    // before any .await (parking_lot guards are !Send)
    {
        let mut rm = state.routing_manager.write();
        if let Some(ref mut handle) = *rm {
            let _ = handle.manager.release_private_route();
        }
    }

    // Clone the API handle (Arc-based) outside the lock
    let api = {
        let node = state.node.read();
        node.as_ref().map(|nh| nh.api.clone())
    };

    let Some(api) = api else {
        return;
    };

    match api.new_private_route().await {
        Ok(route_blob) => {
            // Store route info back in the routing manager
            {
                let mut rm = state.routing_manager.write();
                if let Some(ref mut handle) = *rm {
                    handle.manager.set_allocated_route(
                        route_blob.route_id.clone(),
                        route_blob.blob.clone(),
                    );
                }
            }
            // Also store on node handle
            if let Some(ref mut nh) = *state.node.write() {
                nh.route_blob = Some(route_blob.blob.clone());
            }
            // Notify the frontend immediately about the new route
            emit_network_status(app_handle, state);

            // Re-publish route blob to DHT profile subkey 6
            if let Err(e) =
                super::message_service::push_profile_update(state, 6, route_blob.blob).await
            {
                tracing::warn!(error = %e, "failed to re-publish route blob to DHT");
            }
            tracing::info!("re-allocated private route after route death");
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to re-allocate private route");
        }
    }
}

/// Initialize the Veilid node (called once at app startup).
///
/// Starts the real Veilid node, attaches to the P2P network, creates
/// a routing context, and stores all handles in `AppState`. Returns the
/// `VeilidUpdate` receiver for the dispatch loop.
///
/// The node lives for the entire app lifetime — user login/logout does NOT
/// restart the node. Only `shutdown_app()` (on app exit) shuts it down.
pub async fn initialize_node(
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<mpsc::Receiver<VeilidUpdate>, String> {
    // Determine storage directory inside the Tauri app data dir
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
    };

    // Start the real Veilid node (api_startup + attach + routing_context)
    let mut node = rekindle_protocol::RekindleNode::start(config)
        .await
        .map_err(|e| format!("failed to start veilid node: {e}"))?;

    // Take the VeilidUpdate receiver before storing the node's pieces
    let update_rx = node
        .take_update_receiver()
        .ok_or_else(|| "update receiver already taken".to_string())?;

    // Clone Arc-based handles before storing
    let api = node.api().clone();
    let routing_context = node.routing_context().clone();

    // Store NodeHandle in AppState
    // is_attached starts false — the dispatch loop will set it to true
    // when the first Attachment event with is_attached() arrives.
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
    };
    *state.node.write() = Some(node_handle);

    // Create and store DHTManager
    let dht_handle = DHTManagerHandle::new(routing_context);
    *state.dht_manager.write() = Some(dht_handle);

    // Create and store RoutingManager (route allocation is deferred to
    // spawn_dht_publish() which waits for the network to be ready first)
    let routing_manager = rekindle_protocol::routing::RoutingManager::new(
        api,
        rekindle_protocol::routing::SafetyMode::default(),
    );
    *state.routing_manager.write() = Some(RoutingManagerHandle {
        manager: routing_manager,
    });

    tracing::info!("rekindle node started and attached");
    Ok(update_rx)
}

/// Clean up user-specific state on logout without shutting down the Veilid node.
///
/// The node stays alive for the entire app lifetime. This function:
/// 1. Aborts user-specific background tasks (sync, game detection, DHT publish)
/// 2. Closes all tracked DHT records
/// 3. Releases the private route
/// 4. Clears user-specific mappings from the DHT manager (but keeps the manager alive)
/// 5. Clears identity, friends, communities, signal manager
///
/// Does NOT call `api.shutdown()` — the node continues running for re-login.
pub async fn logout_cleanup(app_handle: Option<&tauri::AppHandle>, state: &AppState) {
    // 1. Abort user-specific background tasks
    {
        let mut handles = state.background_handles.lock();
        for handle in handles.drain(..) {
            handle.abort();
        }
    }

    // 2. Close ALL open DHT records tracked during this session
    {
        let rc_and_keys = {
            let node = state.node.read();
            let rc = node.as_ref().map(|nh| nh.routing_context.clone());
            let keys: Vec<String> = {
                let dht_mgr = state.dht_manager.read();
                dht_mgr
                    .as_ref()
                    .map(|mgr| mgr.open_records.iter().cloned().collect())
                    .unwrap_or_default()
            };
            rc.map(|rc| (rc, keys))
        };
        if let Some((rc, keys)) = rc_and_keys {
            tracing::debug!(count = keys.len(), "closing open DHT records for logout");
            for key_str in &keys {
                if let Ok(record_key) = key_str.parse::<veilid_core::RecordKey>() {
                    if let Err(e) = rc.close_dht_record(record_key).await {
                        tracing::trace!(key = %key_str, error = %e, "close DHT record on logout");
                    }
                }
            }
        }
    }

    // 3. Release private route
    {
        let mut rm = state.routing_manager.write();
        if let Some(ref mut handle) = *rm {
            if let Err(e) = handle.manager.release_private_route() {
                tracing::warn!(error = %e, "failed to release private route during logout");
            }
        }
    }

    // 4. Clear user-specific state from DHT manager (keep manager alive for re-login)
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = *dht_mgr {
            mgr.dht_key_to_friend.clear();
            mgr.conversation_key_to_friend.clear();
            mgr.open_records.clear();
            mgr.manager.route_cache.clear();
            mgr.manager.imported_routes.clear();
            mgr.manager.route_id_to_pubkey.clear();
            mgr.manager.profile_key = None;
            mgr.manager.friend_list_key = None;
        }
    }

    // 5. Clear user-specific data from NodeHandle (keep node alive)
    {
        let mut node = state.node.write();
        if let Some(ref mut nh) = *node {
            nh.route_blob = None;
            nh.profile_dht_key = None;
            nh.profile_owner_keypair = None;
            nh.friend_list_dht_key = None;
            nh.friend_list_owner_keypair = None;
            nh.account_dht_key = None;
        }
    }

    // Notify the frontend that the route is gone
    if let Some(ah) = app_handle {
        emit_network_status(ah, state);
    }

    // 6. Clear identity/friends/communities/signal
    *state.identity.write() = None;
    state.friends.write().clear();
    state.communities.write().clear();
    *state.signal_manager.lock() = None;
    *state.identity_secret.lock() = None;

    // NOTE: Do NOT reset network_ready_tx here. The Veilid node is still alive
    // and attached — the network IS ready. Resetting to false would cause the
    // next login's spawn_dht_publish() to time out waiting for a readiness signal
    // that never arrives (no new Attachment event fires when the node is already attached).

    tracing::info!("logout cleanup complete — node still running");
}

/// Shutdown the Veilid node (called only on app exit).
///
/// Follows the veilid-server shutdown ordering:
/// 1. Signal dispatch loop shutdown
/// 2. Close remaining DHT records
/// 3. Release private route and clear managers
/// 4. `api.shutdown().await`
pub async fn shutdown_app(state: &AppState) {
    // 1. Abort all remaining background tasks
    {
        let mut handles = state.background_handles.lock();
        for handle in handles.drain(..) {
            handle.abort();
        }
    }

    // 2. Close ALL open DHT records tracked during this session
    {
        let rc_and_keys = {
            let node = state.node.read();
            let rc = node.as_ref().map(|nh| nh.routing_context.clone());
            let keys: Vec<String> = {
                let dht_mgr = state.dht_manager.read();
                dht_mgr
                    .as_ref()
                    .map(|mgr| mgr.open_records.iter().cloned().collect())
                    .unwrap_or_default()
            };
            rc.map(|rc| (rc, keys))
        };
        if let Some((rc, keys)) = rc_and_keys {
            tracing::debug!(count = keys.len(), "closing open DHT records for app exit");
            for key_str in &keys {
                if let Ok(record_key) = key_str.parse::<veilid_core::RecordKey>() {
                    if let Err(e) = rc.close_dht_record(record_key).await {
                        tracing::trace!(key = %key_str, error = %e, "close DHT record on app exit");
                    }
                }
            }
        }
    }

    // 3. Release private route and clear managers
    {
        let mut rm = state.routing_manager.write();
        if let Some(ref mut handle) = *rm {
            if let Err(e) = handle.manager.release_private_route() {
                tracing::warn!(error = %e, "failed to release private route during app exit");
            }
        }
        *rm = None;
    }
    *state.dht_manager.write() = None;

    // 4. Shutdown the Veilid API
    let api = {
        let mut node = state.node.write();
        node.take().map(|nh| nh.api)
    };
    if let Some(api) = api {
        api.shutdown().await;
    }

    tracing::info!("veilid node shut down");
}
