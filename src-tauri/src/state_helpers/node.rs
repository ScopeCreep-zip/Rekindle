//! Node / network / routing-context accessors.

use std::sync::Arc;

use crate::state::AppState;

use super::safe_routing_context_from;

/// Tauri app handle (set during setup). Used by background services to emit events.
pub fn app_handle(state: &Arc<AppState>) -> Option<tauri::AppHandle> {
    state.app_handle.read().clone()
}

/// The `(AppHandle, Db)` pair every adapter constructor needs.
///
/// Fifteen `build_adapter` functions across `services/` and `commands/`
/// each re-spelled this: read the app handle out of `AppState`, then
/// pull `Db` off the handle's managed state. They also disagreed on
/// how — some used `app_handle.state::<Db>()`, which **panics** when
/// the pool is not managed, others `try_state()`, which does not. This
/// uses `try_state`, so a missing pool is an error every caller can
/// handle rather than a panic in some of them.
///
/// Returns `Option` so `Result`-flavoured callers can attach their own
/// message with `.ok_or(...)`.
pub fn app_context(state: &Arc<AppState>) -> Option<(tauri::AppHandle, rekindle_db::Db)> {
    let app_handle = state.app_handle.read().clone()?;
    let pool = state.db.current().ok()?;
    Some((app_handle, pool))
}

/// The node's plain routing context if attached. Private: it is the base
/// the safe context is built from, never used directly, since its default
/// safety selection is one hop (V19).
fn routing_context(state: &Arc<AppState>) -> Option<veilid_core::RoutingContext> {
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

/// The API and the plain routing context, the base of
/// [`safe_api_and_routing_context`].
fn api_and_routing_context(
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

/// The session's DHT record pool (plan C7), or an error before login
/// services have started it.
pub fn record_pool(
    state: &Arc<AppState>,
) -> Result<Arc<rekindle_protocol::dht::pool::RecordPool>, String> {
    state
        .record_pool
        .read()
        .clone()
        .ok_or_else(|| "record pool not running".to_string())
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
    let route_blob = state
        .own_routes
        .read()
        .as_ref()
        .and_then(|r| r.blob(rekindle_protocol::own_routes::RouteClass::General))
        .ok_or("route blob not set")?;
    let mailbox_key = nh
        .mailbox_dht_key
        .clone()
        .ok_or("mailbox DHT key not set")?;
    Ok((profile_key, route_blob, mailbox_key))
}

/// The node's own routes (plan C7.9a).
pub fn own_routes(
    state: &AppState,
) -> Option<
    Arc<
        rekindle_protocol::own_routes::OwnRoutes<
            rekindle_protocol::own_routes::VeilidRouteAllocator,
        >,
    >,
> {
    state.own_routes.read().clone()
}

/// Our general route blob: what peers import to message us.
pub fn our_route_blob(state: &Arc<AppState>) -> Option<Vec<u8>> {
    own_routes(state)?.blob(rekindle_protocol::own_routes::RouteClass::General)
}

/// Our media-class route blob: what peers import to send us voice and
/// video. Never substituted by the general route (plan C7.9c): without one,
/// media is unavailable and the window says so.
pub fn our_media_route_blob(state: &Arc<AppState>) -> Option<Vec<u8>> {
    own_routes(state)?.blob(rekindle_protocol::own_routes::RouteClass::Media)
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

/// The login session's scope (plan C4), or `None` while logged out.
pub fn login_scope(state: &AppState) -> Option<Arc<rekindle_lifecycle::SessionScope>> {
    state.login_scope.read().clone()
}

/// The login scope, or a closed one while logged out, so work spawned
/// with no session to own it is dropped.
pub fn login_scope_or_closed(state: &AppState) -> Arc<rekindle_lifecycle::SessionScope> {
    login_scope(state).unwrap_or_else(|| rekindle_lifecycle::SessionScope::closed("login"))
}

/// Run `fut` as a task of the login session; dropped (and logged) when
/// no session is running.
pub fn spawn_in_login<F>(state: &AppState, name: &'static str, fut: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    login_scope_or_closed(state).spawn_or_drop(name, fut);
}

/// Run a task of the login session that watches the session's token, so
/// it stops at its next safe point when the session ends; dropped (and
/// logged) when no session is running.
pub fn spawn_in_login_with_token<F, Fut>(state: &AppState, name: &'static str, task: F)
where
    F: FnOnce(tokio_util::sync::CancellationToken) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    login_scope_or_closed(state).spawn_with_token_or_drop(name, task);
}
