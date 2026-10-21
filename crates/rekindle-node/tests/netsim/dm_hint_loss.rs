//! DM hint loss (plan E2.5, D23): every direct message is appended to the
//! sender's chain before its `app_message` leaves, and that message is only
//! a hint. With 30% of app messages and calls dropped, every message still
//! arrives exactly once and in order, through the diff of the chain that a
//! later hint (or a resend's) triggers; and no chain page holds plaintext.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;

use crate::harness::{eventually, start, SimNode, PASSPHRASE};
use crate::pair::{befriend, key, retired, send, talk, unretired, LIMIT};

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

/// How long after the last send every message is on Jon's side (E2.B
/// F2), from the calibrated latencies: the append in flight and the one
/// that carries the queued run (the outbox hands a stream's due rows over
/// together; each append is a `HEAD` read and two sets, page then `HEAD`),
/// the hint, then a running diff's re-read and a full diff, each reading
/// `HEAD` and the retire mark at once (the empty mark runs to the fanout
/// deadline), one get per page and one ack per entry.
fn recovery_bound(entries: u32, pages: u32) -> Duration {
    use rekindle_netsim::latency::{DEADLINE, GET, MESSAGE, SET};
    let append = GET.max + SET.max * 2;
    let diff = DEADLINE.max + GET.max * pages + MESSAGE.max * entries;
    append * 2 + MESSAGE.max + diff * 2
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
    let sent = tokio::time::Instant::now();

    let mut expected: Vec<String> = vec!["hello: 0 to 1".into()];
    expected.extend(bodies.iter().cloned());
    expected.push(last.clone());
    assert!(
        eventually(&sim, recovery_bound(21, 3), || async {
            let got = received(&nodes[1], &ida).await;
            expected.iter().all(|b| got.contains(b))
        })
        .await,
        "every message reached Jon within {:?} of the last send (after {:?})",
        recovery_bound(21, 3),
        sent.elapsed()
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

async fn delivered(node: &SimNode, peer: &str, body: &str) -> bool {
    let history: Vec<rekindle_types::dto::dm::DmMessage> = serde_json::from_value(
        node.ok(IpcRequest::DmHistory {
            peer_key: peer.to_string(),
            limit: 100,
            before: None,
        })
        .await,
    )
    .expect("the history");
    history.iter().any(|m| {
        m.is_own && m.body == body && m.send_state == rekindle_types::message::SendState::Delivered
    })
}

/// The first `StreamRecord` is lost: the outbox resends it until Jon's
/// `DmAck` names it, and Jon then reads Ida's chain.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_stream_record_is_resent_until_acknowledged() {
    let (sim, mut nodes) = start(2, 72).await;
    for (node, name) in nodes.iter_mut().zip(["Ida", "Jon"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let (ida, jon) = (key(&nodes[0]), key(&nodes[1]));
    // Ida's first message creates her record; its StreamRecord and the
    // message's hint are both lost.
    sim.fault(Fault::DropMessages { per_mille: 1000 });
    send(&nodes[0], &jon, "first").await;
    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    sim.fault(Fault::DropMessages { per_mille: 0 });
    assert!(
        eventually(&sim, LIMIT * 4, || async {
            received(&nodes[1], &ida)
                .await
                .contains(&"first".to_string())
        })
        .await,
        "Jon reads the first message once the resend got through"
    );
    // The chain is readable now: lost hints are recovered from it.
    sim.fault(Fault::DropMessages { per_mille: 300 });
    let bodies: Vec<String> = (1..=10).map(|i| format!("after {i:02}")).collect();
    for body in &bodies {
        send(&nodes[0], &jon, body).await;
    }
    sim.fault(Fault::DropMessages { per_mille: 0 });
    send(&nodes[0], &jon, "last").await;
    assert!(
        eventually(&sim, LIMIT * 4, || async {
            let got = received(&nodes[1], &ida).await;
            bodies.iter().all(|b| got.contains(b)) && got.contains(&"last".to_string())
        })
        .await,
        "every message after the resend reached Jon"
    );
}

/// Ida retires her chain past entries Jon acknowledged (by their hints)
/// but never diffed (his DHT was off); 31 days on, his diff stores a
/// `HistoryGap` marker in his history.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_retired_stretch_is_a_stored_history_gap() {
    let (sim, mut nodes) = start(2, 73).await;
    for (node, name) in nodes.iter_mut().zip(["Ida", "Jon"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "hello").await;
    // Finding F8: the test finds Ida's chain by a watched change of it that
    // reached Jon. "hello" can land before his watch on the chain is held
    // (it is placed when her participant record reaches him), and then his
    // hint diff reads it first and no change of it reaches him; once the
    // watch is held, the next entry's change does (at the replica's next
    // tick, so Jon stays online until it has).
    talk(&sim, &nodes[0], &nodes[1], "watched").await;
    assert!(
        eventually(&sim, LIMIT, || async { !unretired(&sim, 1).is_empty() }).await,
        "a change of Ida's chain reached Jon"
    );
    let (ida, jon) = (key(&nodes[0]), key(&nodes[1]));
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    send(&nodes[0], &jon, "unread on the chain").await;
    assert!(
        eventually(&sim, LIMIT, || delivered(
            &nodes[0],
            &jon,
            "unread on the chain"
        ))
        .await,
        "Jon acknowledged it from its hint"
    );
    sim.fault(Fault::ClockSkew {
        node: 0,
        ms: 31 * 24 * 60 * 60 * 1000,
    });
    let keys = unretired(&sim, 1);
    send(&nodes[0], &jon, "after the horizon").await;
    // The send is committed, not yet appended: the append retires the
    // stretch, and Jon must not be back (his watch and catch-up read the
    // chain at once) before it has.
    assert!(
        eventually(&sim, LIMIT, || async { retired(&sim, &keys) }).await,
        "Ida's chain is retired"
    );
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: false,
    });
    send(&nodes[0], &jon, "once more").await;
    let gap = || async {
        let history: Vec<rekindle_types::dto::dm::DmMessage> = serde_json::from_value(
            nodes[1]
                .ok(IpcRequest::DmHistory {
                    peer_key: ida.clone(),
                    limit: 100,
                    before: None,
                })
                .await,
        )
        .expect("the history");
        history.iter().any(|m| m.history_gap_through.is_some())
    };
    assert!(
        eventually(&sim, LIMIT * 2, gap).await,
        "Jon's history holds the gap marker"
    );
    assert!(
        eventually(&sim, LIMIT, || async {
            received(&nodes[1], &ida)
                .await
                .contains(&"once more".to_string())
        })
        .await,
        "the messages after the gap arrive"
    );
}

/// The friend maintenance's retire tick (plan E2.5): Ida's chain is
/// retired past entries Jon acknowledged but never diffed with no further
/// send from her: 31 days on, her next tick retires them while he is still
/// away, and the diff of his next unlock stores the `HistoryGap` marker.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_retire_tick_retires_a_quiet_conversation() {
    let (sim, mut nodes) = start(2, 74).await;
    for (node, name) in nodes.iter_mut().zip(["Ida", "Jon"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "hello").await;
    let (ida, jon) = (key(&nodes[0]), key(&nodes[1]));
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    send(&nodes[0], &jon, "unread on the chain").await;
    assert!(
        eventually(&sim, LIMIT, || delivered(
            &nodes[0],
            &jon,
            "unread on the chain"
        ))
        .await,
        "Jon acknowledged it from its hint"
    );
    let keys = unretired(&sim, 1);
    sim.fault(Fault::ClockSkew {
        node: 0,
        ms: 31 * 24 * 60 * 60 * 1000,
    });
    // No send from Ida: only her maintenance tick (~3 min) can retire, and
    // Jon stays away until it has (back, he would read the stretch first).
    assert!(
        eventually(&sim, LIMIT * 4, || async { retired(&sim, &keys) }).await,
        "Ida's tick retired her chain"
    );
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: false,
    });
    let gap = || async {
        let history: Vec<rekindle_types::dto::dm::DmMessage> = serde_json::from_value(
            nodes[1]
                .ok(IpcRequest::DmHistory {
                    peer_key: ida.clone(),
                    limit: 100,
                    before: None,
                })
                .await,
        )
        .expect("the history");
        history.iter().any(|m| m.history_gap_through.is_some())
    };
    // Jon's next unlock catches up (`catch_up_all`) and diffs.
    nodes[1].ok(IpcRequest::Lock).await;
    nodes[1]
        .ok(IpcRequest::Unlock {
            identity: jon.clone(),
            passphrase: PASSPHRASE.into(),
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, gap).await,
        "Jon's history holds the gap marker"
    );
}
