//! Identity rotation between friends (plan E2.3, `IdentityRotated` 0x0108).
//!
//! Under identity == address a friend's key changes only when they rotate
//! their identity, so the rotation is a signed frame over the old session:
//! the old identity and the new one both sign `IDENTITY_ROTATED ‖ old ‖
//! new ‖ recipient`, binding the recipient so it cannot be replayed to
//! someone else. The receiver checks both and holds the rotation for the
//! user's review (owner ruling R1, threat model S1: never auto-accept);
//! nothing moves until they accept it. Accepting moves the friend and their
//! history to the new key, drops the old session (it is bound to the old
//! identity), and marks the new key unverified: when the old one was
//! verified, the change is recorded and escalates the next review
//! (`VerifiedKeyChanged`). The next message to the new key opens a session
//! from its bundle. Declining drops the rotation and changes nothing.
//!
//! The pending rotation lives in memory only (`FriendStores`, cleared at
//! lock): after a restart it is gone, and the friend's client re-sends it
//! on their next contact over the old session.

use rekindle_codec::capnp_codec::direct::{DirectBody, IdentityRotated};
use rekindle_crypto::{Identity, SafetyNumber};
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::{state, verification, BackendHold, FriendError, FriendHost};
use crate::operations::dm;

/// The rotation frame from `old` to `new` for `recipient`
/// (`rekindle_crypto::identity_rotation` owns the construction).
#[must_use]
pub fn rotation(old: &Identity, new: &Identity, recipient: &[u8; 32]) -> IdentityRotated {
    let (sig_old, sig_new) = rekindle_crypto::identity_rotation::sign(old, new, recipient);
    IdentityRotated {
        new_identity: new.public_key_bytes(),
        sig_old,
        sig_new,
    }
}

/// Whether `r`, from `old` to `recipient`, is signed by both identities.
#[must_use]
pub fn verified(old: &[u8; 32], recipient: &[u8; 32], r: &IdentityRotated) -> bool {
    rekindle_crypto::identity_rotation::verify(
        old,
        &r.new_identity,
        recipient,
        &r.sig_old,
        &r.sig_new,
    )
}

/// Tell every friend not being removed that we are `new` now, before the
/// identity moves; how many the notice reached.
pub async fn notify_identity_rotated(host: &dyn FriendHost, new: &Identity) -> usize {
    let Ok(local) = host.local_party() else {
        return 0;
    };
    let friends: Vec<String> = state::friends(host)
        .into_iter()
        .filter(|f| f.friendship_state == FriendshipState::Accepted)
        .map(|f| f.public_key)
        .collect();
    let mut notified = 0;
    for friend in &friends {
        let Ok(recipient) = rekindle_types::key_format::public_key_hex(friend) else {
            continue;
        };
        let notice =
            DirectBody::IdentityRotated(rotation(&local.identity, new, &recipient.to_bytes()));
        match dm::send_to_peer(host, friend, &notice).await {
            Ok(()) => notified += 1,
            Err(e) => tracing::warn!(to = %friend, error = %e, "IdentityRotated not sent"),
        }
    }
    notified
}

/// A friend rotated their identity: both signatures are checked, then
/// nothing moves. The rotation waits in [`FriendStores::pending_rotations`]
/// and the backend holds its accept for the user's TrustChanging review
/// (threat model S1: an identity-key change is never auto-accepted). Until
/// it is accepted the new key is a stranger's: the gate refuses its frames
/// and nothing is sent to it.
///
/// Whether it is done: `false` while no identity is unlocked or the hold
/// could not be placed, so the message is kept for the replay.
///
/// [`FriendStores::pending_rotations`]: super::FriendStores::pending_rotations
pub(crate) async fn on_identity_rotated(
    host: &dyn FriendHost,
    sender: &str,
    r: &IdentityRotated,
) -> bool {
    let Ok(local) = host.local_party() else {
        return false;
    };
    let Ok(old) = rekindle_types::key_format::public_key_hex(sender) else {
        return true;
    };
    if !verified(&old.to_bytes(), &local.owner(), r) {
        tracing::warn!(from = %sender, "dropping IdentityRotated: not signed by both identities");
        return true;
    }
    if !is_friend(host, sender).await {
        tracing::debug!(from = %sender, "dropping IdentityRotated from a non-friend");
        return true;
    }
    // One per friend: a newer rotation replaces the older one (and its
    // review, `hold_backend_intent`).
    host.stores()
        .pending_rotations
        .lock()
        .insert(sender.to_string(), r.new_identity);
    let new_identity = hex::encode(r.new_identity);
    if let Err(e) = host
        .hold_backend_intent(BackendHold::IdentityRotation {
            peer: sender.to_string(),
            new_identity: new_identity.clone(),
        })
        .await
    {
        host.stores().pending_rotations.lock().remove(sender);
        tracing::warn!(from = %sender, error = %e, "IdentityRotated not held — kept for the replay");
        return false;
    }
    tracing::info!(from = %sender, to = %new_identity, "friend rotated their identity; held for review");
    true
}

/// Whether `peer` is an accepted friend, by the receive gate's authority
/// (the identity's database).
async fn is_friend(host: &dyn FriendHost, peer: &str) -> bool {
    let (Ok(store), Ok(owner)) = (host.friend_store(), state::owner(host)) else {
        return false;
    };
    store.is_active_friend(&owner, peer).await.unwrap_or(false)
}

/// The safety number with `peer` now and the one with their pending new
/// identity, as the review shows them; `None` when no rotation from
/// `peer` is pending or no identity is unlocked.
#[must_use]
pub fn pending_safety_numbers(host: &dyn FriendHost, peer: &str) -> Option<(String, String)> {
    let new_key = *host.stores().pending_rotations.lock().get(peer)?;
    let (owner, _) = verification::keys(host, peer).ok()?;
    let old_number = verification::safety_number(host, peer)?;
    Some((old_number, SafetyNumber::new(&owner, &new_key).digits()))
}

/// The user accepted `peer`'s pending rotation: the friend row, history,
/// verification row and old session move to the new key in one
/// transaction (`rotate_peer`), the profile watch follows, and every
/// frontend hears `KeyChanged`. The new key is unverified; when the old
/// one was verified the change escalates the next review.
///
/// # Errors
/// `NotFound` when no rotation from `peer` is pending or they are no
/// longer a friend; the move's store error (the rotation stays pending).
pub async fn accept(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
    let new_key = host
        .stores()
        .pending_rotations
        .lock()
        .get(peer)
        .copied()
        .ok_or_else(|| FriendError::NotFound(NOT_PENDING.to_string()))?;
    let (owner, old) = verification::keys(host, peer)?;
    let now = host.clock().now_ms_i64();
    let moved = state::db_call!(host, "move rotated friend", move |conn| {
        rekindle_db::repo::peer_rekey::rotate_peer(conn, &owner, &old, &new_key, now)
    })
    .await?;
    host.stores().pending_rotations.lock().remove(peer);
    if !moved {
        return Err(FriendError::NotFound(format!("{peer} is not a friend")));
    }
    let new_hex = hex::encode(new_key);
    host.unwatch_friend(peer).await;
    if let Some(mut friend) = state::take(host, peer) {
        friend.public_key.clone_from(&new_hex);
        friend.mailbox_dht_key = None;
        let profile = friend.dht_record_key.clone();
        state::put(host, friend);
        if let Some(profile) = profile {
            host.watch_friend(&new_hex, &profile).await;
        }
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::KeyChanged {
        peer_key: peer.to_string(),
        new_key: new_hex.clone(),
    }));
    tracing::info!(from = %peer, to = %new_hex, "friend's identity rotation accepted");
    Ok(())
}

/// The user declined `peer`'s pending rotation: it is dropped and nothing
/// changes (the old friend, session and verification stay; nothing is
/// sent to the new key).
///
/// # Errors
/// `NotFound` when no rotation from `peer` is pending.
pub fn decline(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
    host.stores()
        .pending_rotations
        .lock()
        .remove(peer)
        .map(|_| tracing::info!(from = %peer, "friend's identity rotation declined"))
        .ok_or_else(|| FriendError::NotFound(NOT_PENDING.to_string()))
}

const NOT_PENDING: &str = "no identity rotation from this friend is pending";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rotation_verifies_only_for_its_recipient_and_both_signers() {
        let (old, new, bob, carol) = (
            Identity::generate(),
            Identity::generate(),
            Identity::generate(),
            Identity::generate(),
        );
        let to_bob = rotation(&old, &new, &bob.public_key_bytes());
        assert!(verified(
            &old.public_key_bytes(),
            &bob.public_key_bytes(),
            &to_bob
        ));
        // Replayed to Carol, or claimed by someone else.
        assert!(!verified(
            &old.public_key_bytes(),
            &carol.public_key_bytes(),
            &to_bob
        ));
        assert!(!verified(
            &carol.public_key_bytes(),
            &bob.public_key_bytes(),
            &to_bob
        ));
        // A forged new-side signature.
        let mut forged = to_bob.clone();
        forged.sig_new = carol.sign(&[0]).to_bytes();
        assert!(!verified(
            &old.public_key_bytes(),
            &bob.public_key_bytes(),
            &forged
        ));
    }
}
