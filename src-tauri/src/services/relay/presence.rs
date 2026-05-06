//! Strand Relay presence caching (architecture §13.5).
//!
//! When Carol agrees to relay for Bob, she also serves Bob's status to
//! peers asking "do you know Bob?" — faster than a DHT lookup. The
//! "social CDN" property: status flows along the same friendship edges
//! the relay overlay uses.

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::db::DbPool;
use crate::db_helpers::db_call_or_default;
use crate::services::message_service;
use crate::state::{AppState, UserStatus};
use crate::state_helpers;

/// Cooldown window between consecutive probes for the same target.
/// BEP-11 PEX uses ~60s as a sane bound; we adopt the same. Forward
/// architecture: the spec is silent on rate limits, so we match the
/// canonical peer-exchange interval rather than ship an unbounded
/// fan-out.
const PROBE_COOLDOWN_SECS: u64 = 60;
/// Maximum number of friends queried per probe. Prevents the
/// "100 friends → 100 app_messages" amplification flagged by audit.
const MAX_PROBE_FANOUT: usize = 8;

/// Alice's side: probe a sampled subset of friends — preferring those
/// with a cached route (most likely to be online and to hold a status
/// snapshot for `target_pubkey`). Suppressed if a probe for the same
/// target fired within `PROBE_COOLDOWN_SECS` (architecture §13.5 is
/// silent on rate limiting, so we adopt BEP-11's 60s bound).
pub async fn probe_friends_for_status(
    state: &Arc<AppState>,
    pool: &DbPool,
    target_pubkey: &str,
) {
    if !try_acquire_probe_slot(state, target_pubkey) {
        tracing::trace!(target = %target_pubkey, "dropping status probe — within cooldown");
        return;
    }
    let friends_to_ask = select_probe_targets(state, target_pubkey);
    if friends_to_ask.is_empty() {
        return;
    }
    let payload = MessagePayload::StatusRequest {
        target_pubkey: target_pubkey.to_string(),
    };
    for friend in friends_to_ask {
        let _ = message_service::send_to_peer_raw(state, pool, &friend, &payload).await;
    }
}

/// Pick up to `MAX_PROBE_FANOUT` friends most likely to hold a recent
/// snapshot of `target_pubkey`. Friends with a cached route (recently
/// seen) come first; we backfill from the rest of the friend list only
/// if too few are online.
fn select_probe_targets(state: &Arc<AppState>, target_pubkey: &str) -> Vec<String> {
    let friends: Vec<String> = {
        let friends_map = state.friends.read();
        friends_map
            .keys()
            .filter(|k| k.as_str() != target_pubkey)
            .cloned()
            .collect()
    };
    let mut online: Vec<String> = Vec::new();
    let mut offline: Vec<String> = Vec::new();
    for key in friends {
        if state_helpers::cached_route_blob(state, &key).is_some() {
            online.push(key);
        } else {
            offline.push(key);
        }
    }
    online.truncate(MAX_PROBE_FANOUT);
    if online.len() < MAX_PROBE_FANOUT {
        let need = MAX_PROBE_FANOUT - online.len();
        online.extend(offline.into_iter().take(need));
    }
    online
}

/// Cooldown gate. Returns `true` if a probe for this target hasn't
/// fired in the last `PROBE_COOLDOWN_SECS`; updates the timestamp.
fn try_acquire_probe_slot(state: &Arc<AppState>, target_pubkey: &str) -> bool {
    let now = rekindle_utils::timestamp_secs();
    let mut cooldown = state.relay_probe_cooldown.lock();
    if let Some(&last) = cooldown.get(target_pubkey) {
        if now.saturating_sub(last) < PROBE_COOLDOWN_SECS {
            return false;
        }
    }
    cooldown.insert(target_pubkey.to_string(), now);
    // Opportunistic GC: drop entries older than 4× the cooldown so the
    // map doesn't grow unboundedly for ephemeral probe targets.
    let stale_horizon = PROBE_COOLDOWN_SECS.saturating_mul(4);
    cooldown.retain(|_, &mut t| now.saturating_sub(t) <= stale_horizon);
    true
}

/// Carol's side: respond to a `StatusRequest` from `requester` asking
/// about `target`. Replies only when:
///   - the requester is a known friend (privacy: don't gossip our
///     friends' status to strangers), and
///   - we have an active `strand_relay_volunteered` row for `target`.
pub async fn respond_to_status_request(
    state: &Arc<AppState>,
    pool: &DbPool,
    requester_pubkey: &str,
    target_pubkey: &str,
) -> Result<(), String> {
    if !state_helpers::is_friend(state, requester_pubkey) {
        return Err("status request from non-friend".into());
    }

    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }
    let owner = owner_key;
    let target = target_pubkey.to_string();
    let volunteered: bool = db_call_or_default(pool, move |conn| {
        Ok(conn
            .query_row(
                "SELECT 1 FROM strand_relay_volunteered
                 WHERE owner_key = ?1 AND friend_public_key = ?2 LIMIT 1",
                rusqlite::params![owner, target],
                |_| Ok(()),
            )
            .is_ok())
    })
    .await;
    if !volunteered {
        // Architecture §13.5: only relay friends serve a peer's status.
        // Reply with empty so the requester can fall back to DHT.
        let payload = MessagePayload::StatusResponse {
            target_pubkey: target_pubkey.to_string(),
            status: String::new(),
            status_message: None,
            last_seen: 0,
            route_blob: Vec::new(),
        };
        return message_service::send_to_peer_raw(state, pool, requester_pubkey, &payload).await;
    }

    // Pull our cached friend snapshot.
    let snapshot = {
        let friends = state.friends.read();
        friends.get(target_pubkey).map(|f| {
            let last_seen = f
                .last_heartbeat_at
                .or(f.last_seen_at)
                .map_or(0, |ms| u64::try_from(ms / 1000).unwrap_or(0));
            (
                user_status_str(f.status),
                f.status_message.clone(),
                last_seen,
            )
        })
    };
    let route_blob = state_helpers::cached_route_blob(state, target_pubkey).unwrap_or_default();

    let (status, status_message, last_seen) =
        snapshot.unwrap_or_else(|| (String::from("offline"), None, 0));
    let payload = MessagePayload::StatusResponse {
        target_pubkey: target_pubkey.to_string(),
        status,
        status_message,
        last_seen,
        route_blob,
    };
    message_service::send_to_peer_raw(state, pool, requester_pubkey, &payload).await
}

/// Alice's side: a friend just told us about a peer. Promote the
/// snapshot into our local friend cache only when we know that peer
/// (their existing FriendState gets refreshed) so we never trust a
/// relay's data about a stranger.
pub fn handle_status_response(
    state: &Arc<AppState>,
    target_pubkey: &str,
    status: &str,
    status_message: Option<&str>,
    last_seen: u64,
    route_blob: &[u8],
) {
    if status.is_empty() {
        return; // relay had nothing for us
    }
    if !state_helpers::is_friend(state, target_pubkey) {
        return;
    }
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(target_pubkey) {
            if let Some(parsed) = parse_user_status(status) {
                friend.status = parsed;
            }
            if let Some(msg) = status_message {
                friend.status_message = Some(msg.to_string());
            }
            if last_seen > 0 {
                let ms = i64::try_from(last_seen)
                    .ok()
                    .and_then(|s| s.checked_mul(1000));
                friend.last_seen_at = ms;
                friend.last_heartbeat_at = ms;
            }
        }
    }
    if !route_blob.is_empty() {
        state_helpers::cache_peer_route(state, target_pubkey, route_blob.to_vec());
    }
}

fn user_status_str(status: UserStatus) -> String {
    // Architecture §13.5: relay-served status MUST match what we'd serve
    // over the regular presence channel. Treat `Invisible` as `Offline`
    // on the wire so we don't out our friend.
    match status {
        UserStatus::Online => "online",
        UserStatus::Away => "away",
        UserStatus::Busy => "busy",
        UserStatus::Offline | UserStatus::Invisible => "offline",
    }
    .to_string()
}

fn parse_user_status(s: &str) -> Option<UserStatus> {
    match s {
        "online" => Some(UserStatus::Online),
        "away" => Some(UserStatus::Away),
        "busy" => Some(UserStatus::Busy),
        "offline" => Some(UserStatus::Offline),
        _ => None,
    }
}
