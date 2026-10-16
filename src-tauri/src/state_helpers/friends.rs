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

/// The sender's friendship status from the authoritative store, or
/// `None` for a stranger: the receive gate's input. It reads SQLite
/// through `FriendStore`, not the in-memory `state.friends` map, so a
/// message dispatched before hydration is judged on the stored row.
/// Fails closed (`None`) before the store is wired up or on a store error.
pub async fn friend_status_authoritative(
    state: &Arc<AppState>,
    public_key: &str,
) -> Option<rekindle_types::friend_store::FriendStatus> {
    let store = state.friend_store.read().as_ref().cloned()?;
    let owner = owner_key_or_default(state);
    match store.status_of(&owner, public_key).await {
        Ok(status) => status,
        Err(e) => {
            tracing::warn!(
                target: "friend_store",
                error = %e,
                pubkey = %public_key,
                "friend_status_authoritative SQLite error; failing closed"
            );
            None
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
