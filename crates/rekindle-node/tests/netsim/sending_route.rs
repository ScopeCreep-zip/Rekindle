//! RH1r (plan E4.3 RH2, built ahead): every 1:1 receipted call is pinned to
//! our `Sending` route, wanted from unlock to lock, and its answer comes
//! back over it (Veilid 0.5.8 `rpc_processor/destination.rs:477-497`). With the
//! route live, each receipted row lands on its first call under the sim's normal
//! conditions; a lock releases the route. An invite's
//! request goes to its single-use inbox record, so the accept is the call.
//!
//! That the call is pinned shows where a broken route lands. Pinned, the
//! question goes out on Uma's `Sending` route and names it as the reply
//! route (Veilid 0.5.8 `rpc_processor/destination.rs:486-528`:
//! `select_single_route` with the question's `preferred_route`); unpinned,
//! both ride Veilid's 3-hop pool routes (`route_select.rs:131-136`). So when
//! Uma's `Sending` route stops carrying traffic
//! ([`Fault::LoopbackFailures`]) the accept cannot land until her loopback
//! tests kill that route and she allocates a new one, while her published
//! `General` route is untouched; an unpinned accept would land at once
//! over the pool. (A first-hop failure no longer shows the pin: 0.5.8
//! counts it as a send failure, which recompiles the route and kills none,
//! `route_stats.rs:19-35`, so it lands the same either way.)

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{Fault, Netsim};
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

fn general(node: &SimNode) -> RouteState {
    node.routes.own_state(RouteClass::General).borrow().clone()
}

/// Vic's invite request reaches Uma (by her inbox record, not by call).
async fn requested(sim: &Netsim, nodes: &[SimNode]) {
    let invite: rekindle_types::dto::friends::InviteCreated =
        serde_json::from_value(nodes[0].ok(IpcRequest::FriendInviteCreate).await)
            .expect("the invite");
    nodes[1]
        .ok(IpcRequest::FriendAddFromInvite { invite: invite.url })
        .await;
    let vic = key(&nodes[1]);
    assert!(eventually(sim, LIMIT, || has_request_from(&nodes[0], &vic)).await);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_receipted_call_waits_out_its_broken_sending_route() {
    let (sim, mut nodes) = start(2, 93).await;
    for (node, name) in nodes.iter_mut().zip(["Uma", "Vic"]) {
        node.unlock_new_identity(name).await;
    }
    requested(&sim, &nodes).await;
    assert!(
        eventually(&sim, LIMIT, || async {
            matches!(sending(&nodes[0]), RouteState::Available { .. })
        })
        .await
    );
    let (published, pinned) = (general(&nodes[0]), sending(&nodes[0]));
    sim.fault(Fault::LoopbackFailures {
        node: 0,
        class: RouteClass::Sending,
    });
    accept(&nodes[0], &nodes[1]).await;
    let uma = key(&nodes[0]);
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[1], &uma)).await);
    assert_ne!(
        sending(&nodes[0]),
        pinned,
        "the accept landed only once Uma's broken Sending route was replaced"
    );
    assert_eq!(
        general(&nodes[0]),
        published,
        "her published General route was never in question"
    );
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
    requested(&sim, &nodes).await;
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
