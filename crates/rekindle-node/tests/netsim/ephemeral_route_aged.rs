//! Linux live finding CR1 (round 14): two 1:1 calls worked, and from the
//! third every call signal was dropped as "no route for peer" for 13
//! minutes while the UI rang out. The peer-route cache keeps a blob for a
//! backstop age of 900 s (`rekindle_route::lifecycle::
//! PEER_ROUTE_CACHE_MAX_AGE`); past it the entry is gone, and an ephemeral
//! frame (typing, a call signal, DM video) never re-resolved the route the
//! peer still publishes on its profile, so every such frame was dropped
//! until a durable send happened to refetch it. Calls are not on the
//! daemon's IPC; typing travels the same ephemeral path
//! (`rekindle-transport` `dm::send::route_send_datagram`), so it shows it:
//! after the age passes with nothing sent, Uma's typing still reaches Vic.
//!
//! Fails without the re-resolve on an ephemeral send: no statement after
//! the age.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Netsim;

use crate::harness::{eventually, start};
use crate::pair::{befriend, key, talk, LIMIT};

/// Past the peer-route cache's backstop age (900 s).
const AGED: Duration = Duration::from_secs(16 * 60);

/// The app messages from node `from` delivered to node `to` so far.
fn statements(sim: &Netsim, from: usize, to: usize) -> usize {
    let entry = format!("{to}<-{from} AppMessage");
    sim.trace().of(to).filter(|e| *e == entry).count()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn typing_after_the_cached_route_aged_out_re_resolves_it() {
    let (sim, mut nodes) = start(2, 95).await;
    for (node, name) in nodes.iter_mut().zip(["Uma", "Vic"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[1], &nodes[0]).await;
    talk(&sim, &nodes[0], &nodes[1], "before").await;
    tokio::time::sleep(AGED).await;
    sim.quiesce().await;
    let before = statements(&sim, 0, 1);
    nodes[0]
        .ok(IpcRequest::DmTyping {
            peer_key: key(&nodes[1]),
            typing: true,
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async { statements(&sim, 0, 1) > before }).await,
        "Uma's typing re-resolved Vic's published route and reached Vic"
    );
}
