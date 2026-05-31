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
mod tests {
    use super::*;

    fn policy(max: u32, interval: u32) -> CommunityPolicyState {
        CommunityPolicyState {
            policy_text: None,
            max_joins_per_interval: max,
            join_interval_seconds: interval,
            lamport: 1,
        }
    }

    #[test]
    fn observe_below_threshold_returns_none() {
        let mut window = VecDeque::new();
        let p = policy(5, 60);
        for i in 0..4 {
            assert!(
                observe_join(&mut window, 100 + i, &format!("p{i}"), Some(&p)).is_none()
            );
        }
        assert_eq!(window.len(), 4);
    }

    #[test]
    fn observe_at_threshold_returns_alert() {
        let mut window = VecDeque::new();
        let p = policy(3, 60);
        observe_join(&mut window, 100, "p1", Some(&p));
        observe_join(&mut window, 101, "p2", Some(&p));
        let alert = observe_join(&mut window, 102, "p3", Some(&p));
        assert_eq!(
            alert,
            Some(RaidAlert {
                joins_in_window: 3,
                max_joins_per_interval: 3,
                join_interval_seconds: 60,
            })
        );
    }

    #[test]
    fn old_entries_are_evicted_outside_window() {
        let mut window = VecDeque::new();
        let p = policy(3, 60);
        observe_join(&mut window, 100, "p1", Some(&p));
        observe_join(&mut window, 200, "p2", Some(&p)); // p1 is now stale
        let alert = observe_join(&mut window, 200, "p3", Some(&p));
        assert!(alert.is_none(), "only p2+p3 are inside the 60s window");
        assert_eq!(window.len(), 2);
    }

    #[test]
    fn duplicate_pseudonym_in_window_is_not_double_counted() {
        let mut window = VecDeque::new();
        let p = policy(3, 60);
        observe_join(&mut window, 100, "alice", Some(&p));
        observe_join(&mut window, 101, "alice", Some(&p));
        observe_join(&mut window, 102, "alice", Some(&p));
        assert_eq!(window.len(), 1);
    }

    #[test]
    fn missing_policy_uses_architecture_defaults() {
        let (max, interval) = resolve_thresholds(None);
        assert_eq!(max, 20);
        assert_eq!(interval, 600);
    }
}
