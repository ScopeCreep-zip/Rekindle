//! Asking friends for a peer's status and route (architecture §13.5): the
//! "social CDN" — status flows along the friendship edges the relay
//! overlay uses.

use rekindle_codec::capnp_codec::direct::{DirectBody, StatusRequest, StatusResponse};
use rekindle_types::friend::{FriendshipState, UserStatus};

use crate::operations::dm;
use crate::operations::friend::{state, FriendHost};

/// The interval between probes for one target. BEP-11 peer exchange uses
/// ~60 s; §13.5 is silent on rate limits, so we take the same bound rather
/// than an unbounded fan-out.
const PROBE_COOLDOWN_SECS: u64 = 60;
/// At most this many friends are asked per probe (no "100 friends → 100
/// messages" amplification).
const MAX_PROBE_FANOUT: usize = 8;

/// Up to [`MAX_PROBE_FANOUT`] friends most likely to hold a recent status
/// of `target`: those with a cached route (recently seen) first, the rest
/// only to fill.
fn probe_targets(host: &dyn FriendHost, target: &str) -> Vec<String> {
    let Ok(node) = host.node() else {
        return Vec::new();
    };
    let (mut online, offline): (Vec<String>, Vec<String>) = state::friends(host)
        .into_iter()
        .filter(|f| f.public_key != target && f.friendship_state != FriendshipState::Removing)
        .map(|f| f.public_key)
        .partition(|key| dm::route::cached(&node, key).is_some());
    online.truncate(MAX_PROBE_FANOUT);
    let need = MAX_PROBE_FANOUT.saturating_sub(online.len());
    online.extend(offline.into_iter().take(need));
    online
}

/// Ask a sample of friends whether any holds a fresh status for `target`;
/// suppressed within [`PROBE_COOLDOWN_SECS`] of the last probe for it.
pub async fn probe_friends_for_status(host: &dyn FriendHost, target: &str) {
    let Ok(node) = host.node() else {
        return;
    };
    if !node
        .relay()
        .take_probe_slot(target, host.clock().now_secs(), PROBE_COOLDOWN_SECS)
    {
        tracing::trace!(target = %target, "dropping status probe — within cooldown");
        return;
    }
    let request = DirectBody::StatusRequest(StatusRequest {
        target_pubkey: target.to_string(),
    });
    for friend in probe_targets(host, target) {
        node.relay().record_probe(
            &friend,
            target,
            host.clock().now_secs(),
            PROBE_COOLDOWN_SECS,
        );
        if let Err(e) = dm::send_to_peer(host, &friend, &request).await {
            tracing::trace!(to = %friend, error = %e, "status probe not sent");
        }
    }
}

fn parse_status(s: &str) -> Option<UserStatus> {
    match s {
        "online" => Some(UserStatus::Online),
        "away" => Some(UserStatus::Away),
        "busy" => Some(UserStatus::Busy),
        "offline" => Some(UserStatus::Offline),
        _ => None,
    }
}

/// `friend` answered a probe (the 0x0141 body, §6): the snapshot
/// refreshes our view of `target_pubkey` only when we asked that friend
/// about that target within the cooldown and the target is our accepted
/// friend (plan audit G8), so no friend overwrites another's status
/// unasked and a relay's word about a stranger is never trusted. The
/// route it carries is not cached: a route for the target comes only from
/// the target (their own messages or record), never from a third party.
pub(crate) fn on_status_response(host: &dyn FriendHost, friend: &str, r: &StatusResponse) {
    let target = r.target_pubkey.as_str();
    let Ok(node) = host.node() else {
        return;
    };
    let asked = node.relay().take_probe_answer(
        friend,
        target,
        host.clock().now_secs(),
        PROBE_COOLDOWN_SECS,
    );
    if !asked {
        tracing::debug!(from = %friend, target = %target, "dropping an unasked status response");
        return;
    }
    if r.status.is_empty() || !state::is_accepted(host, target) {
        return;
    }
    let seen_ms = i64::try_from(r.last_seen)
        .ok()
        .and_then(|s| s.checked_mul(1000))
        .filter(|_| r.last_seen > 0);
    state::update(host, target, |f| {
        if let Some(status) = parse_status(&r.status) {
            f.status = status;
        }
        if let Some(message) = &r.status_message {
            f.status_message = Some(message.clone());
        }
        if seen_ms.is_some() {
            f.last_seen_at = seen_ms;
            f.last_heartbeat_at = seen_ms;
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::broadcast::peer_registry::RelayState;

    use super::PROBE_COOLDOWN_SECS;

    #[test]
    fn only_the_friend_asked_about_that_target_may_answer_once() {
        let relay = RelayState::default();
        relay.record_probe("bob", "carol", 100, PROBE_COOLDOWN_SECS);
        // Another friend, or Bob about someone else: unasked.
        assert!(!relay.take_probe_answer("mallory", "carol", 101, PROBE_COOLDOWN_SECS));
        assert!(!relay.take_probe_answer("bob", "dave", 101, PROBE_COOLDOWN_SECS));
        assert!(relay.take_probe_answer("bob", "carol", 101, PROBE_COOLDOWN_SECS));
        // One probe takes one answer.
        assert!(!relay.take_probe_answer("bob", "carol", 102, PROBE_COOLDOWN_SECS));
    }

    #[test]
    fn a_late_answer_is_unasked() {
        let relay = RelayState::default();
        relay.record_probe("bob", "carol", 100, PROBE_COOLDOWN_SECS);
        assert!(!relay.take_probe_answer(
            "bob",
            "carol",
            100 + PROBE_COOLDOWN_SECS + 1,
            PROBE_COOLDOWN_SECS
        ));
    }
}
