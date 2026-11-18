//! Timeout verdicts from the merged governance (plan E3.2 S9), on both
//! modes. A timeout is a signed `TimeoutEntry` op and lifting it a
//! `RemoveTimeoutEntry` (the unsigned `TimeoutMember`, `RemoveTimeout` and
//! `MemberTimedOut` wire arms are retired, S8), so, like the rest of
//! membership (`member_events.rs`), it is read as the difference between
//! the state last projected and the state just merged and published once:
//! `TimedOut` with the duration and reason the op gave, and
//! `TimeoutStatusChanged` with the end the member list shows; `TimeoutRemoved`
//! and `TimeoutStatusChanged { None }` when it is lifted. A timeout that
//! runs out is no transition: the end it published already says when, as
//! Discord's `communication_disabled_until` does.

use rekindle_governance::state::GovernanceState;
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::MembershipEvent;

/// The timeout events that take `p` from `prev` to `next`. A newer
/// `TimeoutEntry` (another stamp) over a running one is a new timeout.
pub(super) fn timeout_transitions(
    community_id: &str,
    p: &PseudonymKey,
    prev: &GovernanceState,
    next: &GovernanceState,
) -> Vec<MembershipEvent> {
    let community = || community_id.to_owned();
    match (prev.timeouts.get(p), next.timeouts.get(p)) {
        (before, Some(now)) if before.map(|b| b.stamp) != Some(now.stamp) => vec![
            MembershipEvent::TimedOut {
                community: community(),
                target_pseudonym: p.to_hex(),
                duration_seconds: now.duration_seconds,
                reason: now.reason.clone(),
            },
            MembershipEvent::TimeoutStatusChanged {
                community: community(),
                pseudonym: p.to_hex(),
                timeout_until: Some(now.started_at.saturating_add(now.duration_seconds)),
            },
        ],
        (Some(_), None) => vec![
            MembershipEvent::TimeoutRemoved {
                community: community(),
                target_pseudonym: p.to_hex(),
            },
            MembershipEvent::TimeoutStatusChanged {
                community: community(),
                pseudonym: p.to_hex(),
                timeout_until: None,
            },
        ],
        _ => Vec::new(),
    }
}
