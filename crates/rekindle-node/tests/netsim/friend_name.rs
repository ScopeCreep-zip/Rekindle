//! A friend's display name (live A/B finding 39): it comes from their
//! profile record's subkey 0, read once their profile is watched and on
//! each change, so a friend added by code shows the name they chose, not
//! their fingerprint, on both sides.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use crate::harness::{eventually, start, SimNode};
use crate::pair::{friends, has_request_from, is_accepted_friend, key, LIMIT};

async fn shows(node: &SimNode, peer: &str, name: &str) -> bool {
    friends(node)
        .await
        .iter()
        .any(|f| f.public_key == peer && f.display_name == name)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn both_sides_show_the_names_chosen_after_an_add_by_code() {
    let (sim, mut nodes) = start(2, 97).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    let events = nodes[0].events().await;
    let code: rekindle_types::dto::friends::FriendCodeView =
        serde_json::from_value(nodes[1].ok(IpcRequest::FriendCode).await).expect("the code");
    nodes[0]
        .ok(IpcRequest::FriendAddFromCode {
            code: code.url,
            message: "hi".to_string(),
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || has_request_from(&nodes[1], &rae)).await,
        "Ann holds the request"
    );
    nodes[1]
        .ok(IpcRequest::FriendAccept {
            public_key: rae.clone(),
        })
        .await;
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await);
    assert!(
        eventually(&sim, LIMIT, || shows(&nodes[0], &ann, "Ann")).await,
        "Rae shows Ann's chosen name"
    );
    assert!(
        eventually(&sim, LIMIT, || shows(&nodes[1], &rae, "Rae")).await,
        "Ann shows Rae's chosen name"
    );
    assert!(
        events.any(|event| matches!(
            event,
            SubscriptionEvent::Friend(FriendEvent::DisplayNameChanged { peer_key, display_name })
                if *peer_key == ann && display_name == "Ann"
        )),
        "Rae's frontends were told Ann's name"
    );
}
