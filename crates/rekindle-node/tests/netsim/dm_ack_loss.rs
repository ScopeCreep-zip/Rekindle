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
