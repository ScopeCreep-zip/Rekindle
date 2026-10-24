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
pub fn owner(host: &dyn FriendHost) -> Result<String, FriendError> {
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

/// A peer's name for a notice: the nickname we gave them, or their key's
/// fingerprint. Never a name from the peer's own payload.
pub fn label(host: &dyn FriendHost, public_key: &str) -> String {
    friend(host, public_key)
        .and_then(|f| f.nickname)
        .unwrap_or_else(|| short_key(public_key))
}

/// A key's fingerprint, or its first characters when it is not a key.
pub fn short_key(public_key: &str) -> String {
    rekindle_types::key_format::public_key_hex(public_key).map_or_else(
        |_| format!("{}…", rekindle_utils::text::prefix(public_key, 16)),
        |key| rekindle_crypto::identity::key_fingerprint(&key.to_bytes()),
    )
}

/// The name the Host itself holds for `public_key` (plan audit U-7: a
/// frontend never supplies it): their friend row's, else the name their
/// pending request carried, else a short key until their profile is read.
///
/// # Errors
/// The pending request could not be read.
pub(crate) async fn known_name(
    host: &dyn FriendHost,
    owner: &str,
    public_key: &str,
) -> Result<String, FriendError> {
    if let Some(f) = friend(host, public_key) {
        return Ok(f.display_name);
    }
    let (ok, pk) = (owner.to_string(), public_key.to_string());
    let answer = db_call!(host, "read pending request", move |conn| {
        rekindle_db::repo::pending_requests::answer(conn, &ok, &pk)
    })
    .await?;
    Ok(answer
        .and_then(|a| a.display_name)
        .unwrap_or_else(|| label(host, public_key)))
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

/// Delete a friend's row and their pending request in one transaction,
/// and wait for it.
pub(crate) async fn write_forget(host: &dyn FriendHost, public_key: &str) -> bool {
    let Ok(owner) = owner(host) else {
        return false;
    };
    let pk = public_key.to_string();
    stored(
        db_call!(host, "forget friend", move |conn| {
            rekindle_db::repo::friends::forget(conn, &owner, &pk)
        })
        .await,
    )
}

/// `public_key` removed us: their row, their request, their queue and the
/// sessions with them, in one transaction; whether it was stored.
pub(crate) async fn write_unfriended(host: &dyn FriendHost, public_key: &str) -> bool {
    let Ok(owner) = owner(host) else {
        return false;
    };
    let pk = public_key.to_string();
    stored(
        db_call!(host, "forget the friend who removed us", move |conn| {
            rekindle_db::repo::friends::unfriended(conn, &owner, &pk)
        })
        .await,
    )
}

/// Their `UnfriendedAck`: our `Unfriended` row to `public_key` settles
/// `delivered` (plan E2.5); the removal ends on the outbox's settle event.
/// Whether it was stored.
pub(crate) async fn acknowledge_unfriended(host: &dyn FriendHost, public_key: &str) -> bool {
    let id = crate::operations::dm::handshake::unfriended_client_id(public_key);
    match host.outbox() {
        Ok(outbox) => match outbox.acknowledge_client(&id).await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!(from = %public_key, error = %e, "UnfriendedAck not stored");
                false
            }
        },
        Err(e) => {
            tracing::warn!(from = %public_key, error = %e, "UnfriendedAck with no outbox");
            false
        }
    }
}

/// The friendship with `peer`, or our attempt at one, ended (`why`: a
/// cancel, a reject, a removal, a block, an expiry): our pending messages
/// to them are discarded through the outbox, each settling `failed` with
/// the reason, so none reaches a later friendship and none holds the
/// stream (live A/B findings 31, 35; Signal and Matrix drop undelivered
/// sends to a removed contact). Every ending path calls it.
///
/// # Errors
/// The outbox is not running or its storage failed.
pub(crate) async fn discard_pending_dms(
    host: &dyn FriendHost,
    peer: &str,
    why: &str,
) -> Result<(), FriendError> {
    let reason = format!("not delivered: {why}");
    let discarded = host.outbox()?.discard_to(peer, &reason).await?;
    if discarded > 0 {
        tracing::info!(peer = %peer, discarded, why, "pending messages discarded");
    }
    Ok(())
}

/// A new friendship with `peer` starts (our request is about to be
/// committed, or our accept): every handshake row to them still pending
/// from a friendship that ended (a pending `Unfriended`, an old
/// `FriendsKey`) settles `failed` through the outbox's discard, so a stale
/// removal framed under the ended session can never end the new
/// friendship (live A/B finding 38). Called before the new friendship's
/// own rows are committed, so none of them is discarded.
///
/// # Errors
/// The outbox is not running or its storage failed.
pub(crate) async fn settle_ended_handshakes(
    host: &dyn FriendHost,
    peer: &str,
) -> Result<(), FriendError> {
    let reason = "superseded by a new friendship";
    let settled = host.outbox()?.discard_handshakes_to(peer, reason).await?;
    if settled > 0 {
        tracing::info!(peer = %peer, settled, "handshake rows of an ended friendship settled");
    }
    Ok(())
}

/// The friendship with `peer`, whom we asked, opened (their `FriendAccept`,
/// or their crossing request): the answer proves our request arrived, so
/// our pending `FriendRequest` row to them settles `delivered` through the
/// outbox even when every receipt of it was lost, and stops heading the
/// handshake stream, so our `FriendsKey` behind it goes (live A/B findings
/// 56, 60).
pub(crate) async fn settle_answered_request(host: &dyn FriendHost, peer: &str) {
    let prefix = crate::operations::dm::handshake::client_prefix(
        rekindle_codec::capnp_codec::direct::DmTypeId::FriendRequest,
    );
    let settled = match host.outbox() {
        Ok(outbox) => outbox.receipt_handshakes_to(peer, &prefix).await,
        Err(e) => Err(e),
    };
    match settled {
        Ok(0) => {}
        Ok(settled) => tracing::info!(peer = %peer, settled, "our answered request settled"),
        Err(e) => tracing::warn!(peer = %peer, error = %e, "our answered request not settled"),
    }
}
