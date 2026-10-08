//! Strand Relay selection + reliability primitives (architecture §13).
//!
//! Two pure pieces of the Alice-side relay path live here, free of any
//! Veilid / state / persistence coupling:
//!
//! - **Candidate ranking** ([`rank_candidates`]) — deterministically order
//!   a target's published relay pool by `blake3(target_pubkey || blob)`.
//!   The same target consistently picks the same relay first, so
//!   keepalive/caching effects accumulate on one route. Mirrors the MEK
//!   rotator's `selected_request_responder` "lowest-hash responder".
//! - **Circuit breaker** ([`RelayHealth`] + `record_*`) — per-relay
//!   `(success, failure)` tracking. After three consecutive failures the
//!   circuit opens for [`BREAKER_COOLDOWN`]; further sends through that
//!   relay are skipped. After cooldown the breaker is half-open — exactly
//!   one tentative request goes through; success closes it, failure
//!   re-opens it.
//!
//! The owning process keeps the `HashMap<RelayKey, RelayHealth>` behind a
//! lock (peer reliability is observed, not assigned — §14.5); this module
//! only mutates a borrowed map.

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::time::{Duration, Instant};

const FAILURE_THRESHOLD: u32 = 3;
const BREAKER_COOLDOWN: Duration = Duration::from_secs(60);

/// Identifier for a relay entry. We key on the route blob's Blake3 hash
/// because the relay's pseudonym isn't carried in the relay pool — only
/// the route blob is. The hash collapses identical routes to the same
/// circuit-breaker state without retaining the full blob.
pub type RelayKey = [u8; 32];

#[derive(Debug, Default, Clone)]
pub struct RelayHealth {
    pub successes: u32,
    pub failures: u32,
    /// Consecutive failures since the last success. Resets to 0 on any
    /// successful send. When this reaches `FAILURE_THRESHOLD` the breaker
    /// opens.
    pub consecutive_failures: u32,
    /// When the breaker tripped open. `None` means the breaker is closed
    /// (healthy or in half-open).
    pub opened_at: Option<Instant>,
}

impl RelayHealth {
    /// Whether this relay is currently in an open circuit-breaker state
    /// and should be skipped.
    pub fn is_circuit_open(&self) -> bool {
        match self.opened_at {
            Some(when) => when.elapsed() < BREAKER_COOLDOWN,
            None => false,
        }
    }

    /// Whether the breaker is past cooldown and ready for a tentative
    /// half-open probe. The caller should let exactly ONE send through in
    /// this state; on success they call [`record_success`] (which closes
    /// the breaker), on failure [`record_failure`] re-opens it.
    pub fn is_half_open(&self) -> bool {
        match self.opened_at {
            Some(when) => when.elapsed() >= BREAKER_COOLDOWN,
            None => false,
        }
    }
}

/// Compute the relay key from the route blob. Stable across process
/// restarts (deterministic hash) so cached health survives restarts if we
/// later persist it; today the map is in-memory only.
pub fn key_for(route_blob: &[u8]) -> RelayKey {
    *blake3::hash(route_blob).as_bytes()
}

/// Increment the success counter and close the breaker.
pub fn record_success<S: BuildHasher>(map: &mut HashMap<RelayKey, RelayHealth, S>, key: RelayKey) {
    let entry = map.entry(key).or_default();
    entry.successes = entry.successes.saturating_add(1);
    entry.consecutive_failures = 0;
    entry.opened_at = None;
}

/// Increment the failure counter; open the breaker if we've crossed the
/// consecutive-failure threshold.
pub fn record_failure<S: BuildHasher>(map: &mut HashMap<RelayKey, RelayHealth, S>, key: RelayKey) {
    let entry = map.entry(key).or_default();
    entry.failures = entry.failures.saturating_add(1);
    entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
    if entry.consecutive_failures >= FAILURE_THRESHOLD {
        entry.opened_at = Some(Instant::now());
    }
}

/// Read-only snapshot of a relay's circuit state. Returns `None` when the
/// relay has no recorded history (treat as healthy).
pub fn lookup<S: BuildHasher>(
    map: &HashMap<RelayKey, RelayHealth, S>,
    key: &RelayKey,
) -> Option<RelayHealth> {
    map.get(key).cloned()
}

/// Rank candidate relay blobs by `blake3(target_pubkey || blob)`
/// ascending. The same target consistently picks the same relay first, so
/// caching/keepalive effects accumulate on one route rather than
/// scattering across N relays per send.
pub fn rank_candidates(target_pubkey: &str, candidates: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let mut scored: Vec<([u8; 32], Vec<u8>)> = candidates
        .into_iter()
        .map(|blob| {
            let mut hasher = blake3::Hasher::new();
            hasher.update(target_pubkey.as_bytes());
            hasher.update(&blob);
            (*hasher.finalize().as_bytes(), blob)
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0));
    scored.into_iter().map(|(_, blob)| blob).collect()
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

    #[test]
    fn rank_is_deterministic_per_target() {
        let candidates = vec![vec![0x10, 0x20], vec![0x30, 0x40], vec![0x50, 0x60]];
        let a = rank_candidates("alice-pubkey-hex", candidates.clone());
        let b = rank_candidates("alice-pubkey-hex", candidates);
        assert_eq!(a, b, "ranking must be stable across calls");
    }

    #[test]
    fn rank_differs_per_target() {
        let candidates = vec![vec![0x11], vec![0x22], vec![0x33], vec![0x44]];
        let a = rank_candidates("alice-pubkey", candidates.clone());
        let b = rank_candidates("zach-pubkey", candidates);
        assert_ne!(a, b);
    }
}
