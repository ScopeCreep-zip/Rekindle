//! Read-only helpers for extracting commonly-accessed fields from [`AppState`].
//!
//! Each function acquires a read lock, clones out the needed value(s), and
//! drops the guard immediately — safe to call before `.await` points.
//!
//! Write patterns are too varied for generic helpers and stay inline.

use std::sync::Arc;

use crate::state::{AppState, FriendState, FriendshipState, IdentityState, UserStatus};

// ── Identity ──────────────────────────────────────────────────────────

/// Current identity's public key, or error `"not logged in"`.
pub fn current_owner_key(state: &Arc<AppState>) -> Result<String, String> {
    state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .ok_or_else(|| "not logged in".to_string())
}

/// Current identity's public key, or empty string (for non-critical paths).
pub fn owner_key_or_default(state: &Arc<AppState>) -> String {
    state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default()
}

/// Clone the full identity state, or error `"not logged in"`.
pub fn current_identity(state: &Arc<AppState>) -> Result<IdentityState, String> {
    state
        .identity
        .read()
        .clone()
        .ok_or_else(|| "not logged in".to_string())
}

/// Current identity's display name, or empty string.
pub fn identity_display_name(state: &Arc<AppState>) -> String {
    state
        .identity
        .read()
        .as_ref()
        .map(|id| id.display_name.clone())
        .unwrap_or_default()
}

/// Current identity's status.
pub fn identity_status(state: &Arc<AppState>) -> Option<UserStatus> {
    state.identity.read().as_ref().map(|id| id.status)
}

// ── Node / Network ───────────────────────────────────────────────────

/// Routing context if node is attached. Returns `None` if not initialized
/// or not attached to the network.
pub fn routing_context(state: &Arc<AppState>) -> Option<veilid_core::RoutingContext> {
    let node = state.node.read();
    node.as_ref()
        .filter(|nh| nh.is_attached)
        .map(|nh| nh.routing_context.clone())
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

// ── Friends ──────────────────────────────────────────────────────────

/// Check if a friend has `Accepted` friendship state.
pub fn is_friend_accepted(state: &Arc<AppState>, public_key: &str) -> bool {
    state
        .friends
        .read()
        .get(public_key)
        .is_some_and(|f| f.friendship_state == FriendshipState::Accepted)
}

/// Check if a person is in the friends map at all.
pub fn is_friend(state: &Arc<AppState>, public_key: &str) -> bool {
    state.friends.read().contains_key(public_key)
}

/// Generic friend field extractor.
pub fn friend_field<T>(
    state: &Arc<AppState>,
    key: &str,
    f: impl FnOnce(&FriendState) -> Option<T>,
) -> Option<T> {
    state.friends.read().get(key).and_then(f)
}

/// Friend's DHT record key.
pub fn friend_dht_key(state: &Arc<AppState>, key: &str) -> Option<String> {
    friend_field(state, key, |f| f.dht_record_key.clone())
}

/// Friend's display name.
pub fn friend_display_name(state: &Arc<AppState>, key: &str) -> Option<String> {
    friend_field(state, key, |f| Some(f.display_name.clone()))
}

/// Friend's mailbox DHT key.
pub fn friend_mailbox_key(state: &Arc<AppState>, key: &str) -> Option<String> {
    friend_field(state, key, |f| f.mailbox_dht_key.clone())
}

/// Collect all accepted friend keys.
pub fn accepted_friend_keys(state: &Arc<AppState>) -> Vec<String> {
    state
        .friends
        .read()
        .values()
        .filter(|f| f.friendship_state == FriendshipState::Accepted)
        .map(|f| f.public_key.clone())
        .collect()
}

/// Collect friends with DHT record keys (for sync/presence).
pub fn friends_with_dht_keys(state: &Arc<AppState>) -> Vec<(String, String)> {
    state
        .friends
        .read()
        .values()
        .filter_map(|f| {
            f.dht_record_key
                .as_ref()
                .map(|k| (f.public_key.clone(), k.clone()))
        })
        .collect()
}

// ── DHT Manager ──────────────────────────────────────────────────────

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
    let api = veilid_api(state);
    let mut dht_mgr = state.dht_manager.write();
    if let (Some(api), Some(mgr)) = (api, dht_mgr.as_mut()) {
        mgr.manager.cache_route(&api, peer_key, route_blob);
    }
}

/// Look up cached route blob for a peer.
pub fn cached_route_blob(state: &Arc<AppState>, peer_key: &str) -> Option<Vec<u8>> {
    state
        .dht_manager
        .read()
        .as_ref()
        .and_then(|mgr| mgr.manager.get_cached_route(peer_key).cloned())
}

// ── Communities ──────────────────────────────────────────────────────

/// Get a community's server route blob.
pub fn community_server_route(state: &Arc<AppState>, id: &str) -> Option<Vec<u8>> {
    state
        .communities
        .read()
        .get(id)
        .and_then(|c| c.server_route_blob.clone())
}

/// Collect communities with DHT record keys (for sync).
pub fn communities_with_dht_keys(state: &Arc<AppState>) -> Vec<(String, String)> {
    state
        .communities
        .read()
        .values()
        .filter_map(|c| c.dht_record_key.as_ref().map(|k| (c.id.clone(), k.clone())))
        .collect()
}
