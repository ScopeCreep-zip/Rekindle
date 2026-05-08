//! Wave 7 P7.3 — per-relay health tracking with circuit breaker.
//!
//! Each Strand Relay route accumulates `(success_count, failure_count)`
//! over its lifetime in this process. After three consecutive failures
//! the circuit opens for `BREAKER_COOLDOWN`; further sends through that
//! relay are skipped without trying. After cooldown the breaker enters
//! a half-open state — exactly one tentative request goes through; on
//! success the breaker closes, on failure the cooldown resets.
//!
//! The state lives on `AppState` as a single `Mutex<HashMap<...>>` so
//! the relay-send hot path takes one lock per call, plus `record_*`
//! lock per result. Mirrors the chiral mutual-aid principle in §14.5
//! (peer reliability is observed, not assigned).

use std::collections::HashMap;
use std::time::{Duration, Instant};

const FAILURE_THRESHOLD: u32 = 3;
const BREAKER_COOLDOWN: Duration = Duration::from_secs(60);

/// Identifier for a relay entry. We key on the route blob's Blake3
/// hash because the relay's pseudonym isn't carried in the relay pool —
/// only the route blob is. The hash collapses identical routes to the
/// same circuit-breaker state without retaining the full blob.
pub type RelayKey = [u8; 32];

#[derive(Debug, Default, Clone)]
pub struct RelayHealth {
    pub successes: u32,
    pub failures: u32,
    /// Consecutive failures since the last success. Resets to 0 on any
    /// successful send. When this reaches `FAILURE_THRESHOLD` the
    /// breaker opens.
    pub consecutive_failures: u32,
    /// When the breaker tripped open. `None` means the breaker is
    /// closed (healthy or in half-open).
    pub opened_at: Option<Instant>,
}

impl RelayHealth {
    /// Whether this relay is currently in an open circuit breaker
    /// state and should be skipped.
    pub fn is_circuit_open(&self) -> bool {
        match self.opened_at {
            Some(when) => when.elapsed() < BREAKER_COOLDOWN,
            None => false,
        }
    }

    /// Whether the breaker is past cooldown and ready for a tentative
    /// half-open probe. The caller should let exactly ONE send through
    /// in this state; on success they call `record_success` (which
    /// closes the breaker), on failure `record_failure` re-opens it.
    pub fn is_half_open(&self) -> bool {
        match self.opened_at {
            Some(when) => when.elapsed() >= BREAKER_COOLDOWN,
            None => false,
        }
    }
}

/// Compute the relay key from the route blob. Stable across process
/// restarts (deterministic hash) so cached health survives restarts if
/// we later persist it; today the map is in-memory only.
pub fn key_for(route_blob: &[u8]) -> RelayKey {
    *blake3::hash(route_blob).as_bytes()
}

/// Increment the success counter and close the breaker.
pub fn record_success(map: &mut HashMap<RelayKey, RelayHealth>, key: RelayKey) {
    let entry = map.entry(key).or_default();
    entry.successes = entry.successes.saturating_add(1);
    entry.consecutive_failures = 0;
    entry.opened_at = None;
}

/// Increment the failure counter; open the breaker if we've crossed
/// the consecutive-failure threshold.
pub fn record_failure(map: &mut HashMap<RelayKey, RelayHealth>, key: RelayKey) {
    let entry = map.entry(key).or_default();
    entry.failures = entry.failures.saturating_add(1);
    entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
    if entry.consecutive_failures >= FAILURE_THRESHOLD {
        entry.opened_at = Some(Instant::now());
    }
}

/// Read-only snapshot of a relay's circuit state. Returns `None` when
/// the relay has no recorded history (treat as healthy).
pub fn lookup(map: &HashMap<RelayKey, RelayHealth>, key: &RelayKey) -> Option<RelayHealth> {
    map.get(key).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> RelayKey {
        [byte; 32]
    }

    #[test]
    fn closed_breaker_is_neither_open_nor_half_open() {
        let mut map: HashMap<RelayKey, RelayHealth> = HashMap::new();
        record_success(&mut map, key(1));
        let health = lookup(&map, &key(1)).unwrap();
        assert!(!health.is_circuit_open());
        assert!(!health.is_half_open());
        assert_eq!(health.consecutive_failures, 0);
    }

    #[test]
    fn three_consecutive_failures_open_breaker() {
        let mut map: HashMap<RelayKey, RelayHealth> = HashMap::new();
        record_failure(&mut map, key(1));
        record_failure(&mut map, key(1));
        let health = lookup(&map, &key(1)).unwrap();
        assert!(!health.is_circuit_open(), "two failures must not open");
        record_failure(&mut map, key(1));
        let health = lookup(&map, &key(1)).unwrap();
        assert!(health.is_circuit_open(), "three failures must open");
    }

    #[test]
    fn success_after_failures_resets_consecutive_count() {
        let mut map: HashMap<RelayKey, RelayHealth> = HashMap::new();
        record_failure(&mut map, key(1));
        record_failure(&mut map, key(1));
        record_success(&mut map, key(1));
        record_failure(&mut map, key(1));
        let health = lookup(&map, &key(1)).unwrap();
        assert_eq!(health.consecutive_failures, 1);
        assert!(!health.is_circuit_open());
    }

    #[test]
    fn distinct_relays_isolated() {
        let mut map: HashMap<RelayKey, RelayHealth> = HashMap::new();
        record_failure(&mut map, key(1));
        record_failure(&mut map, key(1));
        record_failure(&mut map, key(1));
        // Relay 2 is untouched.
        assert!(lookup(&map, &key(2)).is_none());
        assert!(lookup(&map, &key(1)).unwrap().is_circuit_open());
    }
}
