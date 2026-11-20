//! App messages, calls and route death on the simulated network, against
//! the Veilid behaviour the app depends on ([`crate::messages`]).

use std::time::Duration;

use rekindle_records::port::{PortError, PortEvent, RouteClass, RouteState};
use rekindle_types::config::SafetyProfile;

use crate::tests::{attached, recv};
use crate::{call_timeout, Fault, Netsim};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn messages_and_calls_reach_the_route_s_owner() {
    let (sim, mut nodes) = Netsim::nodes(2, 5);
    let pump = sim.start();
    nodes[1].ports.routes.want_own();
    let blob = tokio::time::timeout(
        Duration::from_secs(5),
        nodes[1].ports.routes.available_own(RouteClass::General),
    )
    .await
    .unwrap();
    attached(&mut nodes[1]).await;
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    let profile = SafetyProfile::default_dht();
    crate::tests::sending(&nodes[0]).await;
    nodes[0]
        .ports
        .messages
        .send(&target, &profile, b"hello".to_vec())
        .await
        .unwrap();
    assert!(
        matches!(recv(&mut nodes[1]).await, PortEvent::AppMessage { message, .. } if message == b"hello")
    );

    let caller = nodes[0].ports.messages.clone();
    let call = tokio::spawn(async move { caller.call(&target, &profile, b"ping".to_vec()).await });
    let event = recv(&mut nodes[1]).await;
    let PortEvent::AppCall {
        call_id, message, ..
    } = event
    else {
        panic!("expected a call");
    };
    assert_eq!(message, b"ping");
    nodes[1]
        .ports
        .messages
        .reply(call_id, b"pong".to_vec())
        .await
        .unwrap();
    assert_eq!(call.await.unwrap().unwrap(), b"pong");
    assert!(matches!(
        nodes[0]
            .ports
            .messages
            .send(
                &rekindle_records::port::PeerTarget::new("gone".into()),
                &SafetyProfile::default_dht(),
                vec![1]
            )
            .await,
        Err(PortError::RouteUnusable(_))
    ));
    assert!(matches!(
        nodes[0]
            .ports
            .messages
            .send(
                &nodes[0].ports.routes.import(&blob).unwrap(),
                &SafetyProfile::default_dht(),
                vec![0; rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN + 1]
            )
            .await,
        Err(PortError::MessageTooLarge { .. })
    ));
    pump.abort();
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unanswered_call_times_out_and_a_partition_drops() {
    let (sim, mut nodes) = Netsim::nodes(2, 6);
    let _pump = sim.start();
    nodes[1].ports.routes.want_own();
    let blob = nodes[1]
        .ports
        .routes
        .available_own(RouteClass::General)
        .await;
    attached(&mut nodes[1]).await;
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    let profile = SafetyProfile::default_dht();
    crate::tests::sending(&nodes[0]).await;
    sim.fault(Fault::Unanswered {
        node: 1,
        unanswered: true,
    });
    let started = tokio::time::Instant::now();
    assert!(nodes[0]
        .ports
        .messages
        .call(&target, &profile, vec![1])
        .await
        .is_err());
    assert!(started.elapsed() >= call_timeout(&profile));
    sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1],
    });
    nodes[0]
        .ports
        .messages
        .send(&target, &profile, vec![2])
        .await
        .unwrap();
    sim.quiesce().await;
    while let Ok(event) = nodes[1].events.try_recv() {
        assert!(
            !matches!(event, PortEvent::AppMessage { .. }),
            "partitioned"
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn route_death_reaches_the_owner_only() {
    let (sim, mut nodes) = Netsim::nodes(2, 8);
    let _pump = sim.start();
    nodes[1].ports.routes.want_own();
    let blob = nodes[1]
        .ports
        .routes
        .available_own(RouteClass::General)
        .await;
    attached(&mut nodes[0]).await;
    attached(&mut nodes[1]).await;
    nodes[0].ports.routes.import(&blob).unwrap();
    sim.fault(Fault::RouteDeath { node: 1 });
    let PortEvent::RouteChange { dead, .. } = recv(&mut nodes[1]).await else {
        panic!("expected a route change");
    };
    assert!(!dead.is_empty());
    // The importer hears nothing: its sends are lost.
    assert!(nodes[0].events.try_recv().is_err());
    // The owner forgets the dead route and allocates a new one, as the
    // transport's dispatch has it do.
    nodes[1].ports.routes.on_dead_own(&dead);
    let fresh = tokio::time::timeout(
        Duration::from_secs(30),
        nodes[1].ports.routes.available_own(RouteClass::General),
    )
    .await
    .unwrap();
    assert_ne!(fresh, blob);
    assert!(matches!(
        *nodes[1]
            .ports
            .routes
            .own_state(RouteClass::General)
            .borrow(),
        RouteState::Available { .. }
    ));
}

/// While a node's automatic pool routes fail end to end, an app message
/// still arrives: it is pinned to our tested `Sending` route
/// (`AppMessagePort::send`). An unpinned call, whose safety and reply
/// routes come from the pool, is lost and times out.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_statement_rides_our_sending_route_past_failing_pool_routes() {
    let (sim, mut nodes) = Netsim::nodes(2, 25);
    let pump = sim.start();
    nodes[1].ports.routes.want_own();
    let blob = nodes[1]
        .ports
        .routes
        .available_own(RouteClass::General)
        .await;
    attached(&mut nodes[0]).await;
    attached(&mut nodes[1]).await;
    crate::tests::sending(&nodes[0]).await;
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    let profile = SafetyProfile::default_dht();
    sim.fault(Fault::PoolRoutesFail {
        node: 0,
        failing: true,
    });
    nodes[0]
        .ports
        .messages
        .send(&target, &profile, b"pinned".to_vec())
        .await
        .unwrap();
    assert!(matches!(
        recv(&mut nodes[1]).await,
        PortEvent::AppMessage { ref message, .. } if message == b"pinned"
    ));
    let started = tokio::time::Instant::now();
    assert!(nodes[0]
        .ports
        .messages
        .call(&target, &profile, vec![1])
        .await
        .is_err());
    assert!(started.elapsed() >= call_timeout(&profile));
    pump.abort();
}
