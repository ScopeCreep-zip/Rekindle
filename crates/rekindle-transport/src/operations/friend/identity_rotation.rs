//! Identity rotation between friends (plan E2.3, `IdentityRotated` 0x0108).
//!
//! Under identity == address a friend's key changes only when they rotate
//! their identity, so the rotation is a signed frame over the old session:
//! the old identity and the new one both sign `IDENTITY_ROTATED ‖ old ‖
//! new ‖ recipient`, binding the recipient so it cannot be replayed to
//! someone else. The receiver checks both, moves the friend and their
//! history to the new key, drops the old session (it is bound to the old
//! identity), and marks the new key unverified: when the old one was
//! verified, the change is recorded and escalates the next review
//! (`VerifiedKeyChanged`). The next message to the new key opens a session
//! from its bundle.

use rekindle_codec::capnp_codec::direct::{DirectBody, IdentityRotated};
use rekindle_crypto::Identity;
use rekindle_types::domains;
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::{state, FriendHost};
use crate::operations::dm;

/// What both identities sign for `recipient`.
#[must_use]
pub fn signing_bytes(old: &[u8; 32], new: &[u8; 32], recipient: &[u8; 32]) -> Vec<u8> {
    let label = domains::IDENTITY_ROTATED.as_bytes();
    let mut bytes = Vec::with_capacity(label.len() + 96);
    bytes.extend_from_slice(label);
    bytes.extend_from_slice(old);
    bytes.extend_from_slice(new);
    bytes.extend_from_slice(recipient);
    bytes
}

/// The rotation frame from `old` to `new` for `recipient`.
#[must_use]
pub fn rotation(old: &Identity, new: &Identity, recipient: &[u8; 32]) -> IdentityRotated {
    let bytes = signing_bytes(&old.public_key_bytes(), &new.public_key_bytes(), recipient);
    IdentityRotated {
        new_identity: new.public_key_bytes(),
        sig_old: old.sign(&bytes).to_bytes(),
        sig_new: new.sign(&bytes).to_bytes(),
    }
}

/// Whether `r`, from `old` to `recipient`, is signed by both identities.
#[must_use]
pub fn verified(old: &[u8; 32], recipient: &[u8; 32], r: &IdentityRotated) -> bool {
    if *old == r.new_identity {
        return false;
    }
    let bytes = signing_bytes(old, &r.new_identity, recipient);
    let signed_by = |key: &[u8; 32], signature: &[u8; 64]| {
        ed25519_dalek::VerifyingKey::from_bytes(key).is_ok_and(|key| {
            Identity::verify(
                &key,
                &bytes,
                &ed25519_dalek::Signature::from_bytes(signature),
            )
            .is_ok()
        })
    };
    signed_by(old, &r.sig_old) && signed_by(&r.new_identity, &r.sig_new)
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

/// A friend rotated their identity.
pub(crate) async fn on_identity_rotated(host: &dyn FriendHost, sender: &str, r: &IdentityRotated) {
    let (Ok(local), Ok(old)) = (
        host.local_party(),
        rekindle_types::key_format::public_key_hex(sender),
    ) else {
        return;
    };
    let old = old.to_bytes();
    if !verified(&old, &local.owner(), r) {
        tracing::warn!(from = %sender, "dropping IdentityRotated: not signed by both identities");
        return;
    }
    let new_hex = hex::encode(r.new_identity);
    let (owner, new_key, now) = (local.owner(), r.new_identity, host.clock().now_ms_i64());
    // The friend row, the history, the verification row and the old
    // session move in one transaction; their old mailbox key is dropped.
    let moved = state::db_call!(host, "move rotated friend", move |conn| {
        rekindle_db::repo::peer_rekey::rotate_peer(conn, &owner, &old, &new_key, now)
    })
    .await;
    match moved {
        Ok(true) => {}
        Ok(false) => {
            tracing::debug!(from = %sender, "IdentityRotated from a non-friend — nothing moved");
            return;
        }
        Err(e) => {
            tracing::error!(from = %sender, error = %e, "rotated friend not moved");
            return;
        }
    }
    host.unwatch_friend(sender).await;
    if let Some(mut friend) = state::take(host, sender) {
        friend.public_key.clone_from(&new_hex);
        friend.mailbox_dht_key = None;
        let profile = friend.dht_record_key.clone();
        state::put(host, friend);
        if let Some(profile) = profile {
            host.watch_friend(&new_hex, &profile).await;
        }
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::KeyChanged {
        peer_key: sender.to_string(),
        new_key: new_hex.clone(),
    }));
    tracing::info!(from = %sender, to = %new_hex, "friend rotated their identity");
}

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
