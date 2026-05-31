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

    // Architecture §14.2 — returning members request missing message
    // ranges from peers who advertise them. Firing one initial-sync
    // round on rejoin gives faster catch-up than waiting for the next
    // presence-poll cadence tick (5–60 s). Uses gossip degree 1 as
    // the floor — a returning member is connected to at least one
    // peer (the one that triggered the rejoin); the orchestrator's
    // own checks handle the no-peers / no-route cases gracefully.
    crate::services::community::run_initial_sync(state, community_id, 1).await;

    Ok(())
}
