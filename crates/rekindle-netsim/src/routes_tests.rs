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
/// `RouteUnusable`, and the route lives: Veilid 0.5.8 recompiles it
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
        Err(PortError::RouteUnusable(_))
    ));
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
    sim.quiesce().await;
    assert!(nodes[0].events.try_recv().is_err(), "no dead_routes");
    assert!(sim.route_blobs(0).contains(&mine));
}

/// A route that stops carrying traffic dies by its loopback tests: first
/// tested 30 s after it broke, then once per RPC timeout and tick, and the
/// third failure kills an Ordered route, the fifth an Unordered one
/// (Veilid 0.5.8 `route_spec_store/mod.rs:51-63`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn loopback_failures_kill_ordered_after_3_and_unordered_after_5() {
    let (sim, mut nodes) = Netsim::nodes(1, 25);
    let _pump = sim.start();
    attached(&mut nodes[0]).await;
    nodes[0].ports.routes.want_own();
    nodes[0].ports.routes.want_own_class(RouteClass::Sending);
    for class in [RouteClass::General, RouteClass::Sending] {
        nodes[0].ports.routes.available_own(class).await;
    }
    sim.quiesce().await;
    while nodes[0].events.try_recv().is_ok() {}
    for (class, tests) in [(RouteClass::General, 3u32), (RouteClass::Sending, 5)] {
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
/// (today's Sending and Media, `RouteOrdering::of`): one ordering per
/// route, no fallback (Veilid 0.5.8 `route_allocate.rs:897-936`); General
/// (Ordered) allocates. Its live Sending route dies after 5 failed tests.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_udp_blocked_node_allocates_no_unordered_route() {
    let (sim, mut nodes) = Netsim::nodes(1, 26);
    let _pump = sim.start();
    attached(&mut nodes[0]).await;
    nodes[0].ports.routes.want_own();
    nodes[0].ports.routes.want_own_class(RouteClass::Sending);
    let sending = nodes[0]
        .ports
        .routes
        .available_own(RouteClass::Sending)
        .await;
    sim.fault(Fault::UdpBlocked {
        node: 0,
        blocked: true,
    });
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(!live(&sim, &sending), "the UDP route died");
    // The Host forwards each death to `OwnRoutes`, which tries to reallocate.
    while let Ok(event) = nodes[0].events.try_recv() {
        if let PortEvent::RouteChange { dead, .. } = event {
            nodes[0].ports.routes.on_dead_own(&dead);
        }
    }
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert!(nodes[0]
        .ports
        .routes
        .own_blob(RouteClass::Sending)
        .is_none());
    assert!(nodes[0].ports.routes.own_blob(RouteClass::Media).is_none());
    assert!(nodes[0]
        .ports
        .routes
        .own_blob(RouteClass::General)
        .is_some());
    sim.fault(Fault::UdpBlocked {
        node: 0,
        blocked: false,
    });
    let back = tokio::time::timeout(
        Duration::from_secs(300),
        nodes[0].ports.routes.available_own(RouteClass::Sending),
    )
    .await;
    assert!(back.is_ok(), "UDP back, Sending allocates");
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
        matches!(recv(&mut nodes[1]).await, PortEvent::AppMessage { message } if message == b"in")
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
        matches!(recv(&mut nodes[0]).await, PortEvent::AppMessage { message } if message == b"out")
    );
}
