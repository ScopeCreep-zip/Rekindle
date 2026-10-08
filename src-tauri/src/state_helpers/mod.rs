//! State access helpers for extracting and storing commonly-accessed fields
//! from [`AppState`].
//!
//! Read helpers acquire a read lock, clone out the needed value(s), and drop
//! the guard immediately — safe to call before `.await` points. Write helpers
//! (`store_dht_record`, `cache_peer_route`, etc.) acquire
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
//! - [`meks`] — live community/channel keys and the `ChannelKeyProvider`.
//! - [`voice`] — running voice-engine accessors.

mod circuit_breaker;
mod communities;
mod dht_records;
mod friends;
mod governance;
mod governance_persist;
mod identity;
mod meks;
mod node;
mod routes;
mod voice;

pub use circuit_breaker::{is_circuit_open, reset_circuit_breaker, trip_circuit_breaker};
pub use communities::{
    channel_is_stage, communities_with_governance_keys, community_scope, my_pseudonym_key,
    push_community_channel, set_community_channels, spawn_in_community,
    spawn_in_community_with_token,
};
pub use dht_records::{release_friend_record, store_dht_record, DhtRecordType};
pub use friends::{
    accepted_friend_keys, friend_dht_key, friend_display_name, friend_field, friend_mailbox_key,
    friends_with_dht_keys, is_active_friend_authoritative, is_friend, is_friend_accepted,
};
pub use governance::{
    governance_clock, governance_key, governance_state, merge_message_lamport, my_permissions,
    next_governance_lamport, next_message_lamport, observe_governance_lamport, permissions_for,
    permissions_for_pseudonym, set_governance_state,
};
pub use governance_persist::persist_governance_snapshot_to_sqlite;
pub use identity::{
    current_identity, current_owner_key, identity_display_name, identity_secret, identity_status,
    owner_key_or_default, pseudonym_credentials, voice_self_identity,
};
pub use meks::{
    clear_meks, current_mek, forget_community_keys, install_mek, key_provider, text_scope, LiveMek,
    LiveMekCache,
};
pub use node::{
    app_context, app_handle, friend_list_dht_key, friend_list_owner_keypair, is_attached,
    login_scope, login_scope_or_closed, our_media_route_blob, our_route_blob, own_routes,
    profile_dht_info, record_pool, require_safe_routing_context, safe_api_and_routing_context,
    safe_routing_context, spawn_in_login, spawn_in_login_with_token, veilid_api,
};
pub use routes::{
    cache_peer_route, cached_route_blob, call_route_blob, evict_stale_peer_routes,
    friend_for_dht_key, import_route_blob, invalidate_cached_peer_route, message_route_blob,
    note_send_result, on_dead_remote_routes, route_imports, route_send_failed,
    try_import_peer_route,
};
pub use voice::{
    current_voice_scope, media_live_peers, media_roster_for, note_media_live,
    set_voice_engine_deafened, set_voice_engine_muted, voice_engine_present, voice_media,
    voice_transport_for,
};

// ── Shared private helpers used across submodules ──────────────────────

pub(super) fn safe_routing_context_from(
    routing_context: veilid_core::RoutingContext,
) -> Option<veilid_core::RoutingContext> {
    let profile = rekindle_route::contexts::RouteContextSpec::rc_safe().safety_profile();
    routing_context
        .with_safety(rekindle_protocol::dht::pool::safety_selection(&profile))
        .ok()
}

/// Decode a hex string into a 16-byte id, or all-zeros if it is not
/// exactly 16 bytes of valid hex.
///
/// There were three copies of this. Two required an exact 16 bytes;
/// this one decoded whatever it could and zero-padded the remainder, so
/// a truncated id like `"deadbeef"` became
/// `deadbeef00000000000000000000` here and `0…0` everywhere else — a
/// half-real id that shares a prefix with a genuine one is worse than
/// an obviously-invalid one, so the strict behaviour is what survives.
pub(crate) fn hex_to_id_16(hex_str: &str) -> [u8; 16] {
    hex::decode(hex_str)
        .ok()
        .and_then(|b| b.try_into().ok())
        .unwrap_or([0u8; 16])
}
