//! Live A/B finding 64 and the 2026-10-24 register: Jon's `DmAck`s are
//! never queued (`dm::ack`). A lost one is covered by his next, cumulative
//! ack (its `through`, his cursor on Ida's chain); with none to come, Ida
//! asks again: her landed row is hinted again (framed afresh, not appended
//! again) after its first re-ask wait, Jon, who already holds the message,
//! acknowledges it again, and the row settles delivered.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::latency::{GET, MESSAGE, SET};
use rekindle_netsim::Fault;
use rekindle_types::message::{
    SendState, CALL_TIMEOUT_MS, DM_ACK_DELAY_MAX_MS, DM_ACK_DELAY_MIN_MS,
};

use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, key, send, talk, LIMIT};

async fn history(node: &SimNode, peer: &str) -> Vec<rekindle_types::dto::dm::DmMessage> {
    serde_json::from_value(
        node.ok(IpcRequest::DmHistory {
            peer_key: peer.to_string(),
            limit: 100,
            before: None,
        })
        .await,
    )
    .expect("the history")
}

async fn received(node: &SimNode, peer: &str, body: &str) -> bool {
    history(node, peer)
        .await
        .iter()
        .any(|m| !m.is_own && m.body == body)
}

async fn delivered(node: &SimNode, peer: &str, body: &str) -> bool {
    history(node, peer)
        .await
        .iter()
        .any(|m| m.is_own && m.body == body && m.send_state == SendState::Delivered)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_landed_message_whose_acks_were_lost_is_hinted_again_until_delivered() {
    let (sim, mut nodes) = start(2, 78).await;
    for (node, name) in nodes.iter_mut().zip(["Ida", "Jon"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "hello").await;
    let (ida, jon) = (key(&nodes[0]), key(&nodes[1]));
    assert!(
        eventually(&sim, LIMIT, || delivered(&nodes[0], &jon, "hello: 0 to 1")).await,
        "hello settled"
    );

    send(&nodes[0], &jon, "acked late").await;
    // The fault is timed off the hint's arrival, which `eventually` sees
    // within its 250 ms poll: Jon's ack leaves no sooner than the
    // coalescing delay's 2 s minimum after he committed the message, so
    // every ack of this first copy is dropped.
    assert!(Duration::from_millis(DM_ACK_DELAY_MIN_MS) > Duration::from_millis(250) * 4);
    assert!(
        eventually(&sim, SET.min / 2, || received(
            &nodes[1],
            &ida,
            "acked late"
        ))
        .await,
        "Jon has it from its hint"
    );
    sim.fault(Fault::DropMessages { per_mille: 1000 });
    // Held past the latest ack of the first copy and past the append's
    // landing (a `HEAD` read and two sets), so the ack comes after it.
    let ack_window = Duration::from_millis(DM_ACK_DELAY_MAX_MS) + MESSAGE.max;
    let append = GET.max + SET.max * 2;
    tokio::time::sleep(ack_window.max(append)).await;
    assert!(
        !delivered(&nodes[0], &jon, "acked late").await,
        "every ack of the first copy was lost"
    );
    sim.fault(Fault::DropMessages { per_mille: 0 });

    // The append, if its sets were still going (a `HEAD` read and two
    // sets), then the first re-ask 2 x (ack delay + call timeout) after
    // the landing, then Jon's ack delay.
    let first_reask = Duration::from_millis(2 * (DM_ACK_DELAY_MAX_MS + CALL_TIMEOUT_MS));
    let bound = append + first_reask + ack_window + MESSAGE.max * 2;
    let healed = tokio::time::Instant::now();
    assert!(
        eventually(&sim, bound, || delivered(&nodes[0], &jon, "acked late")).await,
        "the re-hint got Jon's ack again within {bound:?} (after {:?})",
        healed.elapsed()
    );
    let copies = history(&nodes[1], &ida)
        .await
        .iter()
        .filter(|m| !m.is_own && m.body == "acked late")
        .count();
    assert_eq!(copies, 1, "Jon holds the message once");
}

/// A/B round 5 finding R5-1: the suspect round (plan RH1r-c S2) that the
/// first send past the ack deadline runs never holds that send's hint
/// behind its probe. Ida's "first" is hinted and Jon's acks are lost, so
/// 30 s later Ida suspects Jon's route; her next send ("second") runs the
/// round: a re-read of Jon's route record, then an `app_call` probe of the
/// route we hold. The probe's answer never changes which route a hint
/// takes (a newer record replaces it; with none the held route stays in
/// use, answered or not), so the hint goes once the re-read is done and the
/// probe runs beside it. Here every call to Jon hangs, so a probe ends only
/// at our `app_call` deadline (11 s): live, the hint of the re-ask waited
/// on it, and a lost DmText came back 42 s after its send instead of on the
/// re-ask's cadence.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_suspect_rounds_probe_never_holds_the_hint_back() {
    use rekindle_netsim::latency::DEADLINE;
    use rekindle_route::liveness::ACK_TIMEOUT_PROVEN;
    use rekindle_transport::broadcast::deadline::app_call_deadline;
    use rekindle_types::config::{
        SafetyProfile, SequencingPreference, StabilityPreference, ANONYMITY_HOP_FLOOR,
    };

    let (sim, mut nodes) = start(2, 79).await;
    for (node, name) in nodes.iter_mut().zip(["Ida", "Jon"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "hello").await;
    let (ida, jon) = (key(&nodes[0]), key(&nodes[1]));
    assert!(
        eventually(&sim, LIMIT, || delivered(&nodes[0], &jon, "hello: 0 to 1")).await,
        "hello settled: Jon's route is proven"
    );
    sim.fault(Fault::CallHangs {
        node: 1,
        hangs: true,
    });

    send(&nodes[0], &jon, "first").await;
    assert!(
        eventually(&sim, SET.min / 2, || received(&nodes[1], &ida, "first")).await,
        "Jon has the first from its hint"
    );
    // Every frame Jon sends is lost until Ida's ack deadline has passed,
    // so Ida hears nothing from him after the first's hint.
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 1000,
    });
    tokio::time::sleep(ACK_TIMEOUT_PROVEN + MESSAGE.max).await;
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 0,
    });

    // The round's re-read (one profile subkey read, bounded by the fanout
    // deadline at worst), then the hint: well inside the probe's deadline.
    let probe = app_call_deadline(&SafetyProfile {
        hop_count: ANONYMITY_HOP_FLOOR,
        stability: StabilityPreference::default(),
        sequencing: SequencingPreference::default(),
    });
    let bound = GET.max + MESSAGE.max * 2;
    assert!(bound * 4 < probe, "a held hint would show");
    assert!(GET.max < DEADLINE.min);
    send(&nodes[0], &jon, "second").await;
    let sent = tokio::time::Instant::now();
    assert!(
        eventually(&sim, bound, || received(&nodes[1], &ida, "second")).await,
        "Jon has the second within {bound:?}, not after the probe's {probe:?} (after {:?})",
        sent.elapsed()
    );
}
