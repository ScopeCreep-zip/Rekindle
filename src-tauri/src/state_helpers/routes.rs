//! Peer route cache and route-import accessors.
//!
//! A peer's latest route blob lives in one place, the timestamped
//! `routing_manager.peer_route_cache`. Turning a blob into a `RouteId` is
//! the process's one importer, `AppState.route_imports` (plan C7.6): it has
//! no TTL and releases a route only after a send to it failed.

use std::sync::Arc;
use std::time::Instant;

use crate::state::AppState;

use super::node::safe_api_and_routing_context;

/// Map a DHT record key to its owning friend.
pub fn friend_for_dht_key(state: &Arc<AppState>, dht_key: &str) -> Option<String> {
    state
        .dht_manager
        .read()
        .as_ref()
        .and_then(|mgr| mgr.friend_for_dht_key(dht_key).cloned())
}

/// Cache a route blob for a peer.
pub fn cache_peer_route(state: &Arc<AppState>, peer_key: &str, route_blob: Vec<u8>) {
    {
        let mut routing_mgr = state.routing_manager.write();
        if let Some(handle) = routing_mgr.as_mut() {
            handle.peer_route_cache.insert_at(
                peer_key.to_string(),
                route_blob.clone(),
                Instant::now(),
            );
        }
    }

    // 1:1-call healing: call signaling carries no route blobs, so the
    // friend-route machinery (profile subkey-6 watch, conversation
    // header sync, mailbox) is the ONLY source of fresh routes for DM
    // call media. If a call with this peer is live, push the fresh blob
    // into their roster slot so frames survive the peer's route
    // rotation.
    let in_call = state
        .active_calls
        .list_all()
        .iter()
        .any(|c| c.peer_pubkey == peer_key);
    if in_call {
        let transport = {
            let ve = state.voice_engine.lock();
            ve.as_ref()
                .filter(|h| h.community_id.is_none())
                .map(|h| h.transport.clone())
        };
        if let Some(transport) = transport {
            let peer = peer_key.to_string();
            crate::state_helpers::login_scope_or_closed(state).spawn_or_drop(
                "voice route refresh",
                async move {
                    transport
                        .lock()
                        .await
                        .refresh_peer_route(&peer, &route_blob);
                },
            );
        }
    }
}

/// Look up cached route blob for a peer.
pub fn cached_route_blob(state: &Arc<AppState>, peer_key: &str) -> Option<Vec<u8>> {
    {
        let mut routing_mgr = state.routing_manager.write();
        if let Some(handle) = routing_mgr.as_mut() {
            if let Some(cached) = handle.peer_route_cache.get(peer_key) {
                if !cached.is_stale_at(
                    Instant::now(),
                    rekindle_route::lifecycle::PEER_ROUTE_CACHE_MAX_AGE,
                ) {
                    return Some(cached.route_blob.clone());
                }
            }
            handle.peer_route_cache.remove(peer_key);
        }
    }

    invalidate_cached_peer_route(state, peer_key);
    None
}

/// Look up a peer's cached route, import its `RouteId`, and return it with the
/// `RoutingContext`. Invalidates the cached route on import failure and returns `None`.
pub fn try_import_peer_route(
    state: &Arc<AppState>,
    peer_key: &str,
) -> Option<(veilid_core::RouteId, veilid_core::RoutingContext)> {
    let (_, rc) = safe_api_and_routing_context(state)?;
    let blob = cached_route_blob(state, peer_key)?;
    match import_route_blob(state, &blob) {
        Ok(route_id) => Some((route_id, rc)),
        Err(e) => {
            tracing::debug!(
                to = %peer_key, error = %e, blob_len = blob.len(),
                "route import failed — invalidating cached route"
            );
            invalidate_cached_peer_route(state, peer_key);
            None
        }
    }
}

/// Forget a peer's cached route blob. The imported `RouteId` stays with the
/// importer, which Veilid expires on its own; a failed send forgets it
/// ([`route_send_failed`]).
pub fn invalidate_cached_peer_route(state: &Arc<AppState>, peer_key: &str) {
    let mut routing_mgr = state.routing_manager.write();
    if let Some(handle) = routing_mgr.as_mut() {
        handle.peer_route_cache.remove(peer_key);
    }
}

/// Evict stale peer route blobs; returns how many.
pub fn evict_stale_peer_routes(state: &Arc<AppState>) -> usize {
    let mut routing_mgr = state.routing_manager.write();
    routing_mgr
        .as_mut()
        .map(|handle| handle.peer_route_cache.evict_stale_at(Instant::now()).len())
        .unwrap_or_default()
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

/// Forget `route_id` when a protocol send to it failed because the route is
/// unusable ([`ProtocolError::RouteUnusable`]); any other failure keeps it.
///
/// [`ProtocolError::RouteUnusable`]: rekindle_protocol::ProtocolError::RouteUnusable
pub fn note_send_result<T>(
    state: &Arc<AppState>,
    route_id: &veilid_core::RouteId,
    result: Result<T, rekindle_protocol::ProtocolError>,
) -> Result<T, rekindle_protocol::ProtocolError> {
    if let Err(rekindle_protocol::ProtocolError::RouteUnusable(_)) = &result {
        route_send_failed(state, route_id);
    }
    result
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
    let mut routing_mgr = state.routing_manager.write();
    routing_mgr
        .as_mut()
        .map(|handle| handle.peer_route_cache.remove_by_blob(&blobs))
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
