//! A friend request by code to a route that died silently after the code
//! was shared (Linux finding L-FR1: a request was called six times over two
//! minutes into the one route the target's profile held; the target never
//! retested, replaced or republished it). Ann's General route stops
//! delivering with no word from Veilid ([`Fault::RouteSilentDeath`]) before
//! Rae reads it from the profile Ann's code names. Plan RV1 deleted the
//! owner's own self-test this first relied on: Veilid tests our allocated
//! routes itself, and a loopback failure deferred while its online
//! detector said Offline counts again once an answer to a question sent
//! since proves the network (0.5.8 `rpc_processor/mod.rs:1711-1728`,
//! `online_detector.rs:1343-1347`); Ann's own DHT work gives that answer,
//! the failures kill the route (`route_stats.rs:19-29`), and
//! `OwnRoutes::on_dead` allocates and republishes a replacement. Rae's
//! request row, re-reading Ann's profile after each failed call
//! (`rekindle-transport` `dm::send::call_datagram_answer`), then lands
//! over the new route.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_records::port::RouteClass;

use crate::harness::{eventually, start};
use crate::pair::{has_request_from, key};

/// Veilid's loopback tests declare the route dead within three failed
/// tests once the network is proven, and the republish and Rae's next
/// attempt follow within a minute.
const HEALED: Duration = Duration::from_secs(300);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_request_to_a_silently_dead_code_route_lands_after_veilid_replaces_it() {
    let (sim, mut nodes) = start(2, 41).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let code: rekindle_types::dto::friends::FriendCodeView =
        serde_json::from_value(nodes[1].ok(IpcRequest::FriendCode).await).expect("the code");
    // After the code is shared, Ann's route dies; nobody is told.
    sim.fault(Fault::RouteSilentDeath {
        node: 1,
        class: RouteClass::General,
    });
    nodes[0]
        .ok(IpcRequest::FriendAddFromCode {
            code: code.url,
            message: "hi".to_string(),
        })
        .await;
    let rae = key(&nodes[0]);
    assert!(
        eventually(&sim, HEALED, || has_request_from(&nodes[1], &rae)).await,
        "Ann republished a tested route and Rae's request reached it"
    );
}
