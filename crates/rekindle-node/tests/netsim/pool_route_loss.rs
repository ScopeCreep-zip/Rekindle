//! E2.1 (Linux live, 2026-11-19), reworked for plan RV1: live, one or two
//! automatic pool routes a minute failed 3 to 5 loopback tests before
//! Veilid released them (0.5.8 `route_spec_store/mod.rs:51-63`,
//! `route_stats.rs:19-29`), and a statement on one vanished: Veilid
//! answers once the first hop takes it and counts only lost questions
//! against a route (`route_stats.rs:144-147`, `:409`). The pinned `Sending`
//! route this scenario first relied on is gone (RV1: a send's safety route
//! is Veilid's tested pool route of the default hop count, never one of
//! ours), so while both nodes' pool routes fail ([`Fault::PoolRoutesFail`])
//! typing, a pure statement with no durable copy, is lost. What the
//! scenario guards is that typing arrives: once Veilid has released the
//! failing pool routes and the pool carries traffic again, Uma's typing
//! reaches Vic, with nothing on either side left broken by the loss.

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
async fn typing_arrives_once_the_failing_pool_routes_are_replaced() {
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
    let lost = statements(&sim, 0, 1);
    let vic = key(&nodes[1]);
    nodes[0]
        .ok(IpcRequest::DmTyping {
            peer_key: vic.clone(),
            typing: true,
        })
        .await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(
        statements(&sim, 0, 1),
        lost,
        "a typing statement over a failing pool route is lost"
    );
    // Veilid released the failing pool routes; the pool carries traffic.
    for node in 0..2 {
        sim.fault(Fault::PoolRoutesFail {
            node,
            failing: false,
        });
    }
    let before = statements(&sim, 0, 1);
    for typing in [false, true, false] {
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
        "each typing statement reached Vic over Uma's pool route"
    );
}
