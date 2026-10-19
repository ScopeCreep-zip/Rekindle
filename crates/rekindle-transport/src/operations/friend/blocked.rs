//! Blocking: a blocked identity is no friend, no request and no message of
//! theirs reaches us, and our profile moves so their copy of its key is
//! dead.

use rekindle_types::dto::friends::BlockedUser;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::error::FriendError;
use super::host::FriendHost;
use super::{rotate, state};
use crate::operations::dm;

/// Whether `public_key` is blocked.
///
/// # Errors
/// The block list could not be read.
pub async fn is_blocked(host: &dyn FriendHost, public_key: &str) -> Result<bool, FriendError> {
    let owner = state::owner(host)?;
    let pk = public_key.to_string();
    state::db_call!(host, "read block list", move |conn| {
        rekindle_db::repo::blocked::is_blocked(conn, &owner, &pk)
    })
    .await
}

/// Block `public_key`: the friend and any request of theirs go, nothing
/// stays queued to them, their watch and route and session end, and our
/// profile moves to a new key.
///
/// # Errors
/// The block could not be stored, or the profile not moved.
pub async fn block(
    host: &dyn FriendHost,
    public_key: &str,
    display_name: Option<&str>,
) -> Result<(), FriendError> {
    state::require_public_key(public_key)?;
    let owner = state::owner(host)?;
    let now = host.clock().now_ms_i64();
    let name = state::friend(host, public_key)
        .map(|f| f.display_name)
        .or_else(|| display_name.map(str::to_string))
        .unwrap_or_else(|| format!("{}...", rekindle_utils::text::prefix(public_key, 12)));
    let pk = public_key.to_string();
    state::db_call!(host, "block user", move |conn| {
        rekindle_db::repo::friends::delete(conn, &owner, &pk)?;
        rekindle_db::repo::pending_requests::delete(conn, &owner, &pk)?;
        rekindle_db::repo::pending_messages::delete_to(conn, &owner, &pk)?;
        rekindle_db::repo::blocked::insert(conn, &owner, &pk, &name, now)
    })
    .await?;

    state::take(host, public_key);
    if let Ok(node) = host.node() {
        dm::route::invalidate(&node, public_key);
    }
    // Watching the presence of someone the user cut off is the worse half
    // of not blocking at all.
    host.unwatch_friend(public_key).await;
    state::forget_session(host, public_key).await;
    rotate::rotate_profile_key(host).await?;
    host.publish(SubscriptionEvent::Friend(FriendEvent::Removed {
        peer_key: public_key.to_string(),
    }));
    tracing::info!(public_key = %public_key, "user blocked and profile key rotated");
    Ok(())
}

/// Unblock `public_key`.
///
/// # Errors
/// `NotFound` when they were not blocked.
pub async fn unblock(host: &dyn FriendHost, public_key: &str) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    let pk = public_key.to_string();
    let removed = state::db_call!(host, "unblock user", move |conn| {
        rekindle_db::repo::blocked::delete(conn, &owner, &pk)
    })
    .await?;
    if !removed {
        return Err(FriendError::NotFound(format!(
            "{public_key} is not blocked"
        )));
    }
    tracing::info!(public_key = %public_key, "user unblocked");
    Ok(())
}

/// Everyone we blocked, most recent first.
///
/// # Errors
/// The block list could not be read.
pub async fn list(host: &dyn FriendHost) -> Result<Vec<BlockedUser>, FriendError> {
    let owner = state::owner(host)?;
    let rows = state::db_call!(host, "read block list", move |conn| {
        rekindle_db::repo::blocked::list(conn, &owner)
    })
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| BlockedUser {
            public_key: row.public_key,
            display_name: row.display_name,
            blocked_at: row.blocked_at,
        })
        .collect())
}
