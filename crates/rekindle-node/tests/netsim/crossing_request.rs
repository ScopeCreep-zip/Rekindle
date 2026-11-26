//! Crossing requests where the higher side's request is lost (RV2,
//! finding 56 corrected): the lower side's request reaching the higher
//! side proves nothing about the higher side's, so the higher side keeps
//! sending its own until it lands. The lower side then sees the crossing,
//! answers with its accept, and both sides are friends.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;

use crate::harness::{eventually, start, SimNode};
use crate::pair::{has_request_from, is_accepted_friend, key, LIMIT};

async fn add_by_code(from: &SimNode, to: &SimNode) {
    let code: rekindle_types::dto::friends::FriendCodeView =
        serde_json::from_value(to.ok(IpcRequest::FriendCode).await).expect("the code");
    from.ok(IpcRequest::FriendAddFromCode {
        code: code.url,
        message: "hi".to_string(),
    })
    .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_request_of_the_higher_side_is_sent_again_after_the_crossing() {
    let (sim, mut nodes) = start(2, 41).await;
    for (node, name) in nodes.iter_mut().zip(["Cal", "Dee"]) {
        node.unlock_new_identity(name).await;
    }
    let (high, low) = if key(&nodes[0]) > key(&nodes[1]) {
        (0, 1)
    } else {
        (1, 0)
    };
    let (high_key, low_key) = (key(&nodes[high]), key(&nodes[low]));
    // Everything the higher side sends is lost: its request never arrives.
    sim.fault(Fault::DropFrom {
        node: high,
        per_mille: 1000,
    });
    add_by_code(&nodes[high], &nodes[low]).await;
    add_by_code(&nodes[low], &nodes[high]).await;
    assert!(
        eventually(&sim, LIMIT, || is_accepted_friend(&nodes[high], &low_key)).await,
        "the higher side took the crossing and awaits the accept"
    );
    sim.fault(Fault::DropFrom {
        node: high,
        per_mille: 0,
    });
    assert!(
        eventually(&sim, LIMIT * 20, || is_accepted_friend(
            &nodes[low],
            &high_key
        ))
        .await,
        "the higher side's request was sent again, so the lower side accepted"
    );
}

/// The same rule for an invite (RV2): the lower side holds the higher
/// side's request, then redeems the higher side's invite. It accepts the
/// held request instead of sending its own: the higher side, seeing its
/// own request crossed, would await an accept the lower side never sends.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn redeeming_the_invite_of_a_peer_whose_request_we_hold_accepts_it() {
    let (sim, mut nodes) = start(2, 43).await;
    for (node, name) in nodes.iter_mut().zip(["Eve", "Fay"]) {
        node.unlock_new_identity(name).await;
    }
    let (high, low) = if key(&nodes[0]) > key(&nodes[1]) {
        (0, 1)
    } else {
        (1, 0)
    };
    let (high_key, low_key) = (key(&nodes[high]), key(&nodes[low]));
    let invite: rekindle_types::dto::friends::InviteCreated =
        serde_json::from_value(nodes[high].ok(IpcRequest::FriendInviteCreate).await)
            .expect("the invite");
    add_by_code(&nodes[high], &nodes[low]).await;
    assert!(
        eventually(&sim, LIMIT, || has_request_from(&nodes[low], &high_key)).await,
        "the lower side holds the higher side's request"
    );
    nodes[low]
        .ok(IpcRequest::FriendAddFromInvite { invite: invite.url })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async {
            is_accepted_friend(&nodes[low], &high_key).await
                && is_accepted_friend(&nodes[high], &low_key).await
        })
        .await,
        "the held request was accepted, and both sides are friends"
    );
}
