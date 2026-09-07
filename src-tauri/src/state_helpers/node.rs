//! Node / network / routing-context accessors.

use std::sync::Arc;

use crate::state::AppState;

use super::safe_routing_context_from;

/// Tauri app handle (set during setup). Used by background services to emit events.
pub fn app_handle(state: &Arc<AppState>) -> Option<tauri::AppHandle> {
    state.app_handle.read().clone()
}

/// The `(AppHandle, DbPool)` pair every adapter constructor needs.
///
/// Fifteen `build_adapter` functions across `services/` and `commands/`
/// each re-spelled this: read the app handle out of `AppState`, then
/// pull `DbPool` off the handle's managed state. They also disagreed on
/// how — some used `app_handle.state::<DbPool>()`, which **panics** when
/// the pool is not managed, others `try_state()`, which does not. This
/// uses `try_state`, so a missing pool is an error every caller can
/// handle rather than a panic in some of them.
///
/// Returns `Option` so `Result`-flavoured callers can attach their own
/// message with `.ok_or(...)`.
pub fn app_context(state: &Arc<AppState>) -> Option<(tauri::AppHandle, crate::db::DbPool)> {
    use tauri::Manager as _;
    let app_handle = state.app_handle.read().clone()?;
    let pool = app_handle.try_state::<crate::db::DbPool>()?.inner().clone();
    Some((app_handle, pool))
}

/// Routing context if node is attached. Returns `None` if not initialized
/// or not attached to the network.
pub fn routing_context(state: &Arc<AppState>) -> Option<veilid_core::RoutingContext> {
    let node = state.node.read();
    node.as_ref()
        .filter(|nh| nh.is_attached)
        .map(|nh| nh.routing_context.clone())
}

/// Routing context configured for safe community/chat transport.
pub fn safe_routing_context(state: &Arc<AppState>) -> Option<veilid_core::RoutingContext> {
    routing_context(state).and_then(safe_routing_context_from)
}

/// Veilid API handle. Returns `None` if the node is not initialized.
pub fn veilid_api(state: &Arc<AppState>) -> Option<veilid_core::VeilidAPI> {
    state.node.read().as_ref().map(|nh| nh.api.clone())
}

/// Both API + routing context together (common combo).
/// Returns `None` if node is not initialized or not attached.
pub fn api_and_routing_context(
    state: &Arc<AppState>,
) -> Option<(veilid_core::VeilidAPI, veilid_core::RoutingContext)> {
    let node = state.node.read();
    let nh = node.as_ref().filter(|nh| nh.is_attached)?;
    Some((nh.api.clone(), nh.routing_context.clone()))
}

/// API plus routing context configured for safe community/chat transport.
pub fn safe_api_and_routing_context(
    state: &Arc<AppState>,
) -> Option<(veilid_core::VeilidAPI, veilid_core::RoutingContext)> {
    let (api, rc) = api_and_routing_context(state)?;
    Some((api, safe_routing_context_from(rc)?))
}

/// Routing context, or error `"node not initialized"` / `"not attached"`.
pub fn require_routing_context(
    state: &Arc<AppState>,
) -> Result<veilid_core::RoutingContext, String> {
    let node = state.node.read();
    let nh = node.as_ref().ok_or("node not initialized")?;
    if !nh.is_attached {
        return Err("not attached to network".to_string());
    }
    Ok(nh.routing_context.clone())
}

/// Routing context configured for safe community/chat transport, or a descriptive error.
pub fn require_safe_routing_context(
    state: &Arc<AppState>,
) -> Result<veilid_core::RoutingContext, String> {
    safe_routing_context(state).ok_or_else(|| "not attached to network".to_string())
}

/// Profile DHT info tuple: `(profile_dht_key, route_blob, mailbox_dht_key)`.
pub fn profile_dht_info(state: &Arc<AppState>) -> Result<(String, Vec<u8>, String), String> {
    let node = state.node.read();
    let nh = node.as_ref().ok_or("node not initialized")?;
    let profile_key = nh
        .profile_dht_key
        .clone()
        .ok_or("profile DHT key not set")?;
    let route_blob = nh.route_blob.clone().ok_or("route blob not set")?;
    let mailbox_key = nh
        .mailbox_dht_key
        .clone()
        .ok_or("mailbox DHT key not set")?;
    Ok((profile_key, route_blob, mailbox_key))
}

/// Route blob for our private route.
pub fn our_route_blob(state: &Arc<AppState>) -> Option<Vec<u8>> {
    state
        .node
        .read()
        .as_ref()
        .and_then(|nh| nh.route_blob.clone())
}

/// Friend list DHT key.
pub fn friend_list_dht_key(state: &Arc<AppState>) -> Option<String> {
    state
        .node
        .read()
        .as_ref()
        .and_then(|nh| nh.friend_list_dht_key.clone())
}

/// Friend list owner keypair.
pub fn friend_list_owner_keypair(state: &Arc<AppState>) -> Option<veilid_core::KeyPair> {
    state
        .node
        .read()
        .as_ref()
        .and_then(|nh| nh.friend_list_owner_keypair.clone())
}

/// Whether the node is attached to the network.
pub fn is_attached(state: &Arc<AppState>) -> bool {
    state.node.read().as_ref().is_some_and(|nh| nh.is_attached)
}

/// Track a tokio background task on `AppState.background_handles` so
/// logout/shutdown can abort it. Wraps the tokio handle in a tauri
/// runtime handle (the vec's element type).
///
/// THE implementation for the adapter `Deps` `register_background_handle`
/// methods — delegate here instead of re-spelling the wrap-and-push.
pub fn register_background_handle(state: &Arc<AppState>, handle: tokio::task::JoinHandle<()>) {
    let wrapped = tauri::async_runtime::spawn(async move {
        let _ = handle.await;
    });
    state.background_handles.lock().push(wrapped);
}
