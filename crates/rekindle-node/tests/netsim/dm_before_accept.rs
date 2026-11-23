//! Live A/B finding R2-2 (plan E2.5i): Jon accepts Ida's request and
//! writes to her at once; his accept's `app_call` is lost (as round 2's
//! were, R2-1) while his message, wrapped in the same PreKeyInit, arrives
//! by `app_message`. Ida still holds the request as `PendingOut`, so her
//! admission holds the authenticated message and applies it when the
//! accept commits: she shows it, once, before Jon's first re-ask, and his
//! row settles by her ack. Before the hold she dropped it and only his
//! re-ask, 30 s on, delivered it.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::latency::MESSAGE;
use rekindle_netsim::{Fault, Writes};
use rekindle_types::message::{SendState, CALL_TIMEOUT_MS, DM_ACK_DELAY_MAX_MS};

use crate::harness::{eventually, start, SimNode};
use crate::pair::{accept, is_accepted_friend, key, request, send, talk, LIMIT};

const BODY: &str = "right after the accept";

async fn history(node: &SimNode, peer: &str) -> Vec<rekindle_types::dto::dm::DmMessage> {
    serde_json::from_value(
        node.ok(IpcRequest::DmHistory {
            peer_key: peer.to_string(),
            limit: 50,
            before: None,
        })
        .await,
    )
    .expect("the history")
}

async fn copies(node: &SimNode, peer: &str, own: bool) -> usize {
    history(node, peer)
        .await
        .iter()
        .filter(|m| m.is_own == own && m.body == BODY)
        .count()
}

async fn delivered(node: &SimNode, peer: &str) -> bool {
    history(node, peer)
        .await
        .iter()
        .any(|m| m.is_own && m.body == BODY && m.send_state == SendState::Delivered)
}

/// When an unacknowledged `dm` row is first asked for again
/// (`rekindle_outbox::stream::DM_FIRST_REASK`).
fn first_reask() -> Duration {
    Duration::from_millis(2 * (DM_ACK_DELAY_MAX_MS + CALL_TIMEOUT_MS))
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_message_sent_before_the_accept_lands_is_held_and_shown_at_the_accept() {
    let (sim, mut nodes) = start(3, 89).await;
    for (node, name) in nodes.iter_mut().zip(["Ida", "Jon"]) {
        node.unlock_new_identity(name).await;
    }
    let (ida, jon) = (key(&nodes[0]), key(&nodes[1]));
    request(&sim, &nodes[0], &nodes[1]).await;
    // Calls to Ida hang from here (Jon's accept among them); messages
    // to her still arrive.
    sim.fault(Fault::CallHangs {
        node: 0,
        hangs: true,
    });
    // The accept's second path (plan RV2) is held back too: Jon's write of
    // it into Ida's reply inbox lands below consensus until the calls heal.
    sim.fault(Fault::WriteFails {
        node: 1,
        of: Writes::Subkey {
            subkey: rekindle_types::dht_layout::request_inbox::REQUEST,
            record: None,
        },
    });
    accept(&nodes[1], &nodes[0]).await;
    let sent = tokio::time::Instant::now();
    send(&nodes[1], &ida, BODY).await;
    // The message's hint reaches Ida before any accept does.
    tokio::time::sleep(MESSAGE.max * 2).await;
    assert!(
        !is_accepted_friend(&nodes[0], &jon).await,
        "the accept has not reached Ida"
    );
    assert_eq!(copies(&nodes[0], &jon, false).await, 0, "held, not shown");
    sim.fault(Fault::CallHangs {
        node: 0,
        hangs: false,
    });
    sim.fault(Fault::Heal);
    assert!(
        eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &jon)).await,
        "the accept's retry reached Ida"
    );
    assert!(
        eventually(
            &sim,
            first_reask().saturating_sub(sent.elapsed()),
            || async { copies(&nodes[0], &jon, false).await > 0 }
        )
        .await,
        "Ida shows the held message at the accept, before any re-ask (after {:?})",
        sent.elapsed()
    );
    let bound = Duration::from_millis(DM_ACK_DELAY_MAX_MS) + MESSAGE.max * 2;
    assert!(
        eventually(&sim, bound, || delivered(&nodes[1], &ida)).await,
        "Jon's row settled by Ida's ack of the applied message (after {:?})",
        sent.elapsed()
    );
    tokio::time::sleep(first_reask()).await;
    assert_eq!(copies(&nodes[0], &jon, false).await, 1, "shown once");
    // One session: the accept, under the same base key, landed on the
    // session the held message opened (plan E2.5h), so both ways open.
    talk(&sim, &nodes[0], &nodes[1], "after").await;
}
