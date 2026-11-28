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
use std::time::Duration;
use tokio::time::Instant;

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

/// The relay routes of a published relay pool (profile subkey 8, JSON
/// `Vec<Vec<u8>>`), dummies dropped: a dummy's first byte is `0xFF`, the
/// padding that hides the pool's real size.
#[must_use]
pub fn decode_relay_pool(pool_body: &[u8]) -> Vec<Vec<u8>> {
    let entries: Vec<Vec<u8>> = serde_json::from_slice(pool_body).unwrap_or_default();
    entries
        .into_iter()
        .filter(|blob| blob.first().is_some_and(|&b| b != 0xFF))
        .collect()
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
#[path = "relay_tests.rs"]
mod tests;
