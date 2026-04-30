use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

pub async fn rejoin_community(state: &Arc<AppState>, community_id: &str) -> Result<(), String> {
    if crate::state_helpers::is_circuit_open(state, community_id) {
        tracing::debug!(community = %community_id, "skipping rejoin — circuit breaker open");
        return Ok(());
    }

    let pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let our_route_blob = state_helpers::our_route_blob(state);
    let status = state_helpers::identity_status(state).unwrap_or(crate::state::UserStatus::Online);
    let status_str = match status {
        crate::state::UserStatus::Online => "online",
        crate::state::UserStatus::Away => "away",
        crate::state::UserStatus::Busy => "busy",
        crate::state::UserStatus::Offline | crate::state::UserStatus::Invisible => "offline",
    };

    match crate::services::community::send_to_mesh(
        state,
        community_id,
        &rekindle_protocol::dht::community::envelope::CommunityEnvelope::PresenceUpdate {
            pseudonym_key,
            status: status_str.to_string(),
            game_info: None,
            route_blob: our_route_blob,
        },
    ) {
        Ok(()) => {
            state_helpers::reset_circuit_breaker(state, community_id);
            tracing::debug!(community = %community_id, "re-announced route via gossip mesh");
        }
        Err(e) => {
            tracing::warn!(community = %community_id, error = %e, "rejoin gossip broadcast failed");
            state_helpers::trip_circuit_breaker(state, community_id);
        }
    }
    Ok(())
}
