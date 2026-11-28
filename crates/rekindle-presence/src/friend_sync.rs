//! Friend-presence staleness (plan E2.5, D23): a local TTL timer over the
//! friend projection, with no DHT read. A friend's presence arrives by
//! their profile record's watch (`watch_friend`, `handle_value_change`);
//! a friend whose last heartbeat is older than
//! [`STALE_PRESENCE_THRESHOLD_MS`] is marked offline here.

use crate::deps::{FriendPresenceDeps, FriendPresenceEvent};
use crate::friend::STALE_PRESENCE_THRESHOLD_MS;

/// Mark every friend with a stale `last_heartbeat_at` as offline
/// and emit `FriendOffline` (privacy-gated by `is_friend_accepted`).
/// Pre-port `check_stale_presences`.
pub fn check_stale_friend_presences<D: FriendPresenceDeps>(deps: &D) {
    let now = deps.now_ms();
    for friend_key in deps.find_stale_friend_heartbeats(STALE_PRESENCE_THRESHOLD_MS) {
        let is_accepted = deps.is_friend_accepted(&friend_key);
        deps.set_friend_offline(&friend_key, now);
        if is_accepted {
            tracing::info!(friend = %friend_key, "stale heartbeat — marking offline");
            deps.emit(FriendPresenceEvent::FriendOffline { friend_key });
        }
    }
}

#[cfg(test)]
#[path = "friend_sync_tests.rs"]
mod tests;
