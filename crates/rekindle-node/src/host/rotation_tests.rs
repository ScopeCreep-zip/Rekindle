//! A friend's signed identity rotation through the Host (owner ruling
//! R1): held for review, moved only when accepted, and only to the key the
//! review named (finding 79 review, M2).

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_transport::operations::direct::replay_unprocessed;
use rekindle_transport::operations::friend::FriendHost;
use rekindle_types::intent::{Fact, IntentProof, IntentReview, ProofRequired};

use super::direct_tests::{ann, befriend, history, pending, text};
use crate::host::adapters::friends::HostFriends;
use crate::host::Host;
use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId};

/// The accept or decline of `old`'s rotation to `new`, as its hold names it.
fn answer(
    accept: bool,
    old: &rekindle_crypto::Identity,
    new: &rekindle_crypto::Identity,
) -> IpcRequest {
    let (public_key, new_key) = (
        hex::encode(old.public_key_bytes()),
        hex::encode(new.public_key_bytes()),
    );
    if accept {
        IpcRequest::IdentityRotationAccept {
            public_key,
            new_key,
        }
    } else {
        IpcRequest::IdentityRotationDecline {
            public_key,
            new_key,
        }
    }
}

async fn commit(host: &Host, review: &IntentReview) -> IpcResponse {
    host.call(
        Caller::shell(),
        IpcRequest::IntentCommit {
            intent_id: review.intent_id,
            proof: if review.class.proof() == ProofRequired::Acknowledge {
                IntentProof::Acknowledged
            } else {
                IntentProof::Commit
            },
        },
    )
    .await
}

/// A verified friend's signed rotation (0x0108) is held: nothing moves
/// until the user commits its review (owner ruling R1, threat model S1).
/// Committing moves them, their history and their verification to the new
/// key (the old key's session is gone, the verified key's change is
/// recorded); declining leaves the old friend intact.
#[tokio::test]
async fn a_signed_identity_rotation_moves_the_friend_only_when_accepted() {
    for accept in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let (host, owner) = ann(&dir).await;
        let (old, new) = (
            rekindle_crypto::Identity::generate(),
            rekindle_crypto::Identity::generate(),
        );
        rotated_to(&host, owner, &old, &new).await;
        // Held: the friend, their history and their verification stay.
        assert_eq!(
            friend_keys(&host, owner).await,
            [hex::encode(old.public_key_bytes())]
        );
        assert_eq!(history(&host, owner, old.public_key_bytes()).await, 1);
        assert!(
            identity_row(&host, owner, old.public_key_bytes())
                .await
                .unwrap()
                .verified
        );
        assert!(identity_row(&host, owner, new.public_key_bytes())
            .await
            .is_none());

        let request = answer(accept, &old, &new);
        let IpcResponse::IntentRequired(review) = host.call(Caller::shell(), request).await else {
            panic!("the rotation's answer was not held for review");
        };
        assert!(review.facts.iter().any(|f| matches!(
            f,
            Fact::SafetyNumberChanged { old_safety_number, new_safety_number }
                if old_safety_number != new_safety_number
        )));
        if accept {
            assert_eq!(
                review.class,
                rekindle_types::intent::IntentClass::TrustChanging
            );
        }
        let committed = commit(&host, &review).await;
        assert!(matches!(committed, IpcResponse::Ok { .. }), "{committed:?}");

        let (kept, gone) = if accept { (&new, &old) } else { (&old, &new) };
        assert_eq!(
            friend_keys(&host, owner).await,
            [hex::encode(kept.public_key_bytes())]
        );
        assert_eq!(history(&host, owner, kept.public_key_bytes()).await, 1);
        assert!(identity_row(&host, owner, gone.public_key_bytes())
            .await
            .is_none());
        let row = identity_row(&host, owner, kept.public_key_bytes())
            .await
            .unwrap();
        if accept {
            assert!(!row.verified);
            assert_eq!(row.changed_from_key, Some(old.public_key_bytes()));
        } else {
            assert!(row.verified);
        }
        assert!(HostFriends::of(&host.ctx)
            .stores()
            .pending_rotations
            .lock()
            .is_empty());
    }
    assert_eq!(DmTypeId::IdentityRotated.as_u16(), 0x0108);
}

/// Bob as `old`, verified, with one message, then his rotation to `new`
/// applied from the ratchet's unprocessed rows.
async fn rotated_to(
    host: &Host,
    owner: [u8; 32],
    old: &rekindle_crypto::Identity,
    new: &rekindle_crypto::Identity,
) {
    befriend(host, owner, old.public_key_bytes()).await;
    pending(host, owner, old.public_key_bytes(), 0, &text(3)).await;
    replay_unprocessed(&HostFriends::of(&host.ctx)).await;
    let old_key = old.public_key_bytes();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            use rekindle_db::repo::signal_identities as ids;
            ids::rotate(conn, &owner, &[9; 32], &old_key, 1)?;
            ids::mark_verified(conn, &owner, &old_key, 2)
        })
        .await
        .unwrap();
    let rotation =
        rekindle_transport::operations::friend::identity_rotation::rotation(old, new, &owner);
    pending(
        host,
        owner,
        old.public_key_bytes(),
        1,
        &DirectBody::IdentityRotated(rotation),
    )
    .await;
    replay_unprocessed(&HostFriends::of(&host.ctx)).await;
}

/// Ann's friends' keys.
async fn friend_keys(host: &Host, owner: [u8; 32]) -> Vec<String> {
    let o = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::friends::load_all(conn, &o))
        .await
        .unwrap()
        .into_iter()
        .map(|f| f.public_key)
        .collect()
}

/// Ann's `signal_identities` row for `peer`.
async fn identity_row(
    host: &Host,
    owner: [u8; 32],
    peer: [u8; 32],
) -> Option<rekindle_db::repo::signal_identities::SignalIdentity> {
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::signal_identities::get(conn, &owner, &peer))
        .await
        .unwrap()
}

/// A rotation replaced between the gate's commit and the apply is not
/// accepted: the request carries the key the review named, and the apply
/// refuses any other pending key (finding 79 review, M2). Nothing moves,
/// and the replacement stays pending for its own review.
#[tokio::test]
async fn a_rotation_replaced_before_the_apply_is_refused() {
    for accept in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let (host, owner) = ann(&dir).await;
        let (old, new, other) = (
            rekindle_crypto::Identity::generate(),
            rekindle_crypto::Identity::generate(),
            rekindle_crypto::Identity::generate(),
        );
        rotated_to(&host, owner, &old, &new).await;
        let old_hex = hex::encode(old.public_key_bytes());
        let IpcResponse::IntentRequired(review) =
            host.call(Caller::shell(), answer(accept, &old, &new)).await
        else {
            panic!("the rotation's answer was not held for review");
        };
        // The rotation is replaced by one to `other` after the review.
        let friends = HostFriends::of(&host.ctx);
        friends
            .stores()
            .pending_rotations
            .lock()
            .insert(old_hex.clone(), other.public_key_bytes());
        let committed = commit(&host, &review).await;
        assert!(
            !matches!(committed, IpcResponse::Ok { .. }),
            "{committed:?}"
        );
        // The apply itself, past a gate that saw `new`, refuses it too.
        let applied = host
            .ctx
            .dispatch_in_lane(
                answer(accept, &old, &new),
                &Caller::shell(),
                crate::host::gate::Acknowledged::NOTHING,
            )
            .await;
        assert!(
            matches!(applied, IpcResponse::Error { code: 409, .. }),
            "{applied:?}"
        );
        assert_eq!(
            friend_keys(&host, owner).await,
            std::slice::from_ref(&old_hex)
        );
        assert_eq!(
            friends.stores().pending_rotations.lock().get(&old_hex),
            Some(&other.public_key_bytes())
        );
    }
}
