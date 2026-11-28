//! The route model against the pinned Veilid ([`crate::routes`]).

use std::time::Duration;

use rekindle_records::port::{PortError, PortEvent, RouteClass};
use rekindle_types::config::SafetyProfile;

use crate::tests::{attached, recv};
use crate::{Fault, Netsim, Node};

/// Whether the route `blob` names is live on the network.
fn live(sim: &Netsim, blob: &[u8]) -> bool {
    crate::routes::route_id(blob).is_some_and(|id| sim.net.inner.lock().routes.contains_key(&id))
}

pub(crate) async fn own_blob(node: &Node) -> Vec<u8> {
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

/// An imported route expires once untouched for 5 minutes, and the private
/// route management task's listing touches it on every tick it runs
/// (Veilid 0.5.8 `route_spec_store/mod.rs:313-324`,
/// `route_spec_store_cache.rs:348-357`): so it expires only while that
/// task is stopped, here the importer detached for 5 minutes. It then no
/// longer resolves (Veilid's `NoConnection`); forgotten and imported
/// again, it resolves.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_import_expires_after_five_minutes_without_the_management_task() {
    let (sim, mut nodes) = Netsim::nodes(2, 22);
    let _pump = sim.start();
    let blob = own_blob(&nodes[1]).await;
    attached(&mut nodes[0]).await;
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    let profile = SafetyProfile::default_dht();
    sim.fault(Fault::Detach {
        node: 0,
        detached: true,
    });
    tokio::time::sleep(Duration::from_secs(300)).await;
    sim.fault(Fault::Detach {
        node: 0,
        detached: false,
    });
    while !matches!(recv(&mut nodes[0]).await, PortEvent::Attachment { state, .. } if state == "attached_good")
    {
    }
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

/// A send failing at the first hop of the sender's safety route fails
/// `Send` (our side: the peer's route is kept), and our route lives: Veilid 0.5.8 recompiles it
/// (`route_stats.rs:19-35`), reports nothing, and the next send goes out
/// on it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_first_hop_failure_recompiles_and_kills_nothing() {
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
        Err(PortError::Send(_))
    ));
    nodes[0]
        .ports
        .messages
        .send(&target, &profile, vec![2])
        .await
        .unwrap();
    assert!(matches!(
        recv(&mut nodes[1]).await,
        PortEvent::AppMessage { message, .. } if message == vec![2]
    ));
    sim.quiesce().await;
    assert!(nodes[0].events.try_recv().is_err(), "no dead_routes");
    assert!(sim.route_blobs(0).contains(&mine));
}

/// A route that stops carrying traffic dies by its loopback tests: first
/// tested 30 s after it broke, then once per RPC timeout and tick, and the
/// third failure kills an Ordered route (General), the fifth an Unordered
/// one (Media) (Veilid 0.5.8 `route_spec_store/mod.rs:51-54`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn loopback_failures_kill_ordered_after_3_and_unordered_after_5() {
    let (sim, mut nodes) = Netsim::nodes(1, 25);
    let _pump = sim.start();
    attached(&mut nodes[0]).await;
    nodes[0].ports.routes.want_own();
    for class in [RouteClass::General, RouteClass::Media] {
        nodes[0].ports.routes.available_own(class).await;
    }
    sim.quiesce().await;
    while nodes[0].events.try_recv().is_ok() {}
    for (class, tests) in [(RouteClass::General, 3u32), (RouteClass::Media, 5)] {
        let old = nodes[0].ports.routes.own_blob(class).unwrap();
        sim.fault(Fault::LoopbackFailures { node: 0, class });
        let death = Duration::from_secs(30) + Duration::from_secs(6) * (tests - 1);
        tokio::time::sleep(death.saturating_sub(Duration::from_secs(2))).await;
        assert!(live(&sim, &old), "{class:?} lives until test {tests}");
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(!live(&sim, &old), "{class:?} dies at test {tests}");
    }
}

/// A node whose network blocks UDP cannot allocate an Unordered route
/// (Media, `RouteOrdering::of`): one ordering per route, no fallback
/// (Veilid 0.5.8 `route_allocate.rs:99-105`, `:936`). Its live Media route
/// dies after 5 failed tests; General (Ordered) lives on.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_udp_blocked_node_allocates_no_unordered_route() {
    let (sim, mut nodes) = Netsim::nodes(1, 26);
    let _pump = sim.start();
    attached(&mut nodes[0]).await;
    nodes[0].ports.routes.want_own();
    let media = nodes[0].ports.routes.available_own(RouteClass::Media).await;
    let general = nodes[0]
        .ports
        .routes
        .available_own(RouteClass::General)
        .await;
    sim.fault(Fault::UdpBlocked {
        node: 0,
        blocked: true,
    });
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(!live(&sim, &media), "the UDP route died");
    assert!(live(&sim, &general), "the Ordered route lives");
    // The Host forwards each death to `OwnRoutes`, which tries to reallocate.
    while let Ok(event) = nodes[0].events.try_recv() {
        if let PortEvent::RouteChange { dead, .. } = event {
            nodes[0].ports.routes.on_dead_own(&dead);
        }
    }
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert!(nodes[0].ports.routes.own_blob(RouteClass::Media).is_none());
    sim.fault(Fault::UdpBlocked {
        node: 0,
        blocked: false,
    });
    let back = tokio::time::timeout(
        Duration::from_secs(300),
        nodes[0].ports.routes.available_own(RouteClass::Media),
    )
    .await;
    assert!(back.is_ok(), "UDP back, Media allocates");
}

/// A one-direction loss (plan C9.4): what node 1 sends is dropped, its call
/// answers included, while what is sent to it arrives.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn drop_from_loses_only_what_the_node_sends() {
    let (sim, mut nodes) = Netsim::nodes(2, 23);
    let _pump = sim.start();
    let to_one = own_blob(&nodes[1]).await;
    let to_zero = own_blob(&nodes[0]).await;
    attached(&mut nodes[0]).await;
    attached(&mut nodes[1]).await;
    let profile = SafetyProfile::default_dht();
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 1000,
    });
    let one = nodes[0].ports.routes.import(&to_one).unwrap();
    nodes[0]
        .ports
        .messages
        .send(&one, &profile, b"in".to_vec())
        .await
        .unwrap();
    assert!(
        matches!(recv(&mut nodes[1]).await, PortEvent::AppMessage { message, .. } if message == b"in")
    );
    let caller = nodes[0].ports.messages.clone();
    let (target, call_profile) = (one.clone(), profile.clone());
    let call =
        tokio::spawn(async move { caller.call(&target, &call_profile, b"q".to_vec()).await });
    let PortEvent::AppCall { call_id, .. } = recv(&mut nodes[1]).await else {
        panic!("the call arrives");
    };
    nodes[1]
        .ports
        .messages
        .reply(call_id, b"a".to_vec())
        .await
        .unwrap();
    assert!(call.await.unwrap().is_err(), "the answer is lost");
    let zero = nodes[1].ports.routes.import(&to_zero).unwrap();
    nodes[1]
        .ports
        .messages
        .send(&zero, &profile, b"out".to_vec())
        .await
        .unwrap();
    sim.quiesce().await;
    while let Ok(event) = nodes[0].events.try_recv() {
        assert!(!matches!(event, PortEvent::AppMessage { .. }), "lost");
    }
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 0,
    });
    nodes[1]
        .ports
        .messages
        .send(&zero, &profile, b"out".to_vec())
        .await
        .unwrap();
    assert!(
        matches!(recv(&mut nodes[0]).await, PortEvent::AppMessage { message, .. } if message == b"out")
    );
}

/// An allocation runs its route's test before it returns (Veilid 0.5.8
/// `veilid_api/api.rs:812-842`): four status calls at once, each a
/// question and its answer, so between two and eight `MESSAGE` legs' worth
/// of simulated time. It is traced from start to end, and a call dropped
/// mid-flight is traced as cut and leaves no route behind.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_allocation_takes_its_route_test_and_a_drop_is_traced_as_cut() {
    use crate::latency::MESSAGE;
    use crate::{
        allocation_entry, FakeAllocator, ALLOCATION_CUT, ALLOCATION_ENDS, ALLOCATION_STARTS,
    };
    use rekindle_route::own_routes::RouteAllocator;

    let (sim, _nodes) = Netsim::nodes(1, 24);
    let alloc = FakeAllocator::new(0, std::sync::Arc::clone(&sim.net));
    let start = tokio::time::Instant::now();
    alloc.allocate(RouteClass::General).await.expect("a route");
    let took = start.elapsed();
    assert!(
        took >= MESSAGE.min * 2 && took <= MESSAGE.max * 2,
        "the route test's round trips, not instant: {took:?}"
    );
    let general: Vec<String> = sim.trace().of(0).map(ToString::to_string).collect();
    assert_eq!(
        general,
        [
            allocation_entry(ALLOCATION_STARTS, RouteClass::General),
            allocation_entry(ALLOCATION_ENDS, RouteClass::General),
        ]
    );
    let routes = sim.net.inner.lock().routes.len();
    let cut = tokio::time::timeout(MESSAGE.min, alloc.allocate(RouteClass::Media)).await;
    assert!(cut.is_err(), "still testing at one leg's shortest");
    assert_eq!(
        sim.trace().of(0).last(),
        Some(allocation_entry(ALLOCATION_CUT, RouteClass::Media).as_str())
    );
    assert_eq!(
        sim.net.inner.lock().routes.len(),
        routes,
        "a cut allocation leaves no route"
    );
}

/// A silently dead route (plan VU2 item 12): while the node's online
/// detector says Offline its loopback failures are deferred, so it outlives
/// any run of tests and no one hears of it (0.5.8 `ping_validator.rs:815-826`);
/// once a round trip proves the network (a returned readiness,
/// `online_detector.rs:1338-1350`) the failures count and the third kills
/// it (`route_spec_store/mod.rs:51-63`). A silent route its owner releases
/// leaves no run behind.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_silent_death_ends_by_release_or_by_tests_once_the_network_is_proven() {
    let (sim, mut nodes) = Netsim::nodes(1, 27);
    let _pump = sim.start();
    attached(&mut nodes[0]).await;
    nodes[0].ports.routes.want_own();
    let general = nodes[0]
        .ports
        .routes
        .available_own(RouteClass::General)
        .await;
    nodes[0].ports.routes.want_own_class(RouteClass::Media);
    let media = nodes[0].ports.routes.available_own(RouteClass::Media).await;
    sim.quiesce().await;
    while nodes[0].events.try_recv().is_ok() {}
    for class in [RouteClass::General, RouteClass::Media] {
        sim.fault(Fault::RouteSilentDeath { node: 0, class });
    }
    tokio::time::sleep(Duration::from_secs(300)).await;
    assert!(live(&sim, &general), "deferred while Offline");
    assert!(
        !std::iter::from_fn(|| nodes[0].events.try_recv().ok())
            .any(|e| matches!(e, PortEvent::RouteChange { .. })),
        "no one hears of a silent death"
    );
    let media_id = crate::routes::route_id(&media).unwrap();
    nodes[0].ports.routes.release_own_class(RouteClass::Media);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        !sim.net.inner.lock().faults.loopback.contains_key(&media_id),
        "a released silent route leaves no run"
    );
    sim.fault(Fault::InternetReady {
        node: 0,
        ready: false,
    });
    sim.fault(Fault::InternetReady {
        node: 0,
        ready: true,
    });
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(!live(&sim, &general), "counted again, the tests kill it");
}

/// F9 N1: under [`Fault::AllocationTestFails`] an allocation runs its
/// route's test and then answers `TryAgain("allocated route failed to
/// test")`, the route released (Veilid 0.5.8 `veilid_api/api.rs:829-836`):
/// it leaves no route; the other class is unaffected, and once cleared the
/// class allocates again.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_allocation_whose_test_fails_answers_try_again_and_leaves_no_route() {
    use crate::latency::MESSAGE;
    use crate::FakeAllocator;
    use rekindle_route::own_routes::{AllocError, RouteAllocator};

    let (sim, _nodes) = Netsim::nodes(1, 28);
    let alloc = FakeAllocator::new(0, std::sync::Arc::clone(&sim.net));
    let general = |sim: &Netsim| {
        sim.net
            .inner
            .lock()
            .routes
            .keys()
            .filter(|id| crate::routes::is_class(id, 0, 'g'))
            .count()
    };
    sim.fault(Fault::AllocationTestFails {
        node: 0,
        class: RouteClass::General,
        failing: true,
    });
    let start = tokio::time::Instant::now();
    let failed = alloc.allocate(RouteClass::General).await;
    let took = start.elapsed();
    assert!(
        matches!(&failed, Err(AllocError::TryAgain(why)) if why == "allocated route failed to test"),
        "{failed:?}"
    );
    assert!(took >= MESSAGE.min * 2, "after the route test: {took:?}");
    assert_eq!(general(&sim), 0, "the failed route is released");
    alloc
        .allocate(RouteClass::Media)
        .await
        .expect("only General's tests fail");
    sim.fault(Fault::AllocationTestFails {
        node: 0,
        class: RouteClass::General,
        failing: false,
    });
    alloc
        .allocate(RouteClass::General)
        .await
        .expect("cleared, General allocates");
    assert_eq!(general(&sim), 1);
}

/// The `dead_remote` ids `node` hears within `within`.
pub(crate) async fn dead_remote_within(node: &mut Node, within: Duration) -> Vec<String> {
    let mut heard = Vec::new();
    let _ = tokio::time::timeout(within, async {
        while let Some(event) = node.events.recv().await {
            if let PortEvent::RouteChange { dead_remote, .. } = event {
                heard.extend(dead_remote);
            }
        }
    })
    .await;
    heard
}

/// F9 N2: an importer tests the route it imported; once the owner releases
/// it, the next test gets no answer and Veilid removes it from the
/// importer's cache, reported as `dead_remote` (Veilid 0.5.8
/// `private_route_management.rs:396-414`, `ping_validator.rs:741-744`,
/// `route_spec_store_cache.rs:397-415`) within one test cadence: idle
/// [`crate::loopback::ROUTE_MIN_IDLE`], the answer wait, two ticks.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_importer_hears_dead_remote_once_the_owner_releases_the_route() {
    let (sim, mut nodes) = Netsim::nodes(2, 29);
    let _pump = sim.start();
    let blob = own_blob(&nodes[1]).await;
    attached(&mut nodes[0]).await;
    attached(&mut nodes[1]).await;
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    assert!(
        dead_remote_within(&mut nodes[0], Duration::from_secs(90))
            .await
            .is_empty(),
        "a live route passes its tests"
    );
    nodes[1].ports.routes.release_own_class(RouteClass::General);
    let cadence = crate::loopback::ROUTE_MIN_IDLE + crate::RPC_TIMEOUT + Duration::from_secs(2);
    assert_eq!(
        dead_remote_within(&mut nodes[0], cadence).await,
        vec![target.as_str().to_string()]
    );
}

/// F9 N2: a route dead from outside while its owner holds it
/// ([`Fault::OutsideDead`]) fails the importer's test, but while the
/// importer's blame is suppressed (its detector Offline) the failure is
/// deferred (`ping_validator.rs:733-739`); once a round trip proves its
/// network the retest releases it as `dead_remote`.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_outside_dead_route_is_released_by_its_importer_but_not_while_blame_is_suppressed() {
    let (sim, mut nodes) = Netsim::nodes(2, 30);
    let _pump = sim.start();
    let blob = own_blob(&nodes[1]).await;
    attached(&mut nodes[0]).await;
    attached(&mut nodes[1]).await;
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    sim.quiesce().await;
    // The importer's detector says Offline (it owns no route to break).
    sim.fault(Fault::RouteSilentDeath {
        node: 0,
        class: RouteClass::General,
    });
    sim.fault(Fault::OutsideDead {
        node: 1,
        class: RouteClass::General,
    });
    assert!(
        dead_remote_within(&mut nodes[0], Duration::from_secs(120))
            .await
            .is_empty(),
        "deferred while the importer's blame is suppressed"
    );
    assert!(live(&sim, &blob), "the owner still holds it");
    sim.fault(Fault::InternetReady {
        node: 0,
        ready: false,
    });
    sim.fault(Fault::InternetReady {
        node: 0,
        ready: true,
    });
    assert_eq!(
        dead_remote_within(&mut nodes[0], Duration::from_secs(15)).await,
        vec![target.as_str().to_string()]
    );
}
