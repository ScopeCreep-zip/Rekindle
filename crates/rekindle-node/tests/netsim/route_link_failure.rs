//! A first-hop link failure on a node's safety route (plan E2.B close-out,
//! premise rewritten for Veilid 0.5.8 at VU2c): the send fails
//! `NoConnection`, which our port reports as a send error on **our** side
//! (plan VU5, synthesis #9: Wes keeps Xan's route, never forgets it and
//! refetches it), and Veilid counts it as a send
//! failure, which only recompiles the route (`route_stats.rs:19-35`:
//! `needs_recompile`, not `means_route_dead`; `route_compile.rs:672`). No
//! `dead_routes` is reported and no route is reallocated: the message still
//! arrives (the sender's outbox retries it) and the pair keeps talking on
//! the routes it had.
//!
//! The work after the failure runs in tasks the Host spawned, which run in
//! the node's span: every WARN or ERROR of the run counts under a node,
//! none under `None`. A stray untagged app message makes Xan's dispatch loop (a task
//! spawned at unlock) warn, and that warning counts under Xan.

use rekindle_netsim::{Fault, FORGETS_PEER_ROUTE};

use crate::harness::{eventually, start};
use crate::logs;
use crate::pair::{befriend, holds_message, key, send, talk, LIMIT};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_first_hop_failure_recompiles_and_keeps_delivering() {
    let (sim, mut nodes) = start(2, 92).await;
    for (node, name) in nodes.iter_mut().zip(["Wes", "Xan"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "before").await;
    let (wes, xan) = (key(&nodes[0]), key(&nodes[1]));
    let routes = sim.route_blobs(0);
    let forgets = |sim: &rekindle_netsim::Netsim| {
        sim.trace()
            .of(0)
            .filter(|e| *e == FORGETS_PEER_ROUTE)
            .count()
    };
    let forgot = forgets(&sim);
    sim.fault(Fault::RouteLinkFailure { node: 0 });
    let body = "across the failed hop";
    send(&nodes[0], &xan, body).await;
    assert!(
        eventually(&sim, LIMIT, || holds_message(&nodes[1], &wes, body)).await,
        "Xan reads {body:?} after Wes's send failed at its first hop"
    );
    talk(&sim, &nodes[0], &nodes[1], "after").await;
    assert_eq!(
        sim.route_blobs(0),
        routes,
        "Wes's route lived: recompiled, never reallocated"
    );
    assert_eq!(
        forgets(&sim),
        forgot,
        "our first hop failed, not Xan's route: Wes kept it"
    );
    let xan_warns = |c: &std::collections::BTreeMap<Option<usize>, logs::Counts>| {
        c.get(&Some(1)).copied().unwrap_or_default().warn
    };
    let before = xan_warns(&logs::counts());
    sim.stray_message(0, 1, b"not a rekindle frame".to_vec());
    assert!(
        eventually(&sim, LIMIT, || {
            let warned = xan_warns(&logs::counts()) > before;
            async move { warned }
        })
        .await,
        "Xan drops the stray message"
    );
    let counts = logs::counts();
    assert_eq!(
        xan_warns(&counts) - before,
        1,
        "Xan's dispatch task's warning counts under Xan: {counts:?}"
    );
    assert_eq!(
        counts.get(&None).copied().unwrap_or_default(),
        logs::Counts::default(),
        "every WARN and ERROR counts under its node: {counts:?}"
    );
}
