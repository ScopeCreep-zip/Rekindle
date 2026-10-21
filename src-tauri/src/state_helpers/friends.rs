//! Friend-state accessors over the Host's projection (plan E2.1): the
//! unported readers' one view of our friends, read-only, until each
//! reader's own step moves it into the Host.

use std::sync::Arc;

use crate::state::{AppState, FriendState};

/// Generic friend field extractor.
pub fn friend_field<T>(
    state: &AppState,
    key: &str,
    f: impl FnOnce(&FriendState) -> Option<T>,
) -> Option<T> {
    let friend = state.host.current()?.friend(key)?;
    f(&friend)
}

/// Check if a person is one of our friends, any friendship state.
pub fn is_friend(state: &Arc<AppState>, public_key: &str) -> bool {
    friend_field(state, public_key, |_| Some(())).is_some()
}

/// Friend's display name.
pub fn friend_display_name(state: &Arc<AppState>, key: &str) -> Option<String> {
    friend_field(state, key, |f| Some(f.display_name.clone()))
}

/// A watched record changed: when it is a friend's profile, the Host's
/// friend presence applies it (plan E2.1).
pub fn friend_presence_change(state: &AppState, record_key: &str, subkeys: &[u32], value: &[u8]) {
    use rekindle_presence::FriendPresenceDeps as _;
    let Some(host) = state.host.current() else {
        return;
    };
    let presence = host.friend_presence();
    if presence.friend_for_dht_key(record_key).is_some() {
        rekindle_presence::handle_value_change(presence.as_ref(), record_key, subkeys, value);
    }
}
