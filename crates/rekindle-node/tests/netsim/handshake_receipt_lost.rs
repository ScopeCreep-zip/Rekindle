//! A friend request's receipt lost on the reply leg (plan RV2, evidence
//! source 5): the request goes by `app_call` (Rae adds Ann by her friend
//! code) and reaches Ann, who stores it and answers the receipt, but every
//! answer of Ann's is lost on its way back (`Fault::AnswerLost`: Veilid
//! hands the answer to the reply route's first hop and tells the answerer
//! nothing more, fork `rpc_processor/mod.rs:2328-2425`; the caller's wait
//! ends in `Timeout`, `:708-729`). Rae times out and re-sends from her
//! handshake stream. Ann dedups every copy (one request row), receipts it
//! again, and on the second copy of the receipted call over her General
//! route (`RECEIPT_LOST_REPEATS`) rotates that route once; further copies
//! within `REPORT_PEER_COOLDOWN` rotate nothing. Ann's accept settles Rae's
//! request. The request's durable copy in Ann's code slot (plan RV2e)
//! settles it `Sent` once it lands, which would end the re-sends, so the
//! write is held below consensus (`Fault::WriteFails`) until the receipts
//! heal: this scenario is about the call path alone.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{Fault, Writes};
use rekindle_records::port::RouteClass;
use rekindle_route::liveness::{RECEIPT_LOST_REPEATS, REPORT_PEER_COOLDOWN, REPORT_ROTATE_AGE};
use rekindle_types::message::SendState;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use crate::harness::{eventually, start, SimNode};
use crate::pair::{accept, is_accepted_friend, key, talk, LIMIT};

/// How long the lost receipts go on after the rotation: many re-sends,
/// well inside Ann's cooldown for Rae.
const AFTER_ROTATION: Duration = Duration::from_secs(30 * 60);

/// The rows of `from`'s requests that `node` holds.
async fn requests_from(node: &SimNode, from: &str) -> usize {
    let requests: Vec<rekindle_types::dto::friends::PendingRequest> =
        serde_json::from_value(node.ok(IpcRequest::FriendRequests).await).expect("the requests");
    requests.iter().filter(|r| r.public_key == from).count()
}

fn request_delivered(event: &SubscriptionEvent) -> bool {
    matches!(
        event,
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DmSendStatus { client_id, state, .. })
            if client_id.starts_with("FriendRequest:") && *state == SendState::Delivered
    )
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn lost_receipts_dedup_rotate_once_and_the_accept_settles() {
    assert_eq!(RECEIPT_LOST_REPEATS, 2);
    assert!(AFTER_ROTATION < REPORT_PEER_COOLDOWN);
    let (sim, mut nodes) = start(2, 113).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    let code: rekindle_types::dto::friends::FriendCodeView =
        serde_json::from_value(nodes[1].ok(IpcRequest::FriendCode).await).expect("the code");
    // Ann's General route is old enough for a peer's evidence to count.
    tokio::time::sleep(REPORT_ROTATE_AGE + Duration::from_secs(10)).await;
    let general = nodes[1].routes.own_id(RouteClass::General);
    assert!(general.is_some(), "Ann's General route is up");
    let rae_events = nodes[0].events().await;
    sim.fault(Fault::AnswerLost {
        node: 1,
        lost: true,
    });
    sim.fault(Fault::WriteFails {
        node: 0,
        of: Writes::Subkey {
            subkey: rekindle_types::dht_layout::request_inbox::REQUEST,
            record: None,
        },
    });
    nodes[0]
        .ok(IpcRequest::FriendAddFromCode {
            code: code.url,
            message: "hi".to_string(),
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async {
            requests_from(&nodes[1], &rae).await == 1
        })
        .await,
        "Ann holds Rae's request"
    );
    assert!(
        eventually(&sim, LIMIT * 5, || async {
            let now = nodes[1].routes.own_id(RouteClass::General);
            now.is_some() && now != general
        })
        .await,
        "Rae's re-sent copies of a receipted call rotated Ann's General route"
    );
    let rotated = nodes[1].routes.own_id(RouteClass::General);
    tokio::time::sleep(AFTER_ROTATION).await;
    assert_eq!(
        nodes[1].routes.own_id(RouteClass::General),
        rotated,
        "Rae's evidence is cooling: one rotation"
    );
    assert_eq!(
        requests_from(&nodes[1], &rae).await,
        1,
        "every copy deduped: one request row"
    );
    assert!(
        !SimNode::saw(&rae_events, request_delivered),
        "no receipt reached Rae"
    );
    sim.fault(Fault::AnswerLost {
        node: 1,
        lost: false,
    });
    sim.fault(Fault::Heal);
    accept(&nodes[1], &nodes[0]).await;
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await);
    assert!(
        eventually(&sim, LIMIT, || async {
            SimNode::saw(&rae_events, request_delivered)
        })
        .await,
        "the accept settled Rae's request"
    );
    talk(&sim, &nodes[0], &nodes[1], "after the lost receipts").await;
}
