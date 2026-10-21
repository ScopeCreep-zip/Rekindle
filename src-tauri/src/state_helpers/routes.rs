//! Peer route and route-import accessors.
//!
//! A peer's latest route blob lives in one place, the transport's
//! `PeerRegistry` (plan E2.1). Turning a blob into a `RouteId` is the
//! process's one importer, `AppState.route_imports` (plan C7.6), which the
//! transport's route port shares: it has no TTL and releases a route only
//! after a send to it failed.

use std::sync::Arc;

use crate::state::AppState;

use super::node::safe_api_and_routing_context;

/// A peer's route blob from the transport's one peer-route cache
/// (`PeerRegistry`, plan E2.1), while it is fresh.
pub fn peer_route(state: &AppState, peer_key: &str) -> Option<Vec<u8>> {
    let node = state.transport.read().clone()?;
    rekindle_transport::operations::dm::route::cached(&node, peer_key)
}

/// The Host's friend operations, for the unported callers of the 1:1 send
/// path (`operations::dm`).
///
/// # Errors
/// The Host has not started.
pub fn friend_host(
    state: &AppState,
) -> Result<Arc<dyn rekindle_transport::operations::friend::FriendHost>, String> {
    state
        .host
        .current()
        .map(|host| host.friend_host())
        .ok_or_else(|| "the host has not started".to_string())
}

/// A peer's name for an OS notice: our nickname for them, else their
/// key's fingerprint (the transport's `friend::label`), never the name
/// their own payload carried. With no Host there is no nickname to hold.
pub fn peer_label(state: &AppState, peer_key: &str) -> String {
    use rekindle_transport::operations::friend;
    friend_host(state).map_or_else(
        |_| friend::short_key(peer_key),
        |host| friend::label(host.as_ref(), peer_key),
    )
}

/// The `RouteId` for a route blob, through the process's one importer.
pub fn import_route_blob(
    state: &Arc<AppState>,
    route_blob: &[u8],
) -> Result<veilid_core::RouteId, String> {
    route_imports(state)?
        .get_or_import(route_blob)
        .map_err(|e| e.to_string())
}

/// A send to `route_id` failed with `NoConnection` or `InvalidTarget`: the
/// route is unusable, so the importer forgets it (never releases: Veilid
/// releases a dead remote route itself, plan C7.9e).
pub fn route_send_failed(state: &Arc<AppState>, route_id: &veilid_core::RouteId) {
    if let Ok(imports) = route_imports(state) {
        imports.invalidate_after_send_failure(route_id);
    }
}

/// Veilid declared these imported routes dead (`RouteChange`): the importer
/// forgets them, and every peer whose cached blob was one of them loses it,
/// so the next send re-fetches a fresh route. Returns those peers.
pub fn on_dead_remote_routes(state: &Arc<AppState>, dead: &[veilid_core::RouteId]) -> Vec<String> {
    let Ok(imports) = route_imports(state) else {
        return Vec::new();
    };
    let blobs = imports.on_dead_remote(dead);
    if blobs.is_empty() {
        return Vec::new();
    }
    state
        .transport
        .read()
        .as_ref()
        .map(|node| node.peers().write().invalidate_blobs(&blobs))
        .unwrap_or_default()
}

/// `app_call` a peer through its route blob: import (through the one
/// importer), call on the safe routing context, and release the route if
/// the call shows it unusable ([`route_send_failed`]).
pub async fn call_route_blob(
    state: &Arc<AppState>,
    route_blob: &[u8],
    payload: Vec<u8>,
) -> Result<Vec<u8>, String> {
    let route_id = import_route_blob(state, route_blob)?;
    let (_, rc) = safe_api_and_routing_context(state).ok_or("not attached")?;
    let result = rc
        .app_call(veilid_core::Target::RouteId(route_id.clone()), payload)
        .await;
    settle_route(state, &route_id, result).map_err(|e| format!("app_call: {e}"))
}

/// `app_message` a peer through its route blob; as [`call_route_blob`].
pub async fn message_route_blob(
    state: &Arc<AppState>,
    route_blob: &[u8],
    payload: Vec<u8>,
) -> Result<(), String> {
    let route_id = import_route_blob(state, route_blob)?;
    let (_, rc) = safe_api_and_routing_context(state).ok_or("not attached")?;
    let result = rc
        .app_message(veilid_core::Target::RouteId(route_id.clone()), payload)
        .await;
    settle_route(state, &route_id, result).map_err(|e| format!("app_message: {e}"))
}

/// Release `route_id` when a send to it shows it unusable.
fn settle_route<T>(
    state: &Arc<AppState>,
    route_id: &veilid_core::RouteId,
    result: Result<T, veilid_core::VeilidAPIError>,
) -> Result<T, veilid_core::VeilidAPIError> {
    if let Err(
        veilid_core::VeilidAPIError::NoConnection { .. }
        | veilid_core::VeilidAPIError::InvalidTarget { .. },
    ) = &result
    {
        route_send_failed(state, route_id);
    }
    result
}

/// The process's route importer (boot-scoped).
pub fn route_imports(
    state: &Arc<AppState>,
) -> Result<Arc<rekindle_protocol::dht::route_imports::RouteImports>, String> {
    state
        .route_imports
        .read()
        .clone()
        .ok_or_else(|| "Veilid not connected".to_string())
}
