//! Live A/B finding 73: a brand-new friendship's first hints (Ida's first
//! three messages to Jon, her `StreamRecord` among them) are shown at
//! receipt and step Jon's cursor on her chain, which her `StreamRecord`
//! no longer resets, so one ack settles them all before any re-ask.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::latency::{MESSAGE, SET};
use rekindle_netsim::{Fault, Netsim};
use rekindle_types::message::{SendState, CALL_TIMEOUT_MS, DM_ACK_DELAY_MAX_MS};

use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, key, send};

const BODIES: [&str; 3] = ["one", "two", "three"];

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

async fn states(node: &SimNode, peer: &str) -> Vec<(String, SendState)> {
    history(node, peer)
        .await
        .into_iter()
        .map(|m| (m.body, m.send_state))
        .collect()
}

fn ack_window() -> Duration {
    Duration::from_millis(DM_ACK_DELAY_MAX_MS) + MESSAGE.max
}

fn first_reask() -> Duration {
    Duration::from_millis(2 * (DM_ACK_DELAY_MAX_MS + CALL_TIMEOUT_MS))
}

/// Ida and Jon, friends with no message yet, and a third node holding
/// records.
async fn new_friends(seed: u64) -> (Netsim, Vec<SimNode>, String, String) {
    let (sim, mut nodes) = start(3, seed).await;
    for (node, name) in nodes.iter_mut().zip(["Ida", "Jon"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let (ida, jon) = (key(&nodes[0]), key(&nodes[1]));
    (sim, nodes, ida, jon)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_new_friendships_first_hints_show_at_once_and_settle_by_one_ack() {
    let (sim, nodes, ida, jon) = new_friends(83).await;
    let started = tokio::time::Instant::now();
    for body in BODIES {
        send(&nodes[0], &jon, body).await;
    }
    for body in BODIES {
        assert!(
            eventually(&sim, SET.min / 2, || has(
                &nodes[1], &ida, body, false, None
            ))
            .await,
            "Jon shows {body} at its hint: {:?}",
            states(&nodes[1], &ida).await
        );
    }
    let delivered = Some(SendState::Delivered);
    for body in BODIES {
        assert!(
            eventually(&sim, ack_window() + MESSAGE.max * 2, || has(
                &nodes[0], &jon, body, true, delivered
            ))
            .await,
            "{body} settled by Jon's ack: {:?}",
            states(&nodes[0], &jon).await
        );
    }
    assert!(
        started.elapsed() < first_reask(),
        "settled before any re-ask"
    );
}

/// Jon's acks of Ida's first three are lost and his DHT is off, so no
/// chain diff repairs his cursor: his ack of a fourth must still carry
/// `through` = 4, his cursor stepped by the first three hints, which her
/// `StreamRecord` among them no longer resets; it settles all four before
/// any re-ask.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_new_friendships_fourth_ack_covers_the_first_three_by_through() {
    let (sim, nodes, ida, jon) = new_friends(97).await;
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 1000,
    });
    let started = tokio::time::Instant::now();
    for body in BODIES {
        send(&nodes[0], &jon, body).await;
    }
    for body in BODIES {
        assert!(
            eventually(&sim, SET.min / 2, || has(
                &nodes[1], &ida, body, false, None
            ))
            .await,
            "Jon shows {body} at its hint"
        );
    }
    tokio::time::sleep(ack_window()).await;
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 0,
    });
    send(&nodes[0], &jon, "four").await;
    let delivered = Some(SendState::Delivered);
    for body in BODIES.into_iter().chain(["four"]) {
        assert!(
            eventually(&sim, SET.min / 2 + ack_window() + MESSAGE.max * 2, || has(
                &nodes[0], &jon, body, true, delivered
            ))
            .await,
            "{body} settled by the fourth's through: {:?}",
            states(&nodes[0], &jon).await
        );
    }
    assert!(
        started.elapsed() < first_reask(),
        "settled before any re-ask"
    );
}
