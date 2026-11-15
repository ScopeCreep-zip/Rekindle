//! Nodes whose UDP path fails (plan VU5, decision 2). Veilid 0.5.8 gives a
//! custom private route exactly its spec's preferred ordering, proves every
//! hop pair on that ordering alone, and has no fallback to the other
//! ordering (`route_allocate.rs:936`, `:99-105`). Our `Sending` route asks
//! `PreferOrdered` (`rekindle-protocol` `own_routes.rs`
//! `sending_private_spec`), so it rides TCP/WS: a network that blocks UDP
//! still allocates it, and a UDP path that fails mid-session does not take
//! the receipted 1:1 calls pinned to it down with it. Before decision 2 it
//! asked `PreferUnordered`: a UDP-blocked node never had it, and a pinned
//! call over a failed UDP path was lost without an error, the question
//! never reaching the callee (A/B round 2, R2-1), until 5 failed loopback
//! tests killed the route (`route_spec_store/mod.rs:51-54`), after which it
//! could not be allocated again.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{Fault, Netsim};
use rekindle_records::port::{RouteClass, RouteState};

use crate::harness::{eventually, start, SimNode};
use crate::pair::{accept, has_request_from, is_accepted_friend, key, LIMIT};

fn state(node: &SimNode, class: RouteClass) -> RouteState {
    node.routes.own_state(class).borrow().clone()
}

fn available(node: &SimNode, class: RouteClass) -> bool {
    matches!(state(node, class), RouteState::Available { .. })
}

/// The app calls from node `from` delivered to node `to` so far.
fn calls(sim: &Netsim, from: usize, to: usize) -> usize {
    let entry = format!("{to}<-{from} AppCall");
    sim.trace().of(to).filter(|e| *e == entry).count()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_udp_blocked_node_allocates_its_sending_route() {
    let (sim, mut nodes) = start(2, 94).await;
    sim.fault(Fault::UdpBlocked {
        node: 1,
        blocked: true,
    });
    for (node, name) in nodes.iter_mut().zip(["Yul", "Zed"]) {
        node.unlock_new_identity(name).await;
    }
    assert!(
        eventually(&sim, LIMIT, || async {
            available(&nodes[0], RouteClass::Sending)
        })
        .await,
        "Yul, with UDP, allocates Sending"
    );
    assert!(
        eventually(&sim, LIMIT, || async {
            available(&nodes[1], RouteClass::General)
        })
        .await,
        "Zed allocates General (Ordered)"
    );
    assert!(
        eventually(&sim, LIMIT, || async {
            available(&nodes[1], RouteClass::Sending)
        })
        .await,
        "Zed, UDP blocked, allocates Sending (PreferOrdered, decision 2)"
    );
}

/// R2-1: Uma's UDP path fails after her Sending route is live, then she
/// accepts Vic's request. Every receipted call of the accept is pinned to
/// her Sending route; each reaches Vic on its first attempt, the first
/// within one answer wait, so no question was lost.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_failed_udp_path_loses_no_pinned_friend_accept_call() {
    let (sim, mut nodes) = start(2, 95).await;
    for (node, name) in nodes.iter_mut().zip(["Uma", "Vic"]) {
        node.unlock_new_identity(name).await;
    }
    for node in &nodes {
        assert!(
            eventually(&sim, LIMIT, || async {
                available(node, RouteClass::Sending)
            })
            .await,
            "node {} has its Sending route",
            node.index
        );
    }
    let invite: rekindle_types::dto::friends::InviteCreated =
        serde_json::from_value(nodes[0].ok(IpcRequest::FriendInviteCreate).await)
            .expect("the invite");
    nodes[1]
        .ok(IpcRequest::FriendAddFromInvite { invite: invite.url })
        .await;
    let vic = key(&nodes[1]);
    assert!(eventually(&sim, LIMIT, || has_request_from(&nodes[0], &vic)).await);
    sim.fault(Fault::UdpBlocked {
        node: 0,
        blocked: true,
    });
    let to_vic = calls(&sim, 0, 1);
    accept(&nodes[0], &nodes[1]).await;
    // One answer wait (5 s + 500 ms per effective hop past 4,
    // `rpc_processor/mod.rs:100-117`) plus latency: a lost first call
    // would retry only after it.
    assert!(
        eventually(&sim, Duration::from_secs(6), || async {
            calls(&sim, 0, 1) > to_vic
        })
        .await,
        "Uma's first FriendAccept call reached Vic within one answer wait"
    );
    let uma = key(&nodes[0]);
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[1], &uma)).await);
    // Past any retry (the dm floor, `rekindle-types` `DM_RETRY_FLOOR`).
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(
        calls(&sim, 0, 1) - to_vic,
        2,
        "Uma's two rows (FriendAccept PreKeyInit, FriendsKey), one call each"
    );
    assert!(
        available(&nodes[0], RouteClass::Sending),
        "Uma's Sending route lives on without UDP"
    );
}
