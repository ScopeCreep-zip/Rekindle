//! State access helpers for extracting and storing commonly-accessed fields
//! from [`AppState`].
//!
//! Read helpers acquire a read lock, clone out the needed value(s), and drop
//! the guard immediately — safe to call before `.await` points. Write helpers
//! (`store_dht_record`, `track_open_records`, `cache_peer_route`, etc.) acquire
//! write locks with the same acquire-then-drop discipline.
//!
//! Helpers are grouped by domain into submodules and re-exported flat so all
//! call sites keep using `crate::state_helpers::<fn>`:
//! - [`dht_records`] — DHT record-key storage/tracking on the node handles.
//! - [`identity`] — current identity accessors.
//! - [`node`] — node / network / routing-context accessors.
//! - [`friends`] — friend-state accessors.
//! - [`routes`] — DHT manager + peer route cache.
//! - [`circuit_breaker`] — per-community RPC circuit breaker.
//! - [`communities`] — community channel-list write helpers.
//! - [`governance`] — v2.0 CRDT governance state read/write.
//! - [`governance_persist`] — SQLite snapshot of merged governance.

mod circuit_breaker;
mod communities;
mod dht_records;
mod friends;
mod governance;
mod governance_persist;
mod identity;
mod node;
mod routes;

pub use circuit_breaker::{is_circuit_open, reset_circuit_breaker, trip_circuit_breaker};
pub use communities::{
    channel_media_mek, channel_media_mek_full, communities_with_governance_keys,
    install_channel_mek, install_community_mek, previous_channel_mek, push_community_channel,
    set_community_channels,
};
pub use dht_records::{
    collect_and_clear_community_records, store_dht_record, track_open_records, untrack_records,
    DhtRecordType,
};
pub use friends::{
    accepted_friend_keys, friend_dht_key, friend_display_name, friend_field, friend_mailbox_key,
    friends_with_dht_keys, is_active_friend_authoritative, is_friend, is_friend_accepted,
};
pub use governance::{
    governance_key, governance_state, increment_lamport, lamport_counter, merge_lamport,
    my_permissions, set_governance_state,
};
pub use governance_persist::persist_governance_snapshot_to_sqlite;
pub use identity::{
    current_identity, current_owner_key, identity_display_name, identity_status,
    owner_key_or_default, pseudonym_credentials, voice_self_identity,
};
pub use node::{
    api_and_routing_context, app_handle, friend_list_dht_key, friend_list_owner_keypair,
    is_attached, our_route_blob, profile_dht_info, require_routing_context,
    require_safe_routing_context, routing_context, safe_api_and_routing_context,
    safe_routing_context, veilid_api,
};
pub use routes::{
    cache_peer_route, cached_route_blob, evict_stale_peer_routes, friend_for_dht_key,
    import_route_blob, invalidate_cached_peer_route, try_import_peer_route,
};

// ── Shared private helpers used across submodules ──────────────────────

pub(super) fn safe_routing_context_from(
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

pub(super) fn hex_to_id_16(hex_str: &str) -> [u8; 16] {
    let bytes = hex::decode(hex_str).unwrap_or_else(|_| vec![0u8; 16]);
    let mut arr = [0u8; 16];
    for (i, b) in bytes.iter().take(16).enumerate() {
        arr[i] = *b;
    }
    arr
}

pub(super) fn role_id_to_legacy_u32(role_id: &rekindle_types::id::RoleId) -> u32 {
    u32::from_le_bytes([role_id.0[0], role_id.0[1], role_id.0[2], role_id.0[3]])
}
