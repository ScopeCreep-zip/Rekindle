//! State access helpers for extracting and storing commonly-accessed fields
//! from [`AppState`].
//!
//! Read helpers acquire a read lock, clone out the needed value(s), and drop
//! the guard immediately — safe to call before `.await` points. Write helpers
//! (`store_dht_record`, `track_open_records`, `cache_peer_route`, etc.) acquire
//! write locks with the same acquire-then-drop discipline.

use std::sync::Arc;

use crate::state::{AppState, FriendState, FriendshipState, IdentityState, UserStatus};

// ── DHT Record Storage ─────────────────────────────────────────────

/// Which type of DHT record is being stored on the node/manager handles.
///
/// `Profile` and `FriendList` carry an optional owner keypair (set on creation,
/// `None` on reopen). Account and Mailbox never carry a keypair.
pub enum DhtRecordType {
    Profile(Option<veilid_core::KeyPair>),
    FriendList(Option<veilid_core::KeyPair>),
    Account,
    Mailbox,
}

/// Store a DHT record key on `NodeHandle` and track it in `DHTManagerHandle`.
///
/// Acquires `node.write()` then `dht_manager.write()` sequentially (matching
/// the lock ordering used everywhere else). Each guard is dropped before the
/// next is acquired — safe with `parking_lot`'s `!Send` guards.
pub fn store_dht_record(state: &Arc<AppState>, key: &str, record_type: &DhtRecordType) {
    {
        let mut node = state.node.write();
        if let Some(ref mut nh) = *node {
            match &record_type {
                DhtRecordType::Profile(kp) => nh.set_profile_dht(key.to_string(), kp.clone()),
                DhtRecordType::FriendList(kp) => {
                    nh.set_friend_list_dht(key.to_string(), kp.clone());
                }
                DhtRecordType::Account => nh.set_account_dht(key.to_string()),
                DhtRecordType::Mailbox => nh.set_mailbox_dht(key.to_string()),
            }
        }
    }
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = *dht_mgr {
            match &record_type {
                DhtRecordType::Profile(_) => mgr.set_profile_key(key),
                DhtRecordType::FriendList(_) => mgr.set_friend_list_key(key),
                DhtRecordType::Account | DhtRecordType::Mailbox => {
                    mgr.track_open_record(key.to_string());
                }
            }
        }
    }
}

/// Track multiple DHT record keys as opened in this session.
///
/// Acquires `dht_manager.write()` once and inserts all keys. Useful for
/// compound records (account children, conversation children) where several
/// sub-records are created together.
pub fn track_open_records(state: &Arc<AppState>, keys: &[String]) {
    let mut dht_mgr = state.dht_manager.write();
    if let Some(ref mut mgr) = *dht_mgr {
        for k in keys {
            mgr.track_open_record(k.clone());
        }
    }
}

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

/// Look up a peer's cached route, import its `RouteId`, and return it with the
/// `RoutingContext`. Invalidates the cached route on import failure and returns `None`.
pub fn try_import_peer_route(
    state: &Arc<AppState>,
    peer_key: &str,
) -> Option<(veilid_core::RouteId, veilid_core::RoutingContext)> {
    let (api, rc) = api_and_routing_context(state)?;
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
            None
        }
    }
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

// ── Communities (write helpers) ───────────────────────────────────────

/// Replace the entire channel list for a community.
pub fn set_community_channels(
    state: &Arc<AppState>,
    community_id: &str,
    channels: Vec<crate::state::ChannelInfo>,
) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community.channels = channels;
    }
}

/// Append a single channel to a community's channel list.
pub fn push_community_channel(
    state: &Arc<AppState>,
    community_id: &str,
    channel: crate::state::ChannelInfo,
) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community.channels.push(channel);
    }
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
