//! Pure presence-sharing policy — the default-deny consent gate.
//!
//! Two pure functions bookend the privacy model so sharing every signal
//! is the user's choice (architecture §10 / §16.3, Signal/SimpleX posture):
//!
//! - [`apply_sharing_policy`] redacts our OWN session down to exactly what
//!   we consented to publish, applied just before the row is signed. What
//!   it strips (`location`/`activity`) is what the caller then re-attaches
//!   as MEK-encrypted [`SessionExtras`].
//! - [`filter_incoming`] drops a PEER's signals we don't reciprocate,
//!   applied after decode. The model self-balances without a coordinator:
//!   you only get to read a signal you also expose.
//!
//! [`coarsen_to_bucket`] floors last-seen to hour granularity — peers
//! never see finer than that unless the user opts into `Exact`.

use rekindle_types::presence::{
    LastSeenPrecision, MemberSession, PresenceSharingPolicy, SessionStatus, ShareScope,
};

/// Seconds in one hour — the coarse last-seen bucket width.
const HOUR_SECS: u64 = 3600;

/// Redact a session to exactly what `policy` permits, before it is signed
/// and published. Pure; the caller splits the surviving `location` /
/// `activity` into MEK-encrypted extras afterward.
#[must_use]
pub fn apply_sharing_policy(mut s: MemberSession, policy: &PresenceSharingPolicy) -> MemberSession {
    // Master switch: not sharing "online" at all ⇒ appear offline. The
    // wire-string derivation folds Invisible → "offline" for peers.
    if !policy.share_online {
        s.status = SessionStatus::Invisible;
    }
    if policy.share_location == ShareScope::None {
        s.location = None;
    }
    if policy.share_activity == ShareScope::None {
        s.activity = None;
    }
    s.last_active = match policy.last_seen {
        LastSeenPrecision::Hidden => 0,
        LastSeenPrecision::Coarse => coarsen_to_bucket(s.last_active),
        LastSeenPrecision::Exact => s.last_active,
    };
    s
}

/// Read-side reciprocity: blank out a peer's signals that our own policy
/// does not share, so the model self-balances (you only see what you also
/// expose). Returns the filtered peer session.
#[must_use]
pub fn filter_incoming(
    mut peer: MemberSession,
    our_policy: &PresenceSharingPolicy,
) -> MemberSession {
    if our_policy.share_location == ShareScope::None {
        peer.location = None;
    }
    if our_policy.share_activity == ShareScope::None {
        peer.activity = None;
    }
    if our_policy.last_seen == LastSeenPrecision::Hidden {
        peer.last_active = 0;
    }
    peer
}

/// Floor a unix-seconds timestamp to the start of its hour. Last-seen is
/// never exposed finer than hour granularity unless the user opts into
/// `Exact`.
#[must_use]
pub fn coarsen_to_bucket(secs: u64) -> u64 {
    secs - (secs % HOUR_SECS)
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
