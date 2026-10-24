//! Live A/B finding 71 refined: a hint carries its chain seq, and Jon's
//! cursor on Ida's chain advances on each hint applied in order, with no
//! chain diff needed (the hints arrive before the appends land). So his
//! next `DmAck.through` is the last chain seq he applied, and Ida settles
//! every row whose chain entry is at or below it: the acks Jon lost
//! (`Fault::DropFrom` on him) are repaired by his next ack, before Ida's
//! first re-ask would have asked for them again.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::latency::{MESSAGE, SET};
use rekindle_netsim::Fault;
use rekindle_types::message::{SendState, CALL_TIMEOUT_MS, DM_ACK_DELAY_MAX_MS};

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

async fn has(node: &SimNode, peer: &str, body: &str, own: bool, state: Option<SendState>) -> bool {
    history(node, peer)
        .await
        .iter()
        .any(|m| m.is_own == own && m.body == body && state.is_none_or(|s| m.send_state == s))
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn hints_in_order_advance_through_and_the_next_ack_repairs_a_lost_one() {
    let (sim, mut nodes) = start(3, 79).await;
    for (node, name) in nodes.iter_mut().zip(["Ida", "Jon"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "hello").await;
    let (ida, jon) = (key(&nodes[0]), key(&nodes[1]));
    let delivered = Some(SendState::Delivered);
    assert!(
        eventually(&sim, LIMIT, || has(
            &nodes[0],
            &jon,
            "hello: 0 to 1",
            true,
            delivered
        ))
        .await,
        "hello settled"
    );

    // Every ack Jon sends for the first two is lost. A third node holds
    // the records too, so Ida's appends land while Jon's sends drop.
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 1000,
    });
    let started = tokio::time::Instant::now();
    for body in ["one", "two"] {
        send(&nodes[0], &jon, body).await;
        assert!(
            eventually(&sim, SET.min / 2, || has(
                &nodes[1], &ida, body, false, None
            ))
            .await,
            "Jon has {body} from its hint"
        );
    }
    let ack_window = Duration::from_millis(DM_ACK_DELAY_MAX_MS) + MESSAGE.max;
    tokio::time::sleep(ack_window).await;
    for body in ["one", "two"] {
        assert!(
            !has(&nodes[0], &jon, body, true, delivered).await,
            "every ack of {body} was lost"
        );
    }
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 0,
    });

    // The third's ack names it in `through`, which covers the first two.
    send(&nodes[0], &jon, "three").await;
    let bound = SET.min / 2 + ack_window + MESSAGE.max * 2;
    for body in ["three", "one", "two"] {
        assert!(
            eventually(&sim, bound, || has(&nodes[0], &jon, body, true, delivered)).await,
            "{body} settled by the third's ack: {:?}",
            history(&nodes[0], &jon)
                .await
                .iter()
                .map(|m| (m.body.clone(), m.send_state))
                .collect::<Vec<_>>()
        );
    }
    let first_reask = Duration::from_millis(2 * (DM_ACK_DELAY_MAX_MS + CALL_TIMEOUT_MS));
    assert!(
        started.elapsed() < first_reask,
        "settled by through ({:?}), before the first re-ask ({first_reask:?})",
        started.elapsed()
    );
}
