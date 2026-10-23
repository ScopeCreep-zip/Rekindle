//! A request after the target's routes died (live A/B 2026-10-23, where a
//! request over the route a peer had before its restart took 2 min 16 s
//! of attempts to land): the target allocates and publishes a new route,
//! and the request still reaches it promptly. A request call whose cached
//! route fails calls again over the one the profile holds
//! (`rekindle-transport` `dm::send::call_datagram_answer`).

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;

use crate::harness::{eventually, start};
use crate::pair::{has_request_from, key};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_request_over_a_dead_route_is_called_again_over_the_profiles() {
    let (sim, mut nodes) = start(2, 67).await;
    for (node, name) in nodes.iter_mut().zip(["Tia", "Ola"]) {
        node.unlock_new_identity(name).await;
    }
    let invite: rekindle_types::dto::friends::InviteCreated =
        serde_json::from_value(nodes[0].ok(IpcRequest::FriendInviteCreate).await)
            .expect("the invite");
    // Tia's routes die after the invite named one; she allocates and
    // publishes a new one.
    sim.fault(Fault::RouteDeath { node: 0 });
    tokio::time::sleep(Duration::from_secs(60)).await;
    nodes[1]
        .ok(IpcRequest::FriendAddFromInvite { invite: invite.url })
        .await;
    let ola = key(&nodes[1]);
    assert!(
        eventually(&sim, Duration::from_secs(30), || has_request_from(
            &nodes[0], &ola
        ))
        .await,
        "the request reached Tia over her new route within one attempt"
    );
}
