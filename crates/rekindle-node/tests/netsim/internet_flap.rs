//! The live log's "PublicInternet ready / unavailable" flap (plan E2.B S9,
//! H5): a friend's public internet drops for a few seconds, several times,
//! while the node stays attached (`RoutingDomainReadyEvent`,
//! `routing_table/health.rs:255-287`; the DHT gate follows it,
//! `storage_manager/mod.rs:887-898`). A message sent inside a flap is held,
//! not lost, and lands once the network is ready; the pair keeps talking
//! after the flaps.

use std::time::Duration;

use rekindle_netsim::Fault;

use crate::harness::{eventually, start};
use crate::pair::{befriend, holds_message, key, send, talk, LIMIT};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_public_internet_flap_holds_then_delivers() {
    let (sim, mut nodes) = start(2, 91).await;
    for (node, name) in nodes.iter_mut().zip(["Uma", "Vic"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "before").await;
    let (uma, vic) = (key(&nodes[0]), key(&nodes[1]));
    for round in 0..3 {
        sim.fault(Fault::InternetReady {
            node: 0,
            ready: false,
        });
        let body = format!("inside flap {round}");
        send(&nodes[0], &vic, &body).await;
        tokio::time::sleep(Duration::from_secs(4)).await;
        sim.fault(Fault::InternetReady {
            node: 0,
            ready: true,
        });
        assert!(
            eventually(&sim, LIMIT, || holds_message(&nodes[1], &uma, &body)).await,
            "Vic reads {body:?} once Uma is ready again"
        );
        tokio::time::sleep(Duration::from_secs(20)).await;
    }
    talk(&sim, &nodes[0], &nodes[1], "after").await;
}
