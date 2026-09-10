//! DHT manager + peer route cache accessors.

use std::sync::Arc;
use std::time::Instant;

use crate::state::AppState;

use super::node::{safe_api_and_routing_context, veilid_api};

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
        let api = veilid_api(state);
        let mut dht_mgr = state.dht_manager.write();
        if let (Some(api), Some(mgr)) = (api, dht_mgr.as_mut()) {
            mgr.manager.cache_route(&api, peer_key, route_blob.clone());
            let mut routing_mgr = state.routing_manager.write();
            if let Some(handle) = routing_mgr.as_mut() {
                handle.peer_route_cache.insert_at(
                    peer_key.to_string(),
                    route_blob.clone(),
                    Instant::now(),
                );
            }
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
            tauri::async_runtime::spawn(async move {
                transport
                    .lock()
                    .await
                    .refresh_peer_route(&peer, &route_blob);
            });
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
    let (api, rc) = safe_api_and_routing_context(state)?;
    let mut dht_mgr = state.dht_manager.write();
    let mgr = dht_mgr.as_mut()?;
    let blob = mgr.manager.get_cached_route(peer_key)?.clone();
    match mgr.manager.get_or_import_route(&api, &blob) {
        Ok(route_id) => Some((route_id, rc)),
        Err(e) => {
            tracing::debug!(
                to = %peer_key, error = %e, blob_len = blob.len(),
                "route import failed — invalidating cached route"
            );
            mgr.manager.invalidate_route_for_peer(peer_key);
            let mut routing_mgr = state.routing_manager.write();
            if let Some(handle) = routing_mgr.as_mut() {
                handle.peer_route_cache.remove(peer_key);
            }
            None
        }
    }
}

/// Invalidate all cached route state for a peer across both route caches.
pub fn invalidate_cached_peer_route(state: &Arc<AppState>, peer_key: &str) {
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.manager.invalidate_route_for_peer(peer_key);
        }
    }
    {
        let mut routing_mgr = state.routing_manager.write();
        if let Some(handle) = routing_mgr.as_mut() {
            handle.peer_route_cache.remove(peer_key);
        }
    }
}

/// Evict stale peer routes from both the timestamped route cache and the imported-route cache.
pub fn evict_stale_peer_routes(state: &Arc<AppState>) -> usize {
    let stale_peers = {
        let mut routing_mgr = state.routing_manager.write();
        routing_mgr
            .as_mut()
            .map(|handle| handle.peer_route_cache.evict_stale_at(Instant::now()))
            .unwrap_or_default()
    };

    if stale_peers.is_empty() {
        return 0;
    }

    let mut dht_mgr = state.dht_manager.write();
    if let Some(mgr) = dht_mgr.as_mut() {
        for peer_key in &stale_peers {
            mgr.manager.invalidate_route_for_peer(peer_key);
        }
    }

    stale_peers.len()
}

/// Import a route blob via `DHTManager` cache (preferred) or raw `VeilidAPI` fallback.
///
/// Consolidates the repeated lock → match Some/None → `get_or_import_route` pattern.
/// Acquires and drops the `dht_manager` write lock synchronously.
pub fn import_route_blob(
    state: &Arc<AppState>,
    route_blob: &[u8],
) -> Result<veilid_core::RouteId, String> {
    let api = veilid_api(state).ok_or("Veilid not connected")?;
    let mut dht_mgr = state.dht_manager.write();
    match dht_mgr.as_mut() {
        Some(mgr) => mgr
            .manager
            .get_or_import_route(&api, route_blob)
            .map_err(|e| e.to_string()),
        None => api
            .import_remote_private_route(route_blob.to_vec())
            .map_err(|e| e.to_string()),
    }
}
