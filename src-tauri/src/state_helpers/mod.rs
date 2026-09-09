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
//! - [`voice`] — running voice-engine accessors.

mod circuit_breaker;
mod communities;
mod dht_records;
mod friends;
mod governance;
mod governance_persist;
mod identity;
mod node;
mod routes;
mod voice;

pub use circuit_breaker::{is_circuit_open, reset_circuit_breaker, trip_circuit_breaker};
pub use communities::{
    channel_media_mek, channel_media_mek_full, communities_with_governance_keys,
    install_channel_mek, install_community_mek, my_pseudonym_key, previous_channel_mek,
    push_community_channel, set_community_channels,
};
pub use dht_records::{
    close_and_untrack, collect_and_clear_community_records, store_dht_record, track_open_records,
    untrack_records, DhtRecordType,
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
    current_identity, current_owner_key, identity_display_name, identity_secret, identity_status,
    owner_key_or_default, pseudonym_credentials, voice_self_identity,
};
pub use node::{
    api_and_routing_context, app_context, app_handle, friend_list_dht_key,
    friend_list_owner_keypair, is_attached, our_route_blob, profile_dht_info,
    register_background_handle, require_routing_context, require_safe_routing_context,
    routing_context, safe_api_and_routing_context, safe_routing_context, veilid_api,
};
pub use routes::{
    cache_peer_route, cached_route_blob, evict_stale_peer_routes, friend_for_dht_key,
    import_route_blob, invalidate_cached_peer_route, try_import_peer_route,
};
pub use voice::{set_voice_engine_deafened, set_voice_engine_muted, voice_engine_present};

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
                    veilid_core::Sequencing::PreferUnordered
                },
            },
        ))
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
