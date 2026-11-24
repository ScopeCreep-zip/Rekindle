//! An importer's remote route test (plan RV1, F9 N2). Tia accepts Ola's
//! request through the invite's inbox, a call over the route Ola's request
//! named, so Tia's node imports it; then Ola's General
//! route stops taking traffic from outside while she still holds it
//! ([`Fault::OutsideDead`]), so Ola is never told. Veilid on Tia's node
//! tests the routes it imported each [`ROUTE_MIN_IDLE`]
//! (veilid-core 0.5.8 `private_route_management.rs:396-414`); the test gets
//! no answer, so at the end of its answer wait Veilid removes the route
//! from Tia's cache and reports it to Tia's Host as
//! `RouteChange.dead_remote` (`ping_validator.rs:740-744`,
//! `route_spec_store_cache.rs:397-415`, `route_spec_store/mod.rs:235-251`),
//! within one test cadence, while the route stays Ola's.
//!
//! Fails without the sim's remote route test (`rekindle-netsim`
//! `loopback.rs` `tick_remote_route_tests`): Tia's node never releases the
//! route.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{Fault, RELEASES_PEER_ROUTE, RPC_TIMEOUT};
use rekindle_records::port::RouteClass;

use crate::harness::{eventually, start};
use crate::pair::{accept, has_request_from, is_accepted_friend, key};

/// Veilid 0.5.8 `route_spec_store/mod.rs:46` `ROUTE_MIN_IDLE_TIME_MS`.
const ROUTE_MIN_IDLE: Duration = Duration::from_secs(30);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_importer_releases_a_route_dead_from_outside_within_one_test_cadence() {
    let (sim, mut nodes) = start(2, 68).await;
    for (node, name) in nodes.iter_mut().zip(["Tia", "Ola"]) {
        node.unlock_new_identity(name).await;
    }
    let invite: rekindle_types::dto::friends::InviteCreated =
        serde_json::from_value(nodes[0].ok(IpcRequest::FriendInviteCreate).await)
            .expect("the invite");
    nodes[1]
        .ok(IpcRequest::FriendAddFromInvite { invite: invite.url })
        .await;
    let ola = key(&nodes[1]);
    assert!(
        eventually(&sim, Duration::from_secs(60), || has_request_from(
            &nodes[0], &ola
        ))
        .await,
        "Tia holds Ola's request and the route it named"
    );
    accept(&nodes[0], &nodes[1]).await;
    let tia = key(&nodes[0]);
    assert!(
        eventually(&sim, Duration::from_secs(120), || is_accepted_friend(
            &nodes[1], &tia
        ))
        .await,
        "the accept reached Ola over the route Tia imported"
    );
    let released = || {
        sim.trace()
            .of(nodes[0].index)
            .filter(|e| *e == RELEASES_PEER_ROUTE)
            .count()
    };
    let before = released();
    sim.fault(Fault::OutsideDead {
        node: nodes[1].index,
        class: RouteClass::General,
    });
    let cadence = ROUTE_MIN_IDLE + RPC_TIMEOUT + Duration::from_secs(2);
    assert!(
        eventually(&sim, cadence, || std::future::ready(released() > before)).await,
        "Tia's test of Ola's route fails and Veilid releases it"
    );
}
