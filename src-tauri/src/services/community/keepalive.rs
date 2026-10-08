use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

/// Start the community's record keepalive (plan C7, 12.K): the shared loop
/// (`rekindle_protocol::dht::pool::keepalive::run`: jittered cadence,
/// per-community stagger, rehydrate each record) over the one authoritative
/// inventory — governance, registry, channels, Plate Gate segments, my live
/// invite secrets and GovernanceOverflow pages (§10/§14.1), so no record
/// type is forgotten and left to expire. Runs on the community's scope, so
/// leaving or logging out stops it.
pub fn start_dht_keepalive(state: Arc<AppState>, community_id: String) {
    let scope = state_helpers::community_scope(&state, &community_id);
    scope.spawn_with_token_or_drop("community DHT keepalive", |stop| async move {
        let pool_state = Arc::clone(&state);
        rekindle_protocol::dht::pool::keepalive::run(
            move || state_helpers::record_pool(&pool_state).ok(),
            move || {
                state
                    .communities
                    .read()
                    .get(&community_id)
                    .map(super::record_inventory::warmable_record_keys)
            },
            stop,
        )
        .await;
    });
}
