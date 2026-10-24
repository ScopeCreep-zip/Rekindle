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

/// Ida's hint is lost and her append's `HEAD` read waits on an unready
/// internet (the pool's readiness wait), so the append is still in flight
/// at the first re-ask: that re-ask resends the hint, Jon has the message
/// before the entry could land, and Ida's row settles delivered once it has.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn append_outlives_reask_then_maintenance_settles() {
    use rekindle_netsim::latency::{GET, MESSAGE, SET};
    use rekindle_outbox::DM_FIRST_REASK;
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
    // The internet comes back this long after the send: the append's
    // `HEAD` read, then its page and `HEAD` sets, cannot land before the
    // first re-ask plus a few message latencies.
    let unready = DM_FIRST_REASK.saturating_sub(GET.max + SET.min);
    let bound = DM_FIRST_REASK + MESSAGE.max * 4;
    assert!(
        unready + GET.min + SET.min * 2 > bound,
        "the append outlasts the re-ask"
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
