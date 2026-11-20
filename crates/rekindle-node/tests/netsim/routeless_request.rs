//! A friend request committed while the requester holds no route of its
//! own and cannot read the target's profile (Linux run 12, live finding
//! LF5: A's Add Friend review was committed while A's General route had
//! died and its replacement was not up, and nothing followed: no outbox
//! row, no call, no retry). Rae's public internet turns unready and Rae's
//! routes die, so no route can be allocated (Veilid 0.5.8 `api.rs:832-836`)
//! and every DHT read answers "try again"; Rae commits the add anyway.
//! The request is committed to Rae's `friend_handshake` stream at once,
//! carrying no route, and lands on Ann once Rae's internet returns: each
//! attempt resolves Ann's route from the profile her code names
//! (`rekindle-transport` `dm::send::call_datagram_answer`).
//!
//! Fails without the change: the add read Ann's route from her profile
//! and waited for Rae's own route before storing the request, so it was
//! refused (`NotReady`) and nothing was left to send it.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;

use crate::harness::{eventually, start};
use crate::pair::{has_request_from, key, LIMIT};

/// Rae unready long enough for every wait of the add to pass.
const UNREADY: Duration = Duration::from_secs(30);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_request_committed_without_a_route_of_our_own_lands_once_we_are_back() {
    let (sim, mut nodes) = start(2, 52).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let code: rekindle_types::dto::friends::FriendCodeView =
        serde_json::from_value(nodes[1].ok(IpcRequest::FriendCode).await).expect("the code");
    sim.fault(Fault::InternetReady {
        node: 0,
        ready: false,
    });
    sim.fault(Fault::RouteDeath { node: 0 });
    nodes[0]
        .ok(IpcRequest::FriendAddFromCode {
            code: code.url,
            message: "hi".to_string(),
        })
        .await;
    let rae = key(&nodes[0]);
    assert!(
        !eventually(&sim, UNREADY, || has_request_from(&nodes[1], &rae)).await,
        "nothing reaches Ann while Rae is unready"
    );
    sim.fault(Fault::InternetReady {
        node: 0,
        ready: true,
    });
    assert!(
        eventually(&sim, LIMIT, || has_request_from(&nodes[1], &rae)).await,
        "Rae's committed request reached Ann once Rae was back"
    );
}
