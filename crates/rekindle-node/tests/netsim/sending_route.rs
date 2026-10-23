//! RH1r (plan E4.3 RH2, built ahead): every 1:1 receipted call is pinned to
//! our `Sending` route, wanted from unlock to lock, and its answer comes
//! back over it (veilid `rpc_processor/destination.rs:477-497`). With the
//! route live, each receipted row lands on its first call under the sim's normal
//! conditions; a lock releases the route. An invite's
//! request goes to its single-use inbox record, so the accept is the call.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Netsim;
use rekindle_records::port::{RouteClass, RouteState};

use crate::harness::{eventually, start, SimNode};
use crate::pair::{accept, has_request_from, is_accepted_friend, key, LIMIT};

fn sending(node: &SimNode) -> RouteState {
    node.routes.own_state(RouteClass::Sending).borrow().clone()
}

/// The app calls from node `from` delivered to node `to` so far.
fn calls(sim: &Netsim, from: usize, to: usize) -> usize {
    let entry = format!("{to}<-{from} AppCall");
    sim.trace().of(to).filter(|e| *e == entry).count()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn receipted_calls_land_on_their_first_attempt_over_the_sending_route() {
    let (sim, mut nodes) = start(2, 93).await;
    assert_eq!(
        sending(&nodes[0]),
        RouteState::Idle,
        "locked: no Sending route"
    );
    for (node, name) in nodes.iter_mut().zip(["Uma", "Vic"]) {
        node.unlock_new_identity(name).await;
    }
    for node in &nodes {
        assert!(
            eventually(&sim, LIMIT, || async {
                matches!(sending(node), RouteState::Available { .. })
            })
            .await,
            "node {} wants its Sending route at unlock",
            node.index
        );
    }
    let invite: rekindle_types::dto::friends::InviteCreated =
        serde_json::from_value(nodes[0].ok(IpcRequest::FriendInviteCreate).await)
            .expect("the invite");
    // An invite's request goes to its single-use inbox record, not by call.
    nodes[1]
        .ok(IpcRequest::FriendAddFromInvite { invite: invite.url })
        .await;
    let vic = key(&nodes[1]);
    assert!(eventually(&sim, LIMIT, || has_request_from(&nodes[0], &vic)).await);
    let (to_vic, to_uma) = (calls(&sim, 0, 1), calls(&sim, 1, 0));
    accept(&nodes[0], &nodes[1]).await;
    let uma = key(&nodes[0]);
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[1], &uma)).await);
    assert!(
        eventually(&sim, LIMIT, || async { calls(&sim, 1, 0) > to_uma }).await,
        "Vic's friends key reached Uma"
    );
    // Past any retry: the dm floor is the longest ack delay plus a call
    // timeout (`rekindle-types` `DM_RETRY_FLOOR`), well under a minute.
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    // Each receipted row is one call: Uma's FriendAccept PreKeyInit and
    // FriendsKey, Vic's FriendsKey. A lost answer would show as a second.
    assert_eq!(
        calls(&sim, 0, 1) - to_vic,
        2,
        "Uma's two rows, one call each"
    );
    assert_eq!(calls(&sim, 1, 0) - to_uma, 1, "Vic's one row, one call");
    nodes[0].ok(IpcRequest::Lock).await;
    assert_eq!(
        sending(&nodes[0]),
        RouteState::Idle,
        "a lock releases the Sending route"
    );
}
