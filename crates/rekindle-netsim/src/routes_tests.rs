//! The route model against the pinned Veilid ([`crate::routes`]).

use std::time::Duration;

use rekindle_records::port::{PortError, PortEvent, RouteClass};
use rekindle_types::config::SafetyProfile;

use crate::tests::{attached, recv};
use crate::{Fault, Netsim, Node};

async fn own_blob(node: &Node) -> Vec<u8> {
    node.ports.routes.want_own();
    node.ports.routes.available_own(RouteClass::General).await
}

/// A dead route's blob still imports, and a message sent to it is lost
/// without an error; the importer hears nothing of the death.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_dead_route_imports_and_swallows_sends() {
    let (sim, mut nodes) = Netsim::nodes(2, 21);
    let _pump = sim.start();
    let blob = own_blob(&nodes[1]).await;
    attached(&mut nodes[0]).await;
    attached(&mut nodes[1]).await;
    sim.fault(Fault::RouteDeath { node: 1 });
    assert!(matches!(
        recv(&mut nodes[1]).await,
        PortEvent::RouteChange { .. }
    ));
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    let profile = SafetyProfile::default_dht();
    nodes[0]
        .ports
        .messages
        .send(&target, &profile, b"lost".to_vec())
        .await
        .unwrap();
    sim.quiesce().await;
    assert!(nodes[0].events.try_recv().is_err());
    assert!(!matches!(
        nodes[1].events.try_recv(),
        Ok(PortEvent::AppMessage { .. })
    ));
}

/// An imported route unused for 5 minutes no longer resolves (Veilid's
/// `NoConnection`); forgotten and imported again, it resolves.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_idle_import_expires_after_five_minutes() {
    let (sim, mut nodes) = Netsim::nodes(2, 22);
    let _pump = sim.start();
    let blob = own_blob(&nodes[1]).await;
    attached(&mut nodes[0]).await;
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    let profile = SafetyProfile::default_dht();
    tokio::time::sleep(Duration::from_secs(299)).await;
    nodes[0]
        .ports
        .messages
        .send(&target, &profile, vec![1])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(300)).await;
    assert!(matches!(
        nodes[0]
            .ports
            .messages
            .send(&target, &profile, vec![2])
            .await,
        Err(PortError::RouteUnusable(_))
    ));
    nodes[0].ports.routes.invalidate_after_send_failure(&target);
    let again = nodes[0].ports.routes.import(&blob).unwrap();
    nodes[0]
        .ports
        .messages
        .send(&again, &profile, vec![3])
        .await
        .unwrap();
}

/// Calls lost to 5 distinct destinations kill the asker's own route,
/// reported to it as dead (`route_stats.rs:56-60`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn five_lost_questions_kill_the_askers_route() {
    let (sim, mut nodes) = Netsim::nodes(6, 23);
    let _pump = sim.start();
    let mine = own_blob(&nodes[0]).await;
    let mut targets = Vec::new();
    for i in 1..6 {
        let blob = own_blob(&nodes[i]).await;
        sim.fault(Fault::Unanswered {
            node: i,
            unanswered: true,
        });
        targets.push(nodes[0].ports.routes.import(&blob).unwrap());
    }
    attached(&mut nodes[0]).await;
    let profile = SafetyProfile::default_dht();
    for (n, target) in targets.iter().enumerate() {
        assert!(nodes[0]
            .ports
            .messages
            .call(target, &profile, vec![1])
            .await
            .is_err());
        if n < 4 {
            assert!(nodes[0].events.try_recv().is_err());
        }
    }
    let PortEvent::RouteChange { dead, .. } = recv(&mut nodes[0]).await else {
        panic!("expected a route change");
    };
    assert_eq!(dead.len(), 1);
    assert!(sim.route_blobs(0).iter().all(|b| *b != mine));
}
