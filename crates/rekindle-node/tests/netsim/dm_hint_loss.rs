//! DM hint loss (plan E2.5, D23): every direct message is appended to the
//! sender's chain before its `app_message` leaves, and that message is only
//! a hint. With 30% of app messages and calls dropped, every message still
//! arrives exactly once and in order, through the diff of the chain that a
//! later hint (or a resend's) triggers; and no chain page holds plaintext.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;

use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, key, send, talk, LIMIT};

async fn received(node: &SimNode, peer: &str) -> Vec<String> {
    let history: Vec<rekindle_types::dto::dm::DmMessage> = serde_json::from_value(
        node.ok(IpcRequest::DmHistory {
            peer_key: peer.to_string(),
            limit: 100,
            before: None,
        })
        .await,
    )
    .expect("the history");
    history
        .into_iter()
        .filter(|m| !m.is_own)
        .map(|m| m.body)
        .collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn lost_hints_are_recovered_from_the_chain_exactly_once_in_order() {
    let (sim, mut nodes) = start(2, 71).await;
    for (node, name) in nodes.iter_mut().zip(["Ida", "Jon"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    // Sessions and both participant records exist before the loss.
    talk(&sim, &nodes[0], &nodes[1], "hello").await;
    let (ida, jon) = (key(&nodes[0]), key(&nodes[1]));

    sim.fault(Fault::DropMessages { per_mille: 300 });
    let bodies: Vec<String> = (1..=20).map(|i| format!("chain message {i:02}")).collect();
    for body in &bodies {
        send(&nodes[0], &jon, body).await;
    }
    sim.fault(Fault::DropMessages { per_mille: 0 });
    let last = "chain message 21".to_string();
    send(&nodes[0], &jon, &last).await;

    let mut expected: Vec<String> = vec!["hello: 0 to 1".into()];
    expected.extend(bodies.iter().cloned());
    expected.push(last.clone());
    assert!(
        eventually(&sim, LIMIT * 4, || async {
            let got = received(&nodes[1], &ida).await;
            expected.iter().all(|b| got.contains(b))
        })
        .await,
        "every message reached Jon"
    );
    let mut got = received(&nodes[1], &ida).await;
    for body in &expected {
        assert_eq!(
            got.iter().filter(|b| *b == body).count(),
            1,
            "{body:?} once"
        );
    }
    // The history lists newest or oldest first; either way, in send order.
    if got.first() != expected.first() {
        got.reverse();
    }
    assert_eq!(got, expected, "in order");

    // A chain page carries ratchet ciphertext only.
    let pages = sim.single_subkey_values();
    assert!(!pages.is_empty(), "the chain pages are on the network");
    for (page_key, bytes) in &pages {
        for body in &expected {
            assert!(
                !bytes.windows(body.len()).any(|w| w == body.as_bytes()),
                "page {page_key} holds {body:?} in the clear"
            );
        }
    }
}
