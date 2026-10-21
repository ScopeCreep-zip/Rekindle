//! Our friend-presence key and our friends' (plan E2.5): profile subkeys 2
//! (STATUS) and 4 (GAME_INFO) are sealed under ours
//! (`rekindle_secrets::friend_presence`), sent to each accepted friend as a
//! ratcheted `FriendsKey`. A new epoch on every unfriend or block, either
//! way, goes to the friends who remain; the removed peer's key of ours is
//! dead and theirs is forgotten.

use rekindle_codec::capnp_codec::direct::{DirectBody, FriendsKey};
use rekindle_secrets::friend_presence::PresenceKey;
use rekindle_types::friend::FriendshipState;

use super::error::FriendError;
use super::host::FriendHost;
use super::state;
use crate::operations::dm;

fn ours(host: &dyn FriendHost) -> Option<PresenceKey> {
    host.session()
        .read()
        .as_ref()
        .and_then(|s| s.presence_key.clone())
}

/// Send our current key to `peer` (their friendship was just accepted).
/// A failure is logged: the next rotation sends again.
pub(crate) async fn send(host: &dyn FriendHost, peer: &str) {
    let Some(key) = ours(host) else {
        tracing::warn!(to = %peer, "no presence key to send");
        return;
    };
    let body = DirectBody::FriendsKey(FriendsKey {
        epoch: key.epoch(),
        key: *key.as_bytes(),
    });
    if let Err(e) = dm::send_to_peer(host, peer, &body).await {
        tracing::warn!(to = %peer, error = %e, "failed to send FriendsKey");
    }
}

/// Mint the next epoch, store it, and send it to every accepted friend
/// but `removed`; `removed`'s key of theirs is forgotten.
///
/// # Errors
/// No current key, the epochs are exhausted, or the store failed.
pub(crate) async fn rotate(host: &dyn FriendHost, removed: &str) -> Result<(), FriendError> {
    forget(host, removed);
    let owner = state::owner(host)?;
    let next = ours(host)
        .and_then(|key| key.next(host.rng()))
        .ok_or_else(|| FriendError::NotFound("no presence key to rotate".into()))?;
    let (epoch, bytes, now) = (
        next.epoch(),
        next.as_bytes().to_vec(),
        host.clock().now_ms_i64(),
    );
    state::db_call!(host, "store presence key", move |conn| {
        rekindle_db::repo::presence_keys::replace(conn, &owner, epoch, &bytes, now)
    })
    .await?;
    if let Some(session) = host.session().write().as_mut() {
        session.presence_key = Some(next);
    }
    let friends: Vec<String> = state::friends(host)
        .into_iter()
        .filter(|f| f.friendship_state == FriendshipState::Accepted && f.public_key != removed)
        .map(|f| f.public_key)
        .collect();
    for friend in &friends {
        send(host, friend).await;
    }
    host.reseal_own_presence().await;
    tracing::info!(epoch, sent = friends.len(), "presence key rotated");
    Ok(())
}

/// Forget `peer`'s key (they are no friend now).
pub(crate) fn forget(host: &dyn FriendHost, peer: &str) {
    if let Some(session) = host.session().write().as_mut() {
        session.friend_presence_keys.remove(peer);
    }
}

/// A friend's `FriendsKey`: stored for an accepted friend only, and never
/// over a newer epoch. Whether it is done (`false`: the store failed).
pub(crate) async fn on_friends_key(host: &dyn FriendHost, sender: &str, body: &FriendsKey) -> bool {
    if !state::is_accepted(host, sender) {
        tracing::debug!(from = %sender, "ignoring FriendsKey from non-friend");
        return true;
    }
    let Ok(owner) = state::owner(host) else {
        return false;
    };
    let (pk, epoch, bytes) = (sender.to_string(), body.epoch, body.key.to_vec());
    let stored = state::db_call!(host, "store friend presence key", move |conn| {
        rekindle_db::repo::friends::set_presence_key(conn, &owner, &pk, epoch, &bytes)
    })
    .await;
    match stored {
        Ok(true) => {
            if let Some(session) = host.session().write().as_mut() {
                session
                    .friend_presence_keys
                    .insert(sender.to_string(), PresenceKey::new(body.epoch, body.key));
            }
            tracing::debug!(from = %sender, epoch, "friend presence key stored");
            true
        }
        Ok(false) => {
            tracing::debug!(from = %sender, epoch, "older friend presence key ignored");
            true
        }
        Err(e) => {
            tracing::warn!(from = %sender, error = %e, "friend presence key not stored");
            false
        }
    }
}
