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
use crate::pair::{befriend, key, send, talk};

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

/// Bo's app calls to others: every app call he makes (plan RV1: no route
/// of his is self-tested by an app call; Veilid's keepalive tests them; a
/// suspect round's probe of Ada's route is a call to her).
fn calls_to_others(sim: &rekindle_netsim::Netsim) -> u64 {
    sim.app_calls(1)
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

/// Plan LF9 rewritten for RV1 (Linux r22: Ada's General route took nothing
/// from outside ~4 min while Veilid's keepalive passed; it never declares
/// such a route dead, 0.5.8 `route_stats.rs:19-29`). With no app self-test,
/// the evidence rotates it: Ada's sends await Bo's acks, which enter her
/// dead route, so nothing arrives over it for `SILENCE_TRIGGER`, and the
/// owner rotates within [`worst_detection`] plus one [`republish`]; the
/// messages both ways land.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_general_route_dead_from_outside_is_rotated_on_evidence() {
    use crate::requester_route_bounds::{republish, worst_detection, MARGIN};
    let (sim, nodes) = friends(127).await;
    let ada = key(&nodes[0]);
    let bo = key(&nodes[1]);
    let before = sim.route_blobs(0);
    assert!(!before.is_empty());
    sim.fault(Fault::OutsideDead {
        node: 0,
        class: RouteClass::General,
    });
    for i in 0..3 {
        send(&nodes[0], &bo, &format!("to bo {i}")).await;
        send(&nodes[1], &ada, &format!("from outside {i}")).await;
    }
    assert!(
        eventually(&sim, worst_detection() + republish() + MARGIN, || {
            let now = sim.route_blobs(0);
            let replaced = !now.is_empty() && now.iter().all(|b| !before.contains(b));
            async move { replaced }
        })
        .await,
        "the route dead from outside was rotated on evidence"
    );
    for i in 0..3 {
        for (node, peer, body) in [
            (&nodes[0], &bo, format!("to bo {i}")),
            (&nodes[1], &ada, format!("from outside {i}")),
        ] {
            assert!(
                eventually(&sim, Duration::from_secs(120), || delivered(
                    node, peer, &body
                ))
                .await,
                "{body} landed over Ada's replacement route"
            );
        }
    }
}
