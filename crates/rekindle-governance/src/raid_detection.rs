//! Architecture §20.6 — peer-side raid detection.
//!
//! No coordinator is available to count joins for the whole community,
//! so each peer keeps a per-community sliding window of recent joins
//! and emits a `RaidDetected` event to its own UI when the rate
//! crosses the policy threshold. Moderators in that client then take
//! the spec-listed actions (pause invites, ban floods, increase
//! verification).
//!
//! The window length and threshold come from
//! `CommunityPolicy.max_joins_per_interval` /
//! `CommunityPolicy.join_interval_seconds` (architecture §20.6 line
//! 2607). When no policy entry exists yet, the architecture defaults
//! apply: 20 joins per 600 s.

use std::collections::VecDeque;

use crate::state::CommunityPolicyState;

/// Result of recording a single observed join. When `Some`, the caller
/// should emit a `RaidDetected` UI event with these values; otherwise
/// the rate is below the threshold (or has already been alerted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaidAlert {
    pub joins_in_window: u32,
    pub max_joins_per_interval: u32,
    pub join_interval_seconds: u32,
}

/// Resolve `(max_joins_per_interval, join_interval_seconds)` for a
/// community, falling back to architecture §20.6 defaults when no
/// `CommunityPolicy` entry has been merged.
pub fn resolve_thresholds(policy: Option<&CommunityPolicyState>) -> (u32, u32) {
    let max_joins = policy
        .map(|p| p.max_joins_per_interval)
        .filter(|n| *n > 0)
        .unwrap_or(CommunityPolicyState::DEFAULT_MAX_JOINS_PER_INTERVAL);
    let interval = policy
        .map(|p| p.join_interval_seconds)
        .filter(|n| *n > 0)
        .unwrap_or(CommunityPolicyState::DEFAULT_JOIN_INTERVAL_SECONDS);
    (max_joins, interval)
}

/// Push `(now_secs, pseudonym)` into the sliding window, evict
/// observations older than `interval_seconds`, and decide whether to
/// alert.
///
/// Pseudonym deduplication: if the same pseudonym appears multiple
/// times within the window (e.g. retry storms or a member rejoining
/// after a brief disconnect), it counts once. The architecture says
/// "moderators alert when join *rate* exceeds threshold" — not
/// observation count.
pub fn observe_join(
    window: &mut VecDeque<(u64, String)>,
    now_secs: u64,
    pseudonym: &str,
    policy: Option<&CommunityPolicyState>,
) -> Option<RaidAlert> {
    let (max_joins, interval) = resolve_thresholds(policy);
    let interval_u64 = u64::from(interval);
    let cutoff = now_secs.saturating_sub(interval_u64);

    while window
        .front()
        .is_some_and(|(timestamp, _)| *timestamp < cutoff)
    {
        window.pop_front();
    }

    if !window.iter().any(|(_, pk)| pk == pseudonym) {
        window.push_back((now_secs, pseudonym.to_string()));
    }

    let joins_in_window = u32::try_from(window.len()).unwrap_or(u32::MAX);
    if joins_in_window >= max_joins {
        Some(RaidAlert {
            joins_in_window,
            max_joins_per_interval: max_joins,
            join_interval_seconds: interval,
        })
    } else {
        None
    }
}

#[cfg(test)]
#[path = "raid_detection_tests.rs"]
mod tests;
