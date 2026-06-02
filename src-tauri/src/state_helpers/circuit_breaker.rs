//! Per-community RPC circuit breaker.

use std::sync::Arc;

use crate::state::AppState;

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
