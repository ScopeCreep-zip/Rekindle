//! State access helpers for extracting and storing commonly-accessed fields
//! from [`AppState`].
//!
//! Read helpers acquire a read lock, clone out the needed value(s), and drop
//! the guard immediately — safe to call before `.await` points. Write helpers
//! (`store_dht_record`, `track_open_records`, `cache_peer_route`, etc.) acquire
//! write locks with the same acquire-then-drop discipline.

use std::sync::Arc;
use std::time::Instant;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::{AppState, FriendState, FriendshipState, IdentityState, UserStatus};

fn safe_routing_context_from(
    routing_context: veilid_core::RoutingContext,
) -> Option<veilid_core::RoutingContext> {
    let spec = rekindle_route::contexts::RouteContextSpec::rc_safe();
    routing_context
        .with_safety(veilid_core::SafetySelection::Safe(
            veilid_core::SafetySpec {
                preferred_route: None,
                hop_count: spec.hop_count,
                stability: veilid_core::Stability::Reliable,
                sequencing: if spec.ordered {
                    veilid_core::Sequencing::PreferOrdered
                } else {
                    veilid_core::Sequencing::NoPreference
                },
            },
        ))
        .ok()
}

fn hex_to_id_16(hex_str: &str) -> [u8; 16] {
    let bytes = hex::decode(hex_str).unwrap_or_else(|_| vec![0u8; 16]);
    let mut arr = [0u8; 16];
    for (i, b) in bytes.iter().take(16).enumerate() {
        arr[i] = *b;
    }
    arr
}

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

/// Remove multiple DHT record keys from the global tracking set.
pub fn untrack_records(state: &Arc<AppState>, keys: &[String]) {
    let mut dht_mgr = state.dht_manager.write();
    if let Some(ref mut mgr) = *dht_mgr {
        for k in keys {
            mgr.untrack_record(k);
        }
    }
}

/// Collect all opened DHT record keys for a community from its `CommunityRecords`.
///
/// Returns the keys and marks the community's records as closed in state.
pub fn collect_and_clear_community_records(
    state: &Arc<AppState>,
    community_id: &str,
) -> Vec<String> {
    let mut communities = state.communities.write();
    let Some(cs) = communities.get_mut(community_id) else {
        return Vec::new();
    };
    let records = &mut cs.open_community_records;
    let mut keys = Vec::new();
    if let Some(ref k) = records.governance_key {
        keys.push(k.clone());
    }
    if let Some(ref k) = records.registry_key {
        keys.push(k.clone());
    }
    keys.append(&mut records.channel_keys);
    records.governance_key = None;
    records.registry_key = None;
    records.registry_writer = None;
    records.records_open = false;
    keys
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

/// Tauri app handle (set during setup). Used by background services to emit events.
pub fn app_handle(state: &Arc<AppState>) -> Option<tauri::AppHandle> {
    state.app_handle.read().clone()
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
        mgr.manager.cache_route(&api, peer_key, route_blob.clone());
        let mut routing_mgr = state.routing_manager.write();
        if let Some(handle) = routing_mgr.as_mut() {
            handle
                .peer_route_cache
                .insert_at(peer_key.to_string(), route_blob, Instant::now());
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
                    rekindle_route::lifecycle::ROUTE_REFRESH_INTERVAL,
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

// ── Circuit Breaker ──────────────────────────────────────────────────

/// Check if the circuit breaker is open (tripped) for a community.
///
/// Returns `true` if the community has >= 1 consecutive failure AND
/// the last failure was within the 45s cooldown window. Trips after a
/// single 8s timeout to prevent parallel RPCs from flooding a dead route.
pub fn is_circuit_open(state: &Arc<AppState>, community_id: &str) -> bool {
    let breakers = state.community_circuit_breakers.read();
    match breakers.get(community_id) {
        Some(cb) => cb.failure_count >= 1 && cb.tripped_at.elapsed().as_secs() < 45,
        None => false,
    }
}

/// Record a failure against a community's circuit breaker.
///
/// Increments the failure count and updates the timestamp.
pub fn trip_circuit_breaker(state: &Arc<AppState>, community_id: &str) {
    let mut breakers = state.community_circuit_breakers.write();
    let entry =
        breakers
            .entry(community_id.to_string())
            .or_insert(crate::state::CircuitBreakerState {
                tripped_at: std::time::Instant::now(),
                failure_count: 0,
            });
    entry.failure_count += 1;
    entry.tripped_at = std::time::Instant::now();
}

/// Reset the circuit breaker for a community on successful RPC.
pub fn reset_circuit_breaker(state: &Arc<AppState>, community_id: &str) {
    state
        .community_circuit_breakers
        .write()
        .remove(community_id);
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

/// Collect communities with governance record keys.
pub fn communities_with_governance_keys(state: &Arc<AppState>) -> Vec<(String, String)> {
    state
        .communities
        .read()
        .values()
        .filter_map(|c| c.governance_key.as_ref().map(|k| (c.id.clone(), k.clone())))
        .collect()
}

// ── v2.0 Governance State Helpers ──────────────────────────────────

/// Get a clone of the cached CRDT governance state for a community.
///
/// Returns `None` if the community doesn't exist or governance state isn't loaded yet.
pub fn governance_state(
    state: &Arc<AppState>,
    community_id: &str,
) -> Option<rekindle_governance::state::GovernanceState> {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|cs| cs.governance_state.clone())
}

/// Update the cached governance state for a community.
///
/// Also syncs `my_role_ids` from the CRDT role assignments so permission
/// checks and SQLite always reflect the latest governance (Bug #6 fix).
///
/// Called after:
/// - GovernanceNotify gossip messages (fast path)
/// - ValueChange DHT watch notifications (consistency path)
/// - Full CRDT merge on join or reconnect
pub fn set_governance_state(
    state: &Arc<AppState>,
    community_id: &str,
    gov_state: rekindle_governance::state::GovernanceState,
) {
    use crate::state::{CategoryInfo, ChannelInfo, ChannelType, RoleDefinition};

    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        // ── Sync my_role_ids from CRDT state ──
        let mut is_creator = false;
        if let Some(ref pk_hex) = cs.my_pseudonym_key {
            if let Ok(pk_bytes) = hex::decode(pk_hex) {
                if let Ok(arr) = <[u8; 32]>::try_from(pk_bytes.as_slice()) {
                    let pseudo = rekindle_types::id::PseudonymKey(arr);
                    if let Some(role_ids) = gov_state.role_assignments.get(&pseudo) {
                        cs.my_role_ids = role_ids.iter().map(role_id_to_legacy_u32).collect();
                        cs.my_role_ids.sort_unstable();
                    }
                    is_creator = gov_state.creator.as_ref() == Some(&pseudo);
                }
            }
        }

        // ── Sync channels from governance ChannelCreated entries ──
        // Build a map of existing unread counts to preserve them
        let existing_unreads: std::collections::HashMap<String, u32> = cs
            .channels
            .iter()
            .map(|ch| (ch.id.clone(), ch.unread_count))
            .collect();
        let existing_record_keys: std::collections::HashMap<String, Option<String>> = cs
            .channels
            .iter()
            .map(|ch| (ch.id.clone(), ch.message_record_key.clone()))
            .collect();
        let existing_notification_levels: std::collections::HashMap<String, String> = cs
            .channels
            .iter()
            .map(|ch| (ch.id.clone(), ch.notification_level.clone()))
            .collect();

        let mut channel_log_keys = std::collections::HashMap::new();
        let mut channels: Vec<ChannelInfo> = gov_state
            .channels
            .iter()
            .map(|(ch_id, ch)| {
                let id_hex = hex::encode(ch_id.0);
                if !ch.record_key.is_empty() {
                    channel_log_keys.insert(id_hex.clone(), ch.record_key.clone());
                }
                ChannelInfo {
                    unread_count: existing_unreads.get(&id_hex).copied().unwrap_or(0),
                    message_record_key: existing_record_keys
                        .get(&id_hex)
                        .cloned()
                        .flatten()
                        .or_else(|| {
                            if ch.record_key.is_empty() {
                                None
                            } else {
                                Some(ch.record_key.clone())
                            }
                        }),
                    id: id_hex.clone(),
                    name: ch.name.clone(),
                    channel_type: match ch.channel_type.as_str() {
                        "voice" => ChannelType::Voice,
                        "announcement" => ChannelType::Announcement,
                        "forum" => ChannelType::Forum,
                        "stage" => ChannelType::Stage,
                        "directory" => ChannelType::Directory,
                        "media" => ChannelType::Media,
                        "events" => ChannelType::Events,
                        "dm" => ChannelType::Dm,
                        _ => ChannelType::Text,
                    },
                    category_id: ch.category_id.map(|c| hex::encode(c.0)),
                    topic: ch.topic.clone().unwrap_or_default(),
                    slowmode_seconds: ch.slowmode_seconds,
                    nsfw: ch.nsfw.unwrap_or(false),
                    mek_generation: 0,
                    notification_level: existing_notification_levels
                        .get(&id_hex)
                        .cloned()
                        .unwrap_or_else(|| "all".to_string()),
                }
            })
            .collect();
        channels.sort_by(|a, b| {
            let a_pos = gov_state
                .channels
                .get(&rekindle_types::id::ChannelId(hex_to_id_16(&a.id)))
                .map_or(u32::MAX, |ch| ch.position);
            let b_pos = gov_state
                .channels
                .get(&rekindle_types::id::ChannelId(hex_to_id_16(&b.id)))
                .map_or(u32::MAX, |ch| ch.position);
            a_pos.cmp(&b_pos).then_with(|| a.name.cmp(&b.name))
        });
        cs.channels = channels;
        cs.channel_log_keys.clone_from(&channel_log_keys);
        cs.open_community_records.channel_keys = channel_log_keys.into_values().collect();

        // ── Sync roles from governance RoleDefinition entries ──
        cs.roles = gov_state
            .roles
            .iter()
            .map(|(rid, r)| RoleDefinition {
                id: role_id_to_legacy_u32(rid),
                name: r.name.clone(),
                color: r.color,
                permissions: r.permissions,
                position: r.position.cast_signed(),
                hoist: r.hoist,
                mentionable: r.mentionable,
                self_assignable: r.self_assignable,
            })
            .collect();
        cs.roles.sort_by_key(|role| role.position);

        // ── Sync categories from governance ──
        cs.categories = gov_state
            .categories
            .iter()
            .map(|(cat_id, cat)| CategoryInfo {
                id: hex::encode(cat_id.0),
                name: cat.name.clone(),
                sort_order: cat.position.cast_signed(),
            })
            .collect();
        cs.categories.sort_by_key(|category| category.sort_order);

        // ── Sync metadata (name, description) ──
        if let Some(ref meta) = gov_state.metadata {
            cs.name.clone_from(&meta.name);
            cs.description.clone_from(&meta.description);
        }

        // ── Detect creator → set my_role = "owner" ──
        if is_creator {
            cs.my_role = Some("owner".to_string());
        } else {
            cs.my_role = Some(crate::state::display_role_name(&cs.my_role_ids, &cs.roles));
        }

        // ── Sync MEK generation ──
        cs.mek_generation = gov_state.mek_generation;

        cs.governance_state = Some(gov_state);
    }
}

/// Persist the current merged governance snapshot into SQLite for restart hydration.
pub async fn persist_governance_snapshot_to_sqlite(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    lamport_clock: u64,
) -> Result<(), String> {
    #[derive(Clone)]
    struct ChannelRow {
        id: String,
        name: String,
        channel_type: String,
        sort_order: i64,
        category_id: Option<String>,
        topic: String,
        slowmode_seconds: i64,
        nsfw: i32,
        message_record_key: Option<String>,
        mek_generation: i64,
        log_key: Option<String>,
        my_sequence: i64,
    }

    #[derive(Clone)]
    struct RoleRow {
        role_id: i64,
        name: String,
        color: i64,
        permissions: i64,
        position: i64,
        hoist: i32,
        mentionable: i32,
        self_assignable: i32,
    }

    #[derive(Clone)]
    struct CategoryRow {
        id: String,
        name: String,
        sort_order: i64,
    }

    #[derive(Clone)]
    struct OverwriteRow {
        channel_id: String,
        target_type: String,
        target_id: String,
        allow: i64,
        deny: i64,
    }

    let owner_key = current_owner_key(state)?;
    let (
        community_id_owned,
        community_name,
        community_description,
        icon_hash,
        banner_hash,
        my_role,
        my_role_ids_json,
        mek_generation,
        channels,
        roles,
        categories,
        overwrites,
    ) = {
        let communities = state.communities.read();
        let community = communities.get(community_id).ok_or("community not found")?;
        let gov_state = community
            .governance_state
            .clone()
            .ok_or("governance state not loaded")?;
        let metadata = gov_state.metadata.clone();

        let mut channels: Vec<ChannelRow> = gov_state
            .channels
            .iter()
            .map(|(channel_id, channel)| {
                let channel_id_hex = hex::encode(channel_id.0);
                ChannelRow {
                    id: channel_id_hex.clone(),
                    name: channel.name.clone(),
                    channel_type: channel.channel_type.clone(),
                    sort_order: i64::from(channel.position),
                    category_id: channel
                        .category_id
                        .map(|category_id| hex::encode(category_id.0)),
                    topic: channel.topic.clone().unwrap_or_default(),
                    slowmode_seconds: i64::from(channel.slowmode_seconds.unwrap_or(0)),
                    nsfw: i32::from(channel.nsfw.unwrap_or(false)),
                    message_record_key: (!channel.record_key.is_empty())
                        .then(|| channel.record_key.clone()),
                    mek_generation: community.mek_generation.try_into().unwrap_or(i64::MAX),
                    log_key: (!channel.record_key.is_empty()).then(|| channel.record_key.clone()),
                    my_sequence: community
                        .channel_sequences
                        .get(&channel_id_hex)
                        .copied()
                        .unwrap_or(0)
                        .try_into()
                        .unwrap_or(i64::MAX),
                }
            })
            .collect();
        channels.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| a.name.cmp(&b.name))
        });

        let mut roles: Vec<RoleRow> = gov_state
            .roles
            .iter()
            .map(|(role_id, role)| RoleRow {
                role_id: i64::from(role_id_to_legacy_u32(role_id)),
                name: role.name.clone(),
                color: i64::from(role.color),
                permissions: role.permissions.cast_signed(),
                position: i64::from(role.position),
                hoist: i32::from(role.hoist),
                mentionable: i32::from(role.mentionable),
                self_assignable: i32::from(role.self_assignable),
            })
            .collect();
        roles.sort_by(|a, b| {
            a.position
                .cmp(&b.position)
                .then_with(|| a.name.cmp(&b.name))
        });

        let mut categories: Vec<CategoryRow> = gov_state
            .categories
            .iter()
            .map(|(category_id, category)| CategoryRow {
                id: hex::encode(category_id.0),
                name: category.name.clone(),
                sort_order: i64::from(category.position),
            })
            .collect();
        categories.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| a.name.cmp(&b.name))
        });

        let mut overwrites: Vec<OverwriteRow> = gov_state
            .overwrites
            .iter()
            .map(|((channel_id, target_id), overwrite)| OverwriteRow {
                channel_id: hex::encode(channel_id.0),
                target_type: overwrite.target_type.clone(),
                target_id: target_id.clone(),
                allow: overwrite.allow.cast_signed(),
                deny: overwrite.deny.cast_signed(),
            })
            .collect();
        overwrites.sort_by(|a, b| {
            a.channel_id
                .cmp(&b.channel_id)
                .then_with(|| a.target_type.cmp(&b.target_type))
                .then_with(|| a.target_id.cmp(&b.target_id))
        });

        let mut my_role_ids = community.my_role_ids.clone();
        my_role_ids.sort_unstable();

        (
            community_id.to_string(),
            community.name.clone(),
            community.description.clone(),
            metadata.as_ref().and_then(|meta| meta.icon_hash.clone()),
            metadata.as_ref().and_then(|meta| meta.banner_hash.clone()),
            community
                .my_role
                .clone()
                .unwrap_or_else(|| "member".to_string()),
            serde_json::to_string(&my_role_ids).unwrap_or_else(|_| "[0]".to_string()),
            community.mek_generation.try_into().unwrap_or(i64::MAX),
            channels,
            roles,
            categories,
            overwrites,
        )
    };

    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE communities SET name = ?1, description = ?2, icon_hash = ?3, banner_hash = ?4, \
             my_role = ?5, my_role_ids = ?6, mek_generation = ?7, lamport_clock = ?8 \
             WHERE owner_key = ?9 AND id = ?10",
            rusqlite::params![
                community_name,
                community_description,
                icon_hash,
                banner_hash,
                my_role,
                my_role_ids_json,
                mek_generation,
                lamport_clock.cast_signed(),
                owner_key,
                community_id_owned,
            ],
        )?;

        conn.execute(
            "DELETE FROM channels WHERE owner_key = ?1 AND community_id = ?2",
            rusqlite::params![owner_key, community_id_owned],
        )?;
        for channel in &channels {
            conn.execute(
                "INSERT INTO channels \
                 (owner_key, id, community_id, name, channel_type, sort_order, category_id, topic, slowmode_seconds, nsfw, message_record_key, mek_generation, log_key, my_sequence) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                rusqlite::params![
                    owner_key,
                    channel.id,
                    community_id_owned,
                    channel.name,
                    channel.channel_type,
                    channel.sort_order,
                    channel.category_id,
                    channel.topic,
                    channel.slowmode_seconds,
                    channel.nsfw,
                    channel.message_record_key,
                    channel.mek_generation,
                    channel.log_key,
                    channel.my_sequence,
                ],
            )?;
        }

        conn.execute(
            "DELETE FROM community_roles WHERE owner_key = ?1 AND community_id = ?2",
            rusqlite::params![owner_key, community_id_owned],
        )?;
        for role in &roles {
            conn.execute(
                "INSERT INTO community_roles \
                 (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable, self_assignable) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    owner_key,
                    community_id_owned,
                    role.role_id,
                    role.name,
                    role.color,
                    role.permissions,
                    role.position,
                    role.hoist,
                    role.mentionable,
                    role.self_assignable,
                ],
            )?;
        }

        conn.execute(
            "DELETE FROM community_categories WHERE owner_key = ?1 AND community_id = ?2",
            rusqlite::params![owner_key, community_id_owned],
        )?;
        for category in &categories {
            conn.execute(
                "INSERT INTO community_categories (owner_key, community_id, id, name, sort_order) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    owner_key,
                    community_id_owned,
                    category.id,
                    category.name,
                    category.sort_order,
                ],
            )?;
        }

        conn.execute(
            "DELETE FROM channel_overwrites WHERE owner_key = ?1 AND community_id = ?2",
            rusqlite::params![owner_key, community_id_owned],
        )?;
        for overwrite in &overwrites {
            conn.execute(
                "INSERT INTO channel_overwrites \
                 (owner_key, community_id, channel_id, target_type, target_id, allow, deny) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    owner_key,
                    community_id_owned,
                    overwrite.channel_id,
                    overwrite.target_type,
                    overwrite.target_id,
                    overwrite.allow,
                    overwrite.deny,
                ],
            )?;
        }

        Ok(())
    })
    .await
}

/// Get the governance record DHT key for a community.
pub fn governance_key(state: &Arc<AppState>, community_id: &str) -> Option<String> {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|cs| cs.governance_key.clone())
}

/// Get the community's current Lamport counter value.
pub fn lamport_counter(state: &Arc<AppState>, community_id: &str) -> u64 {
    state
        .communities
        .read()
        .get(community_id)
        .map_or(0, |cs| cs.lamport_counter)
}

/// Increment the Lamport counter for a community and return the new value.
/// Used on every message send.
pub fn increment_lamport(state: &Arc<AppState>, community_id: &str) -> u64 {
    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        let mut clock = rekindle_gossip::lamport::LamportClock::new(cs.lamport_counter);
        cs.lamport_counter = clock.increment();
        cs.lamport_counter
    } else {
        0
    }
}

/// Merge a received Lamport timestamp into the community's counter.
/// `counter = max(counter, received) + 1` — standard Lamport merge rule.
/// Used on every gossip message receive.
pub fn merge_lamport(state: &Arc<AppState>, community_id: &str, received: u64) {
    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        let mut clock = rekindle_gossip::lamport::LamportClock::new(cs.lamport_counter);
        cs.lamport_counter = clock.merge(received);
    }
}

/// Compute effective permissions for the local user in a community channel.
/// Uses the cached CRDT governance state and returns 0 until governance loads.
pub fn my_permissions(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: Option<&rekindle_types::id::ChannelId>,
) -> u64 {
    let communities = state.communities.read();
    let Some(cs) = communities.get(community_id) else {
        return 0;
    };
    let Some(gov) = &cs.governance_state else {
        return 0;
    };
    let Some(pseudo_hex) = &cs.my_pseudonym_key else {
        return 0;
    };
    // Decode hex pseudonym to PseudonymKey
    let pseudo_bytes: [u8; 32] = match hex::decode(pseudo_hex) {
        Ok(b) if b.len() == 32 => b.try_into().unwrap_or([0u8; 32]),
        _ => return 0,
    };
    let pseudo = rekindle_types::id::PseudonymKey(pseudo_bytes);
    let now = rekindle_utils::timestamp_secs();
    rekindle_governance::permissions::compute_permissions(&pseudo, channel_id, gov, now)
}

fn role_id_to_legacy_u32(role_id: &rekindle_types::id::RoleId) -> u32 {
    u32::from_le_bytes([role_id.0[0], role_id.0[1], role_id.0[2], role_id.0[3]])
}
