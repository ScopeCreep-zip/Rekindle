//! An append that outlasts the re-ask (live A/B finding 73; research
//! `prod_research_batch9.md`): one append is in flight per stream and it is
//! never cancelled, so the first re-ask resends the hint only, and the row
//! settles once its entry lands and the peer's ack is in, whichever path
//! (the appender, a late completion, the maintenance tick) saw it land.

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

async fn has(node: &SimNode, peer: &str, body: &str) -> bool {
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

/// Ida's hint is lost and her internet is unready, so her entry cannot land:
/// the row is due again and re-hinted, Jon has the message before the
/// entry could land, and Ida's row settles delivered once it has.
///
/// The re-hint is the row's backoff after its hand-off (`Outbox::hand_off`,
/// `DM_RETRY_FLOOR` plus the first step), past the append's deadline
/// (`appender.rs` `select_due`), not the first re-ask of a `sent` row
/// (`DM_FIRST_REASK`): this row never reached `sent`. The internet stays
/// unready until that re-hint has been delivered.
///
/// Premise rewritten for Veilid 0.5.8 (E2.5i). It read: the internet returns
/// at `DM_FIRST_REASK - GET.max - SET.min`, with a bound of `DM_FIRST_REASK`
/// plus four message latencies, the append's network `HEAD` read waiting on
/// readiness so the append stayed in flight across the re-ask. With 0.5.8's
/// sets (VU2a `SET.min` 0.33 s) the append lands 1.06 s after the internet
/// returns, inside that bound, so the window could not be staged; and the
/// append no longer reads its own `HEAD` from the network (`rekindle-records`
/// `chain::own_head_at`), so nothing of it waits on readiness: offline, its page set
/// fails at once (0.5.8 `set_value.rs:247-255`) and the row is attempted again
/// at its backoff. That the re-ask of an append still in flight resends only
/// the hint is `rekindle-outbox`'s `reask_never_starts_second_append`.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn append_outlives_reask_then_maintenance_settles() {
    use rekindle_netsim::latency::{MESSAGE, SET};
    use rekindle_outbox::{OutboxRetry, StreamKind, DM_APPEND_DEADLINE};
    let (sim, mut nodes) = start(2, 91).await;
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
    // The latest the handed-off row is due again (its first backoff step
    // past the floor, `OutboxRetry::delay`), and so re-hinted.
    let OutboxRetry::Backoff { base, floor, .. } = StreamKind::Dm.policy() else {
        panic!("a dm row backs off");
    };
    let rehint = floor.max(DM_APPEND_DEADLINE) + base;
    let bound = rehint + MESSAGE.max * 4;
    // The internet comes back at the bound: the append's page and `HEAD`
    // sets cannot land before it.
    let unready = bound;
    assert!(
        unready + SET.min * 2 > bound,
        "the entry cannot land before the re-hint is in"
    );
    sim.fault(Fault::InternetReady {
        node: 0,
        ready: false,
    });
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 1000,
    });
    send(&nodes[0], &jon, "slow append").await;
    let sent = tokio::time::Instant::now();
    tokio::time::sleep(MESSAGE.max * 2).await;
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 0,
    });
    tokio::time::sleep(unready.saturating_sub(sent.elapsed())).await;
    sim.fault(Fault::InternetReady {
        node: 0,
        ready: true,
    });
    assert!(
        eventually(&sim, bound.saturating_sub(sent.elapsed()), || has(
            &nodes[1],
            &ida,
            "slow append"
        ))
        .await,
        "the re-ask resent the hint while the append was in flight (after {:?})",
        sent.elapsed()
    );
    assert!(
        eventually(&sim, LIMIT, || delivered(&nodes[0], &jon, "slow append")).await,
        "Ida's row settles delivered once its entry landed (after {:?})",
        sent.elapsed()
    );
    let copies = history(&nodes[1], &ida)
        .await
        .iter()
        .filter(|m| !m.is_own && m.body == "slow append")
        .count();
    assert_eq!(copies, 1, "Jon has it once");
}
