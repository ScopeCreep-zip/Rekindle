//! A friend offline for a day (plan E2.5, Verify): Bea detaches and is cut
//! off from Ann for 24 simulated hours while Ann sends three messages. Back,
//! Bea gets all three exactly once and in order (from Ann's chain and her
//! outbox's resends), and Ann's rows read delivered after Bea's `DmAck`.
//! The day passes on the simulated clocks (both nodes' clocks move on
//! together), not in real time.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::dto::dm::DmMessage;
use rekindle_types::message::SendState;

use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, key, send, talk, LIMIT};

async fn history(node: &SimNode, peer: &str) -> Vec<DmMessage> {
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

const HOUR_MS: i64 = 60 * 60 * 1000;

fn delivered(state: SendState) -> bool {
    matches!(state, SendState::Delivered | SendState::Read)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "E2.B finding F3: under per-message latency the history does not keep send order"]
async fn a_day_offline_friend_gets_every_message_once_in_order() {
    let (sim, mut nodes) = start(2, 113).await;
    for (node, name) in nodes.iter_mut().zip(["Ann", "Bea"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    // Sessions and both participant records exist before Bea leaves.
    talk(&sim, &nodes[0], &nodes[1], "hello").await;
    let (ann, bea) = (key(&nodes[0]), key(&nodes[1]));

    // Bea is gone: no DHT, no watch change, no app message either way.
    sim.fault(Fault::Detach {
        node: 1,
        detached: true,
    });
    sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1],
    });
    let bodies: Vec<String> = (1..=3).map(|i| format!("while away {i}")).collect();
    for (hours, body) in (8..).step_by(8).zip(&bodies) {
        send(&nodes[0], &bea, body).await;
        // The outbox attempts (and backs off) before the clocks move on.
        tokio::time::sleep(Duration::from_secs(60)).await;
        for node in 0..2 {
            sim.fault(Fault::ClockSkew {
                node,
                ms: hours * HOUR_MS,
            });
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
    // 24 simulated hours on: nothing reached Bea, and Ann's rows wait.
    let got = history(&nodes[1], &ann).await;
    assert!(
        bodies.iter().all(|b| !got.iter().any(|m| &m.body == b)),
        "nothing reached Bea while she was away"
    );
    let own = history(&nodes[0], &bea).await;
    for body in &bodies {
        let row = own
            .iter()
            .find(|m| m.is_own && &m.body == body)
            .expect("Ann's row");
        assert!(
            !delivered(row.send_state) && row.send_state != SendState::Failed,
            "{body:?} still waits for Bea, not failed"
        );
    }

    sim.fault(Fault::Heal);
    sim.fault(Fault::Detach {
        node: 1,
        detached: false,
    });
    assert!(
        eventually(&sim, LIMIT * 4, || async {
            let got = history(&nodes[1], &ann).await;
            bodies
                .iter()
                .all(|b| got.iter().any(|m| !m.is_own && &m.body == b))
        })
        .await,
        "every message reached Bea"
    );
    let mut got: Vec<String> = history(&nodes[1], &ann)
        .await
        .into_iter()
        .filter(|m| !m.is_own)
        .map(|m| m.body)
        .collect();
    let mut expected: Vec<String> = vec!["hello: 0 to 1".into()];
    expected.extend(bodies.iter().cloned());
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

    assert!(
        eventually(&sim, LIMIT * 4, || async {
            let own = history(&nodes[0], &bea).await;
            bodies.iter().all(|b| {
                own.iter()
                    .any(|m| m.is_own && &m.body == b && delivered(m.send_state))
            })
        })
        .await,
        "Ann's rows read delivered after Bea's DmAck"
    );
}
