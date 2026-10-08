//! Phase 23.B — extracted from `state.rs`. Per-community circuit
//! breaker for remote Veilid RPCs. Stored in
//! `AppState.community_circuit_breakers: RwLock<HashMap<String, CircuitBreakerState>>`.

/// Per-community circuit breaker for remote Veilid RPCs.
///
/// Prevents flooding dead routes with parallel 8s timeouts. Once a community
/// trips (>= 3 consecutive failures), further RPCs are rejected instantly for
/// a 30s cooldown period. Resets on success. In-memory only, resets on restart.
pub struct CircuitBreakerState {
    pub tripped_at: std::time::Instant,
    pub failure_count: u32,
}
