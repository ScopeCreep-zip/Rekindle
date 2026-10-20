//! The Host's friend projection and identity, as the operations read and
//! write them. D1's rule: the repository write first, then the projection
//! (`Session.friends`).

use rekindle_types::friend::{FriendState, FriendshipState};

use super::error::FriendError;
use super::host::FriendHost;

/// The unlocked identity's public key.
///
/// # Errors
/// `Locked` while no identity is unlocked.
pub(crate) fn owner(host: &dyn FriendHost) -> Result<String, FriendError> {
    host.session()
        .read()
        .as_ref()
        .map(|s| s.identity.public_key_hex.clone())
        .ok_or(FriendError::Locked)
}

/// Our identity's name and the records peers reach us through.
pub(crate) struct OwnIdentity {
    pub public_key: String,
    pub display_name: String,
    pub profile_dht_key: String,
    pub mailbox_dht_key: String,
    pub friend_list_dht_key: String,
}

/// Our identity, as the session projection names it.
///
/// # Errors
/// `Locked` while no identity is unlocked.
pub(crate) fn own_identity(host: &dyn FriendHost) -> Result<OwnIdentity, FriendError> {
    let guard = host.session().read();
    let identity = &guard.as_ref().ok_or(FriendError::Locked)?.identity;
    Ok(OwnIdentity {
        public_key: identity.public_key_hex.clone(),
        display_name: identity.display_name.clone(),
        profile_dht_key: identity.profile_dht_key.clone(),
        mailbox_dht_key: identity.mailbox_dht_key.clone(),
        friend_list_dht_key: identity.friend_list_dht_key.clone(),
    })
}

/// One friend, any friendship state.
pub(crate) fn friend(host: &dyn FriendHost, public_key: &str) -> Option<FriendState> {
    host.session()
        .read()
        .as_ref()
        .and_then(|s| s.friends.get(public_key).cloned())
}

/// Every friend, any friendship state.
pub(crate) fn friends(host: &dyn FriendHost) -> Vec<FriendState> {
    host.session()
        .read()
        .as_ref()
        .map(|s| s.friends.values().cloned().collect())
        .unwrap_or_default()
}

/// A friend's friendship state, as the projection holds it.
pub(crate) fn friendship(host: &dyn FriendHost, public_key: &str) -> Option<FriendshipState> {
    friend(host, public_key).map(|f| f.friendship_state)
}

/// Whether `public_key` is an accepted friend.
pub(crate) fn is_accepted(host: &dyn FriendHost, public_key: &str) -> bool {
    friendship(host, public_key) == Some(FriendshipState::Accepted)
}

/// Put `friend` in the projection, replacing any row for its key.
pub(crate) fn put(host: &dyn FriendHost, friend: FriendState) {
    if let Some(session) = host.session().write().as_mut() {
        session.friends.insert(friend.public_key.clone(), friend);
    }
}

/// Change one friend in the projection; whether it was there.
pub(crate) fn update(
    host: &dyn FriendHost,
    public_key: &str,
    change: impl FnOnce(&mut FriendState),
) -> bool {
    let mut guard = host.session().write();
    match guard.as_mut().and_then(|s| s.friends.get_mut(public_key)) {
        Some(friend) => {
            change(friend);
            true
        }
        None => false,
    }
}

/// Take one friend out of the projection.
pub(crate) fn take(host: &dyn FriendHost, public_key: &str) -> Option<FriendState> {
    host.session()
        .write()
        .as_mut()
        .and_then(|s| s.friends.remove(public_key))
}

/// A friend's name for a notice: their display name, or a short key.
pub(crate) fn label(host: &dyn FriendHost, public_key: &str) -> String {
    friend(host, public_key).map_or_else(
        || format!("{}…", rekindle_utils::text::prefix(public_key, 16)),
        |f| f.display_name,
    )
}

/// Run a repository call on the identity's database and wait for it: a
/// future of `Result<R, FriendError>`, `Locked` while no identity is
/// unlocked and `Failed` when the call fails. A macro so the closure's
/// connection type is inferred from `Db::call` and the transport never
/// names the driver (`check-sqlite`).
macro_rules! db_call {
    ($host:expr, $what:expr, $function:expr) => {
        async {
            $host
                .db()?
                .call($function)
                .await
                .map_err(|e| $crate::operations::friend::FriendError::store($what, e))
        }
    };
}
pub(crate) use db_call;

/// Queue a repository write on the identity's database; a failure is
/// logged (see [`db_call`]).
macro_rules! db_fire {
    ($host:expr, $what:expr, $function:expr) => {
        match $host.db() {
            Ok(db) => db.fire($what, $function),
            Err(e) => {
                tracing::debug!(what = $what, error = %e, "no database for a queued write");
            }
        }
    };
}
pub(crate) use db_fire;

/// Check that `public_key` is a 32-byte key in hex.
///
/// # Errors
/// `Invalid` when it is not.
pub(crate) fn require_public_key(public_key: &str) -> Result<(), FriendError> {
    rekindle_types::key_format::public_key_hex(public_key)
        .map(|_| ())
        .map_err(|_| {
            FriendError::Invalid(
                "Invalid public key — must be a 64-character hex string".to_string(),
            )
        })
}

/// Write one column of a friend's row and wait for it (D1: the store
/// first, then the projection); whether it was stored (logged otherwise).
pub(crate) async fn write_set(
    host: &dyn FriendHost,
    public_key: &str,
    column: rekindle_db::repo::friends::Column,
    value: Option<String>,
) -> bool {
    let Ok(owner) = owner(host) else {
        return false;
    };
    let pk = public_key.to_string();
    stored(
        db_call!(host, "update friend", move |conn| {
            rekindle_db::repo::friends::set(conn, &owner, &pk, column, value)
        })
        .await,
    )
}

/// Whether an awaited write landed; a failure is logged.
fn stored(result: Result<(), FriendError>) -> bool {
    match result {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(error = %e, "friend write failed; the projection is left as stored");
            false
        }
    }
}

/// End every ratchet session with `peer` (an unfriend, a block, a
/// removal): a later message to them opens a new one. Logged on failure.
pub(crate) async fn forget_session(host: &dyn FriendHost, peer: &str) {
    let (Ok(db), Ok(local), Ok(theirs)) = (
        host.db(),
        host.local_party(),
        rekindle_types::key_format::public_key_hex(peer),
    ) else {
        return;
    };
    if let Err(e) = rekindle_db::ratchet::forget(&db, &local.owner(), &theirs.to_bytes()).await {
        tracing::error!(peer = %peer, error = %e, "session with former friend not forgotten");
    }
}

/// Delete a friend's row and their pending request, and wait for it.
pub(crate) async fn write_forget(host: &dyn FriendHost, public_key: &str) -> bool {
    let Ok(owner) = owner(host) else {
        return false;
    };
    let pk = public_key.to_string();
    stored(
        db_call!(host, "forget friend", move |conn| {
            rekindle_db::repo::friends::delete(conn, &owner, &pk)?;
            rekindle_db::repo::pending_requests::delete(conn, &owner, &pk)
        })
        .await,
    )
}

/// Delete everything queued to `public_key`, and wait for it.
pub(crate) async fn write_drop_queued(host: &dyn FriendHost, public_key: &str) -> bool {
    let Ok(owner) = owner(host) else {
        return false;
    };
    let pk = public_key.to_string();
    stored(
        db_call!(host, "delete pending messages", move |conn| {
            rekindle_db::repo::pending_messages::delete_to(conn, &owner, &pk)
        })
        .await,
    )
}
