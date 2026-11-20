//! The route model against the pinned Veilid ([`crate::routes`]).

use std::time::Duration;

use rekindle_records::port::{PortError, PortEvent, RouteClass};
use rekindle_types::config::SafetyProfile;

use crate::tests::{attached, recv, sending};
use crate::{Fault, Netsim, Node};

/// Whether the route `blob` names is live on the network.
fn live(sim: &Netsim, blob: &[u8]) -> bool {
    crate::routes::route_id(blob).is_some_and(|id| sim.net.inner.lock().routes.contains_key(&id))
}

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
    sending(&nodes[0]).await;
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
    sending(&nodes[0]).await;
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

/// Pinned calls lost to 5 distinct destinations inside one out-of-band
/// test's span kill the pinned route, reported to its owner as dead
/// (Veilid 0.5.8 `route_stats.rs:134-141`); spaced past that span, the
/// passing test clears them each time (`:536-543`), and unpinned losses
/// charge no own route (they count on Veilid's pool routes).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn five_lost_questions_kill_only_a_pinned_route_inside_one_test() {
    let (sim, mut nodes) = Netsim::nodes(6, 23);
    let _pump = sim.start();
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
    nodes[0].ports.routes.want_own();
    nodes[0].ports.routes.want_own_class(RouteClass::Sending);
    let sending = nodes[0]
        .ports
        .routes
        .available_own(RouteClass::Sending)
        .await;
    let profile = SafetyProfile::default_dht();
    let messages = nodes[0].ports.messages.clone();
    for target in &targets {
        assert!(messages.call(target, &profile, vec![1]).await.is_err());
        assert!(messages
            .call_pinned(target, &profile, RouteClass::Sending, vec![1])
            .await
            .is_err());
    }
    sim.quiesce().await;
    assert!(
        !matches!(
            nodes[0].events.try_recv(),
            Ok(PortEvent::RouteChange { .. })
        ),
        "spaced losses are cleared by each passing test"
    );
    let calls = targets.iter().map(|target| {
        let messages = messages.clone();
        let (target, profile) = (target.clone(), profile.clone());
        tokio::spawn(async move {
            messages
                .call_pinned(&target, &profile, RouteClass::Sending, vec![2])
                .await
        })
    });
    for call in calls.collect::<Vec<_>>() {
        assert!(call.await.unwrap().is_err());
    }
    let PortEvent::RouteChange { dead, .. } = recv(&mut nodes[0]).await else {
        panic!("expected a route change");
    };
    assert_eq!(dead.len(), 1);
    assert!(!live(&sim, &sending));
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
    sending(&nodes[0]).await;
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
/// dies after 5 failed tests; General and Sending (Ordered, plan VU
/// decision 2) live on, and Sending allocates while UDP is blocked.
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
    nodes[0].ports.routes.want_own_class(RouteClass::Sending);
    let sending = tokio::time::timeout(
        Duration::from_secs(120),
        nodes[0].ports.routes.available_own(RouteClass::Sending),
    )
    .await;
    assert!(sending.is_ok(), "Sending (Ordered) allocates without UDP");
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

/// R2-1: a pinned call over an Unordered route whose UDP path fails is
/// lost without an error, the question never reaching the callee (the
/// reply route fixes the safety route, Veilid 0.5.8
/// `route_compile.rs:436-447`, and Check #3b does not move it, `:568-569`),
/// while one pinned to an Ordered route lands. This is why Sending is
/// Ordered (plan VU decision 2).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_failed_udp_path_loses_calls_pinned_to_an_unordered_route_only() {
    let (sim, mut nodes) = Netsim::nodes(2, 28);
    let _pump = sim.start();
    let blob = own_blob(&nodes[1]).await;
    attached(&mut nodes[0]).await;
    attached(&mut nodes[1]).await;
    nodes[0].ports.routes.want_own();
    nodes[0].ports.routes.want_own_class(RouteClass::Sending);
    for class in [RouteClass::Media, RouteClass::Sending] {
        nodes[0].ports.routes.available_own(class).await;
    }
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    sim.fault(Fault::UdpBlocked {
        node: 0,
        blocked: true,
    });
    let profile = SafetyProfile::default_dht();
    let messages = nodes[0].ports.messages.clone();
    let lost = messages
        .call_pinned(&target, &profile, RouteClass::Media, vec![1])
        .await;
    assert!(
        lost.is_err(),
        "the question over the failed UDP path is lost"
    );
    sim.quiesce().await;
    assert!(
        !matches!(nodes[1].events.try_recv(), Ok(PortEvent::AppCall { .. })),
        "no question reached the callee"
    );
    let (caller, to) = (messages.clone(), target.clone());
    let call = tokio::spawn(async move {
        caller
            .call_pinned(&to, &profile, RouteClass::Sending, vec![2])
            .await
    });
    let PortEvent::AppCall { call_id, .. } = recv(&mut nodes[1]).await else {
        panic!("expected a call");
    };
    nodes[1]
        .ports
        .messages
        .reply(call_id, b"a".to_vec())
        .await
        .unwrap();
    assert!(
        call.await.unwrap().is_ok(),
        "the Ordered route's call lands"
    );
}

/// An unpinned call's reply route is a Veilid pool route nothing holds
/// mid-call: crossing 30 s since it was last valid while the question is
/// out, it is released and the answer is lost; a pinned call at the same
/// moment is answered over the published Sending route (plan VU2 item 14).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unpinned_reply_route_released_mid_call_loses_the_answer() {
    let (sim, mut nodes) = Netsim::nodes(2, 27);
    let _pump = sim.start();
    let blob = own_blob(&nodes[1]).await;
    attached(&mut nodes[0]).await;
    attached(&mut nodes[1]).await;
    nodes[0].ports.routes.want_own();
    nodes[0].ports.routes.want_own_class(RouteClass::Sending);
    nodes[0]
        .ports
        .routes
        .available_own(RouteClass::Sending)
        .await;
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    let profile = SafetyProfile::default_dht();
    let messages = nodes[0].ports.messages.clone();
    for pinned in [false, true] {
        let ask = || {
            let (caller, to, profile) = (messages.clone(), target.clone(), profile.clone());
            tokio::spawn(async move {
                if pinned {
                    caller
                        .call_pinned(&to, &profile, RouteClass::Sending, vec![1])
                        .await
                } else {
                    caller.call(&to, &profile, vec![1]).await
                }
            })
        };
        for late in [false, true] {
            if late {
                // The first answer made the reply route valid as it arrived.
                tokio::time::sleep(Duration::from_millis(29_700)).await;
            }
            let call = ask();
            let PortEvent::AppCall { call_id, .. } = recv(&mut nodes[1]).await else {
                panic!("expected a call");
            };
            nodes[1]
                .ports
                .messages
                .reply(call_id, b"a".to_vec())
                .await
                .unwrap();
            let answer = call.await.unwrap();
            assert_eq!(
                answer.is_ok(),
                pinned || !late,
                "pinned {pinned}, crossing the mark {late}: {answer:?}"
            );
        }
    }
}

/// A pinned call (RH1r): refused `Routing` while the caller's `Sending`
/// route is not live; answered over it while it is; and its answer lost
/// when the caller releases the route before the answer comes back
/// (Veilid 0.5.8 `rpc_processor/destination.rs:477-497`: the reply private route
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
    tokio::join!(sending(&nodes[0]), sending(&nodes[1]));
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
