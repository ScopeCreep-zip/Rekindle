//! E2.1 (Linux live, 2026-11-19): every app message is pinned to our tested
//! `Sending` route, as our 1:1 calls are (RH1r). Unpinned, Veilid sends a
//! statement over an automatic pool route (0.5.8 `route_select.rs:182-188`,
//! `:265-269`); live, one or two of those a minute fail 3 to 5 loopback
//! tests before Veilid releases them, and a statement on one vanishes:
//! Veilid answers once the first hop takes it and counts only lost
//! questions against a route (`route_stats.rs:144-147`, `:409`). Typing is
//! a pure statement, with no durable copy behind it, so it shows the route:
//! while both nodes' pool routes fail ([`Fault::PoolRoutesFail`]) and their
//! `Sending` routes live, Uma's typing still reaches Vic.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{Fault, Netsim};

use crate::harness::{eventually, start};
use crate::pair::{befriend, key, talk, LIMIT};

/// The app messages from node `from` delivered to node `to` so far.
fn statements(sim: &Netsim, from: usize, to: usize) -> usize {
    let entry = format!("{to}<-{from} AppMessage");
    sim.trace().of(to).filter(|e| *e == entry).count()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn typing_arrives_while_the_pool_routes_fail() {
    let (sim, mut nodes) = start(2, 94).await;
    for (node, name) in nodes.iter_mut().zip(["Uma", "Vic"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[1], &nodes[0]).await;
    talk(&sim, &nodes[0], &nodes[1], "before").await;
    for node in 0..2 {
        sim.fault(Fault::PoolRoutesFail {
            node,
            failing: true,
        });
    }
    sim.quiesce().await;
    let before = statements(&sim, 0, 1);
    let vic = key(&nodes[1]);
    for typing in [true, false, true] {
        nodes[0]
            .ok(IpcRequest::DmTyping {
                peer_key: vic.clone(),
                typing,
            })
            .await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(
        eventually(&sim, LIMIT, || async {
            statements(&sim, 0, 1) >= before + 3
        })
        .await,
        "each typing statement reached Vic over Uma's Sending route"
    );
}
