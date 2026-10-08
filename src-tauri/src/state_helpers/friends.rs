//! Friend-state accessors.

use std::sync::Arc;

use crate::state::{AppState, FriendState, FriendshipState};

use super::identity::owner_key_or_default;

/// Check if a friend has `Accepted` friendship state.
pub fn is_friend_accepted(state: &Arc<AppState>, public_key: &str) -> bool {
    state
        .friends
        .read()
        .get(public_key)
        .is_some_and(|f| f.friendship_state == FriendshipState::Accepted)
}

/// Check if a person is in the friends map at all.
pub fn is_friend(state: &Arc<AppState>, public_key: &str) -> bool {
    state.friends.read().contains_key(public_key)
}

/// Phase 2 Track A — SQLite-authoritative receive-path friend gate.
///
/// Replaces [`is_friend`] in the Veilid receive-dispatch path so the
/// authorization decision reads the source-of-truth `friends` table rather
/// than the in-memory `state.friends` map (which suffered a hydration race:
/// the dispatch loop spawned at app boot, before login-time
/// `load_friends_from_db`).
///
/// Returns `true` only for `FriendshipState::Accepted` (mapped to
/// [`rekindle_transport::FriendStatus::Active`]). Fails closed on:
/// - missing `friend_store` wire-up (defensive — should be wired at setup)
/// - SQLite errors
/// - unknown sender
///
/// Hot path: ~75-150 µs per call. See `.claude/plans/phase-2-dht-inbox-pivot.md`
/// Track A for the latency analysis and the `feedback_backend_owns_policy.md`
/// rationale (no in-memory cache, SQLite is authoritative).
pub async fn is_active_friend_authoritative(state: &Arc<AppState>, public_key: &str) -> bool {
    let store = {
        let guard = state.friend_store.read();
        guard.as_ref().cloned()
    };
    let Some(store) = store else {
        tracing::warn!(
            target: "friend_store",
            "is_active_friend_authoritative called before friend_store wire-up; failing closed"
        );
        return false;
    };
    let owner = owner_key_or_default(state);
    match store.is_active_friend(&owner, public_key).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                target: "friend_store",
                error = %e,
                pubkey = %public_key,
                "is_active_friend_authoritative SQLite error; failing closed"
            );
            false
        }
    }
}

/// Generic friend field extractor.
pub fn friend_field<T>(
    state: &Arc<AppState>,
    key: &str,
    f: impl FnOnce(&FriendState) -> Option<T>,
) -> Option<T> {
    state.friends.read().get(key).and_then(f)
}

/// Friend's DHT record key.
pub fn friend_dht_key(state: &Arc<AppState>, key: &str) -> Option<String> {
    friend_field(state, key, |f| f.dht_record_key.clone())
}

/// Friend's display name.
pub fn friend_display_name(state: &Arc<AppState>, key: &str) -> Option<String> {
    friend_field(state, key, |f| Some(f.display_name.clone()))
}

/// Friend's mailbox DHT key.
pub fn friend_mailbox_key(state: &Arc<AppState>, key: &str) -> Option<String> {
    friend_field(state, key, |f| f.mailbox_dht_key.clone())
}

/// Collect all accepted friend keys.
pub fn accepted_friend_keys(state: &Arc<AppState>) -> Vec<String> {
    state
        .friends
        .read()
        .values()
        .filter(|f| f.friendship_state == FriendshipState::Accepted)
        .map(|f| f.public_key.clone())
        .collect()
}

/// Collect friends with DHT record keys (for sync/presence).
pub fn friends_with_dht_keys(state: &Arc<AppState>) -> Vec<(String, String)> {
    state
        .friends
        .read()
        .values()
        .filter_map(|f| {
            f.dht_record_key
                .as_ref()
                .map(|k| (f.public_key.clone(), k.clone()))
        })
        .collect()
}
