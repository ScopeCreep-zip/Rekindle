//! Reading the buddy list and the requests waiting for our answer.

use rekindle_types::dto::friends::{Friend, PendingRequest};
use rekindle_types::friend::FriendshipState;

use super::error::FriendError;
use super::host::FriendHost;
use super::state;

/// The buddy list: every friend but those being removed, presence shown
/// only for an accepted one.
#[must_use]
pub fn friends(host: &dyn FriendHost) -> Vec<Friend> {
    let mut list: Vec<Friend> = state::friends(host)
        .iter()
        .filter(|f| f.friendship_state != FriendshipState::Removing)
        .map(Friend::of)
        .collect();
    list.sort_by(|a, b| {
        a.display_name
            .cmp(&b.display_name)
            .then(a.public_key.cmp(&b.public_key))
    });
    list
}

/// The requests waiting for our answer, oldest first.
///
/// # Errors
/// They could not be read.
pub async fn pending(host: &dyn FriendHost) -> Result<Vec<PendingRequest>, FriendError> {
    let owner = state::owner(host)?;
    let rows = state::db_call!(host, "read pending requests", move |conn| {
        rekindle_db::repo::pending_requests::list(conn, &owner)
    })
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| PendingRequest {
            public_key: row.public_key,
            display_name: row.display_name,
            message: row.message,
            received_at: row.received_at,
        })
        .collect())
}
