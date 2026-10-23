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

/// A send failing at the first hop of the sender's own route fails
/// `RouteUnusable` and kills that route after one failure, reported to it
/// as dead (`route_spec_store/mod.rs:48`); the next send goes out.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn one_first_hop_failure_kills_the_senders_route() {
    let (sim, mut nodes) = Netsim::nodes(2, 24);
    let _pump = sim.start();
    let mine = own_blob(&nodes[0]).await;
    let theirs = own_blob(&nodes[1]).await;
    attached(&mut nodes[0]).await;
    attached(&mut nodes[1]).await;
    let target = nodes[0].ports.routes.import(&theirs).unwrap();
    let profile = SafetyProfile::default_dht();
    sim.fault(Fault::RouteLinkFailure { node: 0 });
    assert!(matches!(
        nodes[0]
            .ports
            .messages
            .send(&target, &profile, vec![1])
            .await,
        Err(PortError::RouteUnusable(_))
    ));
    let PortEvent::RouteChange { dead, dead_remote } = recv(&mut nodes[0]).await else {
        panic!("expected a route change");
    };
    assert_eq!((dead.len(), dead_remote.len()), (1, 0));
    assert!(sim.route_blobs(0).iter().all(|b| *b != mine));
    nodes[0]
        .ports
        .messages
        .send(&target, &profile, vec![2])
        .await
        .unwrap();
    assert!(matches!(
        recv(&mut nodes[1]).await,
        PortEvent::AppMessage { message } if message == vec![2]
    ));
}

/// A pinned call (RH1r): refused `Routing` while the caller's `Sending`
/// route is not live; answered over it while it is; and its answer lost
/// when the caller releases the route before the answer comes back
/// (veilid `rpc_processor/destination.rs:477-497`: the reply private route
/// is the preferred route).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_pinned_call_is_answered_over_its_sending_route() {
    let (sim, mut nodes) = Netsim::nodes(2, 23);
    let _pump = sim.start();
    let blob = own_blob(&nodes[1]).await;
    attached(&mut nodes[0]).await;
    attached(&mut nodes[1]).await;
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    let profile = SafetyProfile::default_dht();
    let refused = nodes[0]
        .ports
        .messages
        .call_pinned(&target, &profile, RouteClass::Sending, b"q".to_vec())
        .await;
    assert!(matches!(refused, Err(PortError::Routing(_))), "{refused:?}");

    nodes[0].ports.routes.want_own_class(RouteClass::Sending);
    nodes[0]
        .ports
        .routes
        .available_own(RouteClass::Sending)
        .await;
    for (body, release) in [(b"kept".to_vec(), false), (b"gone".to_vec(), true)] {
        let caller = nodes[0].ports.messages.clone();
        let to = target.clone();
        let profile = profile.clone();
        let call = tokio::spawn(async move {
            caller
                .call_pinned(&to, &profile, RouteClass::Sending, body)
                .await
        });
        let PortEvent::AppCall { call_id, .. } = recv(&mut nodes[1]).await else {
            panic!("expected a call");
        };
        if release {
            nodes[0].ports.routes.release_own_class(RouteClass::Sending);
        }
        nodes[1]
            .ports
            .messages
            .reply(call_id, b"a".to_vec())
            .await
            .unwrap();
        let answer = call.await.unwrap();
        if release {
            assert!(answer.is_err(), "the answer's reply route is gone");
        } else {
            assert_eq!(answer.unwrap(), b"a");
        }
    }
}
