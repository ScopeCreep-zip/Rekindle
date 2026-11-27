//! Phase 17 — cascade election constants + delay computation.
//!
//! The deterministic ranking primitives (`select_mek_responder` for
//! cascade_index=0 and `cascade_candidates` for index ≥1) already live
//! in `rekindle_secrets::rotator`. This module wraps those with the
//! cascade-timing constants the MEK rotation orchestrator needs.

use std::time::Duration;

// Re-export so callers can use the rotator helpers via the crate's
// public surface without adding a separate dep on rekindle-secrets.
pub use rekindle_secrets::rotator::{cascade_candidates, select_mek_responder};

/// Architecture §10.5 — wait this long before falling through to the
/// next cascade level when the elected rotator doesn't deliver.
pub const CASCADE_TIMEOUT_SECS: u64 = 30;

/// Architecture §10.5 — at most this many cascade levels (0 = primary,
/// 1..=3 = fallback). 3 cascades × 30 s = ≤120 s before MEK rotation
/// gives up.
pub const MAX_CASCADES: usize = 3;

/// Delay before attempting cascade level `index`. Level 0 is the
/// primary rotator and fires immediately (`Duration::ZERO`); each
/// subsequent level waits an additional 30 s (`CASCADE_TIMEOUT_SECS`).
#[must_use]
pub fn cascade_delay(index: usize) -> Duration {
    Duration::from_secs(CASCADE_TIMEOUT_SECS * index as u64)
}

#[cfg(test)]
#[path = "election_tests.rs"]
mod tests;
