//! 1:1 over real Hosts (plan E2.3): two identities befriend each other and
//! message both ways; then one rotates its identity, and both message
//! again. Neither holds a session for the new identity after the move, so
//! the first message each way opens one from the other's published bundle
//! (`dm/frame.rs`, `published_bundle`).

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;

use crate::harness::{eventually, start, SimNode, PASSPHRASE};

const LIMIT: Duration = Duration::from_secs(180);

async fn friends(node: &SimNode) -> Vec<rekindle_types::dto::friends::Friend> {
    serde_json::from_value(node.ok(IpcRequest::FriendList).await).expect("the friend list")
}

async fn is_accepted_friend(node: &SimNode, key: &str) -> bool {
    friends(node).await.iter().any(|f| {
        f.public_key == key
            && f.friendship_state == rekindle_types::friend::FriendshipState::Accepted
    })
}

async fn has_request_from(node: &SimNode, key: &str) -> bool {
    let requests: Vec<rekindle_types::dto::friends::PendingRequest> =
        serde_json::from_value(node.ok(IpcRequest::FriendRequests).await).expect("the requests");
    requests.iter().any(|r| r.public_key == key)
}

async fn holds_message(node: &SimNode, peer: &str, body: &str) -> bool {
    let history: Vec<rekindle_types::dto::dm::DmMessage> = serde_json::from_value(
        node.ok(IpcRequest::DmHistory {
            peer_key: peer.to_string(),
            limit: 50,
            before: None,
        })
        .await,
    )
    .expect("the history");
    history.iter().any(|m| m.body == body && !m.is_own)
}

async fn send(node: &SimNode, peer: &str, body: &str) {
    node.ok(IpcRequest::DmSend {
        peer_key: peer.to_string(),
        body: body.to_string(),
        reply_to: None,
    })
    .await;
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
    let invite: rekindle_types::dto::friends::InviteCreated =
        serde_json::from_value(nodes[1].ok(IpcRequest::FriendInviteCreate).await)
            .expect("the invite");
    nodes[0]
        .ok(IpcRequest::FriendAddFromInvite { invite: invite.url })
        .await;
    assert!(eventually(&sim, LIMIT, || has_request_from(&nodes[1], &ann)).await);
    nodes[1]
        .ok(IpcRequest::FriendAccept {
            public_key: ann.clone(),
            display_name: "Ann".into(),
        })
        .await;
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &bea)).await);
    assert!(is_accepted_friend(&nodes[1], &ann).await);

    send(&nodes[0], &bea, "ann to bea").await;
    assert!(eventually(&sim, LIMIT, || holds_message(&nodes[1], &ann, "ann to bea")).await);
    send(&nodes[1], &ann, "bea to ann").await;
    assert!(eventually(&sim, LIMIT, || holds_message(&nodes[0], &bea, "bea to ann")).await);

    let mailbox_before = nodes[0].ok(IpcRequest::IdentityShow).await["mailboxDhtKey"]
        .as_str()
        .map(str::to_string);
    // Ann rotates: Bea hears it under the old key and moves her; Ann's
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
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[1], &new_ann)).await);
    assert!(!is_accepted_friend(&nodes[1], &ann).await);
    // The conversation moved with her, and her old mailbox did not.
    assert!(holds_message(&nodes[1], &new_ann, "ann to bea").await);
    let shown = nodes[0].ok(IpcRequest::IdentityShow).await;
    let mailbox = shown["mailboxDhtKey"]
        .as_str()
        .expect("a mailbox")
        .to_string();
    assert!(!mailbox.is_empty());
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
