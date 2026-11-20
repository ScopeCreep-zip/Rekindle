//! Route liveness (plan RH1r-c; live finding 75; research batch 12). A's
//! General route stops delivering with no word from Veilid
//! ([`Fault::RouteSilentDeath`], as live: its loopback failures dropped
//! while Offline, `ping_validator.rs:666-690`). A finds it by its own
//! round-trip self-test and republishes a replacement; B, whose sends go
//! unanswered, re-reads A's route at a bounded rate. A healthy, busy route
//! is never probed.

use std::time::Duration;

use rekindle_netsim::Fault;
use rekindle_records::port::RouteClass;
use rekindle_types::message::SendState;

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

/// Ada's self-tests of her General route, the one Bo's sends arrive over
/// (plan RT1: her `Sending` route is self-tested too, on its own schedule,
/// and is not the route this file's busy/idle rule is about).
fn general_probes(sim: &rekindle_netsim::Netsim) -> u64 {
    sim.own_calls(0, RouteClass::General)
}

/// Bo's app calls to others: every call but his own routes' self-tests
/// (plan RT1: those now include his `Sending` route's, on their own
/// schedule; a suspect round's probe of Ada's route is a call to her).
fn calls_to_others(sim: &rekindle_netsim::Netsim) -> u64 {
    let own: u64 = [RouteClass::General, RouteClass::Media, RouteClass::Sending]
        .into_iter()
        .map(|class| sim.own_calls(1, class))
        .sum();
    sim.app_calls(1) - own
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

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_silently_dead_inbound_route_is_found_and_replaced_and_the_acks_arrive() {
    let (sim, nodes) = friends(121).await;
    let bo = key(&nodes[1]);
    sim.fault(Fault::RouteSilentDeath {
        node: 0,
        class: RouteClass::General,
    });
    // Bo's acks of these go into Ada's dead route until she replaces it.
    for i in 0..3 {
        send(&nodes[0], &bo, &format!("into the dark {i}")).await;
    }
    for i in 0..3 {
        let body = format!("into the dark {i}");
        assert!(
            eventually(&sim, Duration::from_secs(200), || delivered(
                &nodes[0], &bo, &body
            ))
            .await,
            "{body} was acked within about 3 minutes"
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_reconnect_self_tests_at_once_and_replaces_a_dead_route() {
    let (sim, _nodes) = friends(122).await;
    let before = sim.route_blobs(0);
    assert!(!before.is_empty());
    sim.fault(Fault::RouteSilentDeath {
        node: 0,
        class: RouteClass::General,
    });
    sim.fault(Fault::InternetReady {
        node: 0,
        ready: false,
    });
    tokio::time::sleep(Duration::from_secs(1)).await;
    sim.fault(Fault::InternetReady {
        node: 0,
        ready: true,
    });
    // Well inside the 40-53 s schedule and three failures: only the
    // reconnect's immediate test finds it this soon.
    assert!(
        eventually(&sim, Duration::from_secs(30), || {
            let now = sim.route_blobs(0);
            let replaced = !now.is_empty() && now.iter().all(|b| !before.contains(b));
            async move { replaced }
        })
        .await,
        "the dead route was replaced and released after the reconnect"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn eight_unanswered_sends_make_bo_suspect_adas_route_at_most_once_a_minute() {
    let (sim, nodes) = friends(123).await;
    let ada = key(&nodes[0]);
    // Bo's chain is announced and its `StreamRecord` settled first: that
    // record goes as an app call until Ada receipts it (L-DM1), so with it
    // pending every call counted below would not be a probe.
    talk(&sim, &nodes[1], &nodes[0], "first").await;
    tokio::time::sleep(Duration::from_millis(
        rekindle_types::message::DM_ACK_DELAY_MAX_MS + rekindle_types::message::CALL_TIMEOUT_MS,
    ))
    .await;
    sim.fault(Fault::RouteSilentDeath {
        node: 0,
        class: RouteClass::General,
    });
    // Unanswered: nothing of Ada's reaches Bo either. Her route's silence
    // alone leaves Bo's sends answered, since she reads them from his chain
    // and acks them over his route, and any authenticated frame resets his
    // counters (plan RH1r-c S5; `direct/frame.rs` `open_ratchet`); healing
    // that case is her own self-test's (O1, O2, S6).
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 1000,
    });
    // A suspect round re-reads Ada's route and, finding the record Bo
    // holds, probes it with one app call; nothing else of Bo's calls to
    // others here (his own routes' self-tests are not counted).
    let calls = calls_to_others(&sim);
    for i in 0..9 {
        send(&nodes[1], &ada, &format!("unanswered {i}")).await;
    }
    // The round runs before Bo's next attempt to Ada once eight sends went
    // unanswered: the next of these rows' re-hints, within their re-ask
    // wait (and inside the 55 s ack deadline of a route not yet proven).
    assert!(
        eventually(&sim, Duration::from_secs(60), || {
            let now = calls_to_others(&sim);
            async move { now > calls }
        })
        .await,
        "eight unanswered sends began a suspect round"
    );
    let first = calls_to_others(&sim);
    for i in 0..9 {
        send(&nodes[1], &ada, &format!("still unanswered {i}")).await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    assert!(
        calls_to_others(&sim) <= first + 1,
        "no more than one more suspect round in the next minute"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_healthy_busy_route_is_never_probed_and_an_idle_one_is() {
    let (sim, nodes) = friends(124).await;
    let ada = key(&nodes[0]);
    let probes = general_probes(&sim);
    for i in 0..30 {
        send(&nodes[1], &ada, &format!("busy {i}")).await;
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
    assert_eq!(general_probes(&sim), probes, "a busy route is never probed");
    // Within its period of the last inbound (plan RH1r-c O1): a test that
    // falls due inside the 40 s idle gate is skipped and rescheduled one
    // period on, 40 s + U(0, 13.3 s), so the first idle test comes at most
    // 40 + 40 + 13.3 s after the last arrival, which was 10 s before here.
    assert!(
        eventually(&sim, Duration::from_secs(90), || {
            let now = general_probes(&sim);
            async move { now > probes }
        })
        .await,
        "an idle route is tested within its period"
    );
    assert!(eventually(&sim, LIMIT, || delivered(&nodes[1], &ada, "busy 29")).await);
}

/// Plan RT1 (Linux r7 probe: both Sending deaths came 200/331 s after
/// allocation, never self-tested): Ada's `Sending` route, never published,
/// stops carrying traffic with no word from Veilid. Her own self-test finds
/// it (first test 40-53.3 s after it came up, retests 10-20 s apart, three
/// failures) and allocates a replacement, which passes before the old one
/// is released.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_silently_dead_sending_route_is_found_by_its_self_test_and_replaced() {
    use rekindle_records::port::RouteState;
    let (sim, nodes) = friends(125).await;
    let sending = || {
        nodes[0]
            .routes
            .own_state(RouteClass::Sending)
            .borrow()
            .clone()
    };
    let RouteState::Available { blob: before } = sending() else {
        panic!("Ada's Sending route is live");
    };
    sim.fault(Fault::RouteSilentDeath {
        node: 0,
        class: RouteClass::Sending,
    });
    // 53.3 s to the first test, then two retests of at most 20 s, each
    // probe waiting out its 15 s deadline: about 2 min 40 s.
    assert!(
        eventually(&sim, Duration::from_secs(200), || {
            let replaced = matches!(sending(), RouteState::Available { blob } if blob != before);
            async move { replaced }
        })
        .await,
        "the dead Sending route was replaced by Ada's own self-test"
    );
}
