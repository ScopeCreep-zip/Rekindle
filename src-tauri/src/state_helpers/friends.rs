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
