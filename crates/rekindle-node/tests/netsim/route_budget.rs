//! Plan RV1 (owner decision 2026-11-22; threat model L2 bounded exposure):
//! a send's safety route is Veilid's pool route of the default hop count,
//! never one of ours, and every new route of ours spends from the
//! identity's persisted budget, so a storm of route deaths, forced or
//! forged, stops at the cap with the class down and the pause shown.

use std::time::Duration;

use rekindle_codec::ingress::{LIVENESS_PROBE_LEN, LIVENESS_TAG};
use rekindle_netsim::Fault;
use rekindle_records::port::{RouteClass, RoutePauseCause, RouteState};
use rekindle_route::budget::BUDGET_CAPACITY;
use rekindle_route::liveness::REPORT_ROTATE_AGE;
use rekindle_types::message::SendState;
use rekindle_types::subscription_events::{NetworkEvent, SubscriptionEvent};

use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, key, send, talk, LIMIT};

async fn delivered(node: &SimNode, peer: &str, body: &str) -> bool {
    let history: Vec<rekindle_types::dto::dm::DmMessage> = serde_json::from_value(
        node.ok(rekindle_ipc::protocol::IpcRequest::DmHistory {
            peer_key: peer.to_string(),
            limit: 100,
            before: None,
        })
        .await,
    )
    .expect("the history");
    history
        .iter()
        .any(|m| m.is_own && m.body == body && m.send_state == SendState::Delivered)
}

async fn friends(seed: u64) -> (rekindle_netsim::Netsim, Vec<SimNode>) {
    let (sim, mut nodes) = start(2, seed).await;
    for (node, name) in nodes.iter_mut().zip(["Ada", "Bo"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "hello").await;
    (sim, nodes)
}

/// The app messages and calls from node `from` that reached node `to` over
/// a route so far. A send's DM chain entry is a DHT write (plan E2.5), not
/// counted here; [`Fault::PoolRoutesFail`] stops it too, since every DHT
/// question rides the record's `Safe` selection over the same pool.
fn over_routes(sim: &rekindle_netsim::Netsim, from: usize, to: usize) -> usize {
    let message = format!("{to}<-{from} AppMessage");
    let call = format!("{to}<-{from} AppCall");
    sim.trace()
        .of(to)
        .filter(|e| *e == message || *e == call)
        .count()
}

fn paused(node: &SimNode) -> Option<RoutePauseCause> {
    *node.routes.own_paused().borrow()
}

fn told_exhausted(e: &SubscriptionEvent) -> bool {
    matches!(
        e,
        SubscriptionEvent::Network(NetworkEvent::RoutePaused {
            cause: RoutePauseCause::Exhausted
        })
    )
}

/// Ada's sends ride Veilid's pool routes: sends 45 s apart (past the 30 s a
/// pool route of another hop count lives untested, the netsim's
/// `ROUTE_MIN_IDLE`, Veilid 0.5.8 `route_stats.rs:144-158`) are all
/// delivered, so the routes are of the default hop count, kept by the
/// keepalive; when her pool routes fail and her own routes are healthy, her
/// send is not delivered (neither a route nor her DM chain write carries it),
/// no app message or call of hers reaches Bo over a route, and her own
/// routes are untouched (no own route is a safety route); once the pool
/// heals the send is delivered.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn sends_ride_tested_pool_routes_at_the_default_hop_count() {
    let (sim, nodes) = friends(131).await;
    let bo = key(&nodes[1]);
    for i in 0..4 {
        let body = format!("spaced {i}");
        send(&nodes[0], &bo, &body).await;
        assert!(
            eventually(&sim, LIMIT, || delivered(&nodes[0], &bo, &body)).await,
            "{body} delivered over a pool route"
        );
        tokio::time::sleep(Duration::from_secs(45)).await;
    }
    let own = sim.route_blobs(0);
    sim.fault(Fault::PoolRoutesFail {
        node: 0,
        failing: true,
    });
    sim.quiesce().await;
    let carried = over_routes(&sim, 0, 1);
    send(&nodes[0], &bo, "pool down").await;
    assert!(
        !eventually(&sim, Duration::from_secs(30), || delivered(
            &nodes[0],
            &bo,
            "pool down"
        ))
        .await,
        "with the pool failing, Ada's own healthy routes carry no send"
    );
    assert_eq!(
        over_routes(&sim, 0, 1),
        carried,
        "no app message or call of hers reached Bo over a route"
    );
    assert_eq!(sim.route_blobs(0), own, "her own routes are untouched");
    sim.fault(Fault::PoolRoutesFail {
        node: 0,
        failing: false,
    });
    assert!(
        eventually(&sim, Duration::from_secs(300), || delivered(
            &nodes[0],
            &bo,
            "pool down"
        ))
        .await,
        "delivered once the pool heals"
    );
}

/// A storm of forced route deaths (Veilid reports both classes dead each
/// round) spends a token per replacement: within the budget's capacity the
/// classes stay down (`Allocating`, no route published), the Host tells
/// `RoutePaused { Exhausted }`, and later deaths draw nothing.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_forced_death_storm_spends_the_budget_and_the_class_stays_down() {
    let (sim, nodes) = friends(132).await;
    let events = nodes[0].events().await;
    let mut rounds = 0;
    while paused(&nodes[0]).is_none() && rounds <= BUDGET_CAPACITY {
        sim.fault(Fault::RouteDeath { node: 0 });
        tokio::time::sleep(Duration::from_secs(5)).await;
        rounds += 1;
    }
    assert_eq!(paused(&nodes[0]), Some(RoutePauseCause::Exhausted));
    assert!(
        rounds <= BUDGET_CAPACITY,
        "stopped at the cap ({rounds} rounds)"
    );
    assert!(
        eventually(&sim, Duration::from_secs(5), || {
            let told = events.any(told_exhausted);
            async move { told }
        })
        .await,
        "the Host tells every frontend"
    );
    for _ in 0..5 {
        sim.fault(Fault::RouteDeath { node: 0 });
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(sim.route_blobs(0).is_empty(), "nothing drawn past the cap");
    assert_eq!(
        *nodes[0].routes.own_state(RouteClass::General).borrow(),
        RouteState::Allocating
    );
}

/// A forged-signal storm: an attacker who holds Ada's route calls it with
/// `'L'` probes, each a report that rotates her General route once it is
/// older than [`REPORT_ROTATE_AGE`]. Each rotation spends a token, and the
/// storm stops at the cap with the class down and the pause shown.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_forged_signal_storm_stops_at_the_cap() {
    let (sim, nodes) = friends(133).await;
    let mut probe = vec![LIVENESS_TAG];
    probe.resize(LIVENESS_PROBE_LEN, 7);
    let mut rounds = 0;
    while paused(&nodes[0]).is_none() && rounds <= BUDGET_CAPACITY + 1 {
        tokio::time::sleep(REPORT_ROTATE_AGE + Duration::from_secs(5)).await;
        sim.stray_call(1, 0, probe.clone());
        tokio::time::sleep(Duration::from_secs(5)).await;
        rounds += 1;
    }
    assert_eq!(paused(&nodes[0]), Some(RoutePauseCause::Exhausted));
    assert!(
        rounds <= BUDGET_CAPACITY + 1,
        "stopped at the cap ({rounds} rounds)"
    );
    assert_eq!(nodes[0].routes.own_id(RouteClass::General), None);
}
