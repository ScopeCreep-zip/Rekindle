//! Live A/B finding 73: a brand-new friendship's first hints (Ida's first
//! three messages to Jon, her `StreamRecord` among them) are shown at
//! receipt and settle by Jon's acks before any re-ask. The guard of the
//! replace-only cursor reset is the re-friend scenario (it fails with the
//! reset on every new key) and `repo::stream_cursors`' unit test; the
//! fourth-ack scenario passes either way at seed 97, where the
//! `StreamRecord` reaches Jon before hint 1 (review LV19 F5, run locally).

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::latency::{MESSAGE, SET};
use rekindle_netsim::{Fault, Netsim};
use rekindle_types::message::{SendState, CALL_TIMEOUT_MS, DM_ACK_DELAY_MAX_MS};

use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, is_accepted_friend, key, send, talk, LIMIT};

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
    fourth_ack_covers_the_first_three(&sim, &nodes, &ida, &jon).await;
}

/// Live A/B finding 77: after a removal and a new friendship, Ida's new
/// chain's first hints step Jon's cursor and her new `StreamRecord` does
/// not put it back, so his fourth ack's `through` settles all four (at
/// f0e141f8 the old reset held hint 3 past a gap at cursor 0).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_refriended_peers_new_chain_steps_the_cursor_from_its_first_hint() {
    let (sim, nodes, ida, jon) = new_friends(97).await;
    talk(&sim, &nodes[0], &nodes[1], "before").await;
    talk(&sim, &nodes[0], &nodes[1], "and again").await;
    nodes[0]
        .ok(IpcRequest::FriendRemove {
            public_key: jon.clone(),
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async {
            !is_accepted_friend(&nodes[1], &ida).await
        })
        .await,
        "Jon heard of the removal"
    );
    befriend(&sim, &nodes[0], &nodes[1]).await;
    fourth_ack_covers_the_first_three(&sim, &nodes, &ida, &jon).await;
}

/// Jon's DHT is off and his acks of Ida's three messages are lost, so only
/// his cursor, stepped by their hints (her `StreamRecord` among them), can
/// settle them: his ack of a fourth carries `through` = 4 and settles all
/// four before any re-ask.
async fn fourth_ack_covers_the_first_three(sim: &Netsim, nodes: &[SimNode], ida: &str, jon: &str) {
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
        send(&nodes[0], jon, body).await;
    }
    for body in BODIES {
        assert!(
            eventually(sim, SET.min / 2, || has(&nodes[1], ida, body, false, None)).await,
            "Jon shows {body} at its hint"
        );
    }
    tokio::time::sleep(ack_window()).await;
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 0,
    });
    send(&nodes[0], jon, "four").await;
    let delivered = Some(SendState::Delivered);
    for body in BODIES.into_iter().chain(["four"]) {
        assert!(
            eventually(sim, SET.min / 2 + ack_window() + MESSAGE.max * 2, || has(
                &nodes[0], jon, body, true, delivered
            ))
            .await,
            "{body} settled by the fourth's through: {:?}",
            states(&nodes[0], jon).await
        );
    }
    assert!(
        started.elapsed() < first_reask(),
        "settled before any re-ask"
    );
}

/// Live A/B finding 73b: Jon's ack of a message Ida sent once their
/// conversation was set up is lost (his sends dropped while it is owed),
/// and he holds the message; her `StreamRecord` settled long before, so no
/// resend of it carries a fresh ack. Her first re-ask's re-hint, framed
/// afresh because the entry landed (`dm::outbox::rehint`), opens on Jon
/// under the same stream seq and is acknowledged again, and her row
/// settles at that re-ask, not at a later one.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_first_ack_is_answered_again_at_the_re_hint() {
    let (sim, nodes, ida, jon) = new_friends(101).await;
    talk(&sim, &nodes[0], &nodes[1], "hello").await;
    assert!(
        eventually(&sim, first_reask(), || has(
            &nodes[0],
            &jon,
            "hello: 0 to 1",
            true,
            Some(SendState::Delivered)
        ))
        .await,
        "hello settled"
    );
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 1000,
    });
    let started = tokio::time::Instant::now();
    send(&nodes[0], &jon, "one").await;
    assert!(
        eventually(&sim, SET.min / 2, || has(
            &nodes[1], &ida, "one", false, None
        ))
        .await,
        "Jon shows it at its hint"
    );
    tokio::time::sleep(ack_window()).await;
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 0,
    });
    assert!(
        !has(&nodes[0], &jon, "one", true, Some(SendState::Delivered)).await,
        "the first ack was lost"
    );
    let bound = first_reask() + ack_window() + MESSAGE.max * 2;
    assert!(
        eventually(&sim, bound, || has(
            &nodes[0],
            &jon,
            "one",
            true,
            Some(SendState::Delivered)
        ))
        .await,
        "settled by the re-ack within {bound:?} (after {:?}): {:?}",
        started.elapsed(),
        states(&nodes[0], &jon).await
    );
}
