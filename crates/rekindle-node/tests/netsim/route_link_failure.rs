//! A first-hop link failure on a node's own route (plan E2.B close-out):
//! Veilid counts a send that fails at its safety route's first hop as a
//! send failure on that route, and one kills it (`route_spec_store/mod.rs:48`,
//! `route_stats.rs:56-60`), reported to the owner as `dead_routes`. Our
//! `OwnRoutes` forgets the dead route and allocates another, the new blob
//! is published, and the pair keeps talking.
//!
//! The work after the failure runs in tasks the Host spawned (the dispatch
//! loop hears the death, `OwnRoutes` reallocates), which run in the node's
//! span: every WARN or ERROR of the run counts under a node, none under
//! `None`. A stray untagged app message makes Xan's dispatch loop (a task
//! spawned at unlock) warn, and that warning counts under Xan.

use rekindle_netsim::Fault;

use crate::harness::{eventually, start};
use crate::logs;
use crate::pair::{befriend, holds_message, key, send, talk, LIMIT};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_first_hop_failure_reallocates_and_keeps_delivering() {
    let (sim, mut nodes) = start(2, 92).await;
    for (node, name) in nodes.iter_mut().zip(["Wes", "Xan"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "before").await;
    let (wes, xan) = (key(&nodes[0]), key(&nodes[1]));
    let routes = sim.route_blobs(0);
    sim.fault(Fault::RouteLinkFailure { node: 0 });
    let body = "across the failed hop";
    send(&nodes[0], &xan, body).await;
    assert!(
        eventually(&sim, LIMIT, || holds_message(&nodes[1], &wes, body)).await,
        "Xan reads {body:?} after Wes's route died at its first hop"
    );
    assert!(
        eventually(&sim, LIMIT, || {
            let fresh = sim.route_blobs(0).iter().any(|b| !routes.contains(b));
            async move { fresh }
        })
        .await,
        "Wes reallocated its dead route"
    );
    talk(&sim, &nodes[0], &nodes[1], "after").await;
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
