//! A node on a network that blocks UDP (plan VU2 item 11, a VU5 product
//! finding). Veilid 0.5.8 gives a custom private route exactly its spec's
//! preferred ordering, proves every hop pair on that ordering alone, and
//! has no fallback to the other ordering (`route_allocate.rs:897-936`,
//! `:99-105`). Our `Sending` route asks `PreferUnordered`
//! (`rekindle-protocol` `own_routes.rs:43-51`), so on such a network it is
//! never allocated: every receipted 1:1 call, which is pinned to it, stays
//! refused while `General` (PreferOrdered) is live. This scenario asserts
//! today's behaviour and so names the finding: plan VU5 (decision 2) makes
//! `Sending` `PreferOrdered`, `RouteOrdering::of` follows, and this
//! scenario then asserts the opposite (its Sending route is available).

use rekindle_netsim::Fault;
use rekindle_records::port::{RouteClass, RouteState};

use crate::harness::{eventually, start, SimNode};
use crate::pair::LIMIT;

fn state(node: &SimNode, class: RouteClass) -> RouteState {
    node.routes.own_state(class).borrow().clone()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_udp_blocked_node_never_allocates_its_sending_route() {
    let (sim, mut nodes) = start(2, 94).await;
    sim.fault(Fault::UdpBlocked {
        node: 1,
        blocked: true,
    });
    for (node, name) in nodes.iter_mut().zip(["Yul", "Zed"]) {
        node.unlock_new_identity(name).await;
    }
    let available =
        |node: &SimNode, class| matches!(state(node, class), RouteState::Available { .. });
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
        !eventually(&sim, LIMIT, || async {
            available(&nodes[1], RouteClass::Sending)
        })
        .await,
        "VU5 finding: Zed, UDP blocked, never allocates Sending (PreferUnordered)"
    );
}
