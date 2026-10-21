//! 1:1 over real Hosts (plan E2.3): two identities befriend each other and
//! message both ways; then one rotates its identity, the other accepts the
//! rotation's review, and both message
//! again. Neither holds a session for the new identity after the move, so
//! the first message each way opens one from the other's published bundle
//! (`dm/frame.rs`, `published_bundle`).

use rekindle_ipc::protocol::IpcRequest;

use crate::harness::{eventually, start, SimNode, PASSPHRASE};
use crate::pair::{
    accept, has_request_from, holds_message, is_accepted_friend, key, request, send, LIMIT,
};

/// The unlocked identity's mailbox key, empty before the unlock made one.
async fn mailbox(node: &SimNode) -> String {
    node.ok(IpcRequest::IdentityShow).await["mailboxDhtKey"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_rotated_friend_is_messaged_both_ways() {
    let (sim, mut nodes) = start(2, 7).await;
    nodes[0].unlock_new_identity("Ann").await;
    nodes[1].unlock_new_identity("Bea").await;
    let ann = nodes[0].public_key.clone().expect("ann's key");
    let bea = nodes[1].public_key.clone().expect("bea's key");

    // Bea's invite carries her route and records; she accepts the request
    // that answers it.
    request(&sim, &nodes[0], &nodes[1]).await;
    accept(&nodes[1], &nodes[0]).await;
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &bea)).await);
    assert!(is_accepted_friend(&nodes[1], &ann).await);

    send(&nodes[0], &bea, "ann to bea").await;
    assert!(eventually(&sim, LIMIT, || holds_message(&nodes[1], &ann, "ann to bea")).await);
    send(&nodes[1], &ann, "bea to ann").await;
    assert!(eventually(&sim, LIMIT, || holds_message(&nodes[0], &bea, "bea to ann")).await);

    let mailbox_before = Some(mailbox(&nodes[0]).await);
    // Ann rotates: Bea hears it under the old key and, once she accepts
    // its review, moves her; Ann's
    // daemon is left locked and unlocks under the new key.
    let rotated = nodes[0]
        .ok(IpcRequest::IdentityRotate {
            passphrase: PASSPHRASE.into(),
        })
        .await;
    let new_ann = rotated["new_public_key"]
        .as_str()
        .expect("the new key")
        .to_string();
    assert_ne!(new_ann, ann);
    nodes[0]
        .ok(IpcRequest::Unlock {
            identity: new_ann.clone(),
            passphrase: PASSPHRASE.into(),
        })
        .await;
    // Bea's Host holds the rotation for her review (owner ruling R1):
    // nothing moves until she accepts it.
    assert!(is_accepted_friend(&nodes[1], &ann).await);
    assert!(!is_accepted_friend(&nodes[1], &new_ann).await);
    assert!(
        eventually(&sim, LIMIT, || async {
            nodes[1]
                .call(IpcRequest::IdentityRotationAccept {
                    public_key: ann.clone(),
                })
                .await
                .is_ok()
        })
        .await
    );
    assert!(is_accepted_friend(&nodes[1], &new_ann).await);
    assert!(!is_accepted_friend(&nodes[1], &ann).await);
    // The conversation moved with her, and her old mailbox did not.
    assert!(holds_message(&nodes[1], &new_ann, "ann to bea").await);
    // The unlock's own-records stage creates the new mailbox.
    assert!(
        eventually(&sim, LIMIT, || async {
            !mailbox(&nodes[0]).await.is_empty()
        })
        .await
    );
    let mailbox = mailbox(&nodes[0]).await;
    assert_ne!(Some(mailbox), mailbox_before);

    send(&nodes[1], &new_ann, "bea to new ann").await;
    assert!(
        eventually(&sim, LIMIT, || holds_message(
            &nodes[0],
            &bea,
            "bea to new ann"
        ))
        .await
    );
    send(&nodes[0], &bea, "new ann to bea").await;
    assert!(
        eventually(&sim, LIMIT, || holds_message(
            &nodes[1],
            &new_ann,
            "new ann to bea"
        ))
        .await
    );
}

/// An invite is single use (plan E2.5, D17): the first request written to
/// its inbox is admitted and the inbox is dropped, so a second holder of
/// the same link — whose write still lands, it holds the writer — is never
/// admitted.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_invite_admits_one_requester() {
    let (sim, mut nodes) = start(3, 11).await;
    nodes[0].unlock_new_identity("Ann").await;
    nodes[1].unlock_new_identity("Bea").await;
    nodes[2].unlock_new_identity("Cat").await;
    let (ann, cat) = (key(&nodes[0]), key(&nodes[2]));
    let invite: rekindle_types::dto::friends::InviteCreated =
        serde_json::from_value(nodes[1].ok(IpcRequest::FriendInviteCreate).await)
            .expect("the invite");
    nodes[0]
        .ok(IpcRequest::FriendAddFromInvite {
            invite: invite.url.clone(),
        })
        .await;
    assert!(eventually(&sim, LIMIT, || has_request_from(&nodes[1], &ann)).await);
    // The inbox holds Ann's request: a second redeemer's write is refused
    // and they see it.
    assert!(nodes[2]
        .call(IpcRequest::FriendAddFromInvite { invite: invite.url })
        .await
        .is_err());
    assert!(!eventually(&sim, LIMIT, || has_request_from(&nodes[1], &cat)).await);
    assert!(has_request_from(&nodes[1], &ann).await);
}
