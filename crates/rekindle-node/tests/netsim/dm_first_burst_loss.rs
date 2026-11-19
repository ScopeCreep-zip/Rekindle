//! Live finding L-DM1 (Linux, 3 of 30 runs): the first burst on a new
//! friendship (the hint of Ida's first message and her `StreamRecord`, the
//! one frame that names her chain to Jon) is lost in transit with no error,
//! and the route carries her next frame fine. Her `StreamRecord` goes as an
//! `app_call` (`dm::outbox::record_call`), so its loss is seen within the
//! call timeout and it is sent again at once; Jon then diffs her chain and
//! shows the message well before the first re-ask (`DM_FIRST_REASK`, 30 s),
//! the only recovery an `app_message` has.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::latency::{GET, MESSAGE, SET};
use rekindle_netsim::Fault;
use rekindle_types::message::{CALL_TIMEOUT_MS, DM_ACK_DELAY_MAX_MS};

use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, key, send};

/// How long everything Ida sends is lost: past her first message's hint
/// and the `StreamRecord` its chain's creation queues (a record create and
/// a set).
const LOSS: Duration = Duration::from_secs(4);

async fn shows(node: &SimNode, peer: &str, body: &str) -> bool {
    let history: Vec<rekindle_types::dto::dm::DmMessage> = serde_json::from_value(
        node.ok(IpcRequest::DmHistory {
            peer_key: peer.to_string(),
            limit: 100,
            before: None,
        })
        .await,
    )
    .expect("the history");
    history.iter().any(|m| !m.is_own && m.body == body)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_new_friendships_lost_first_burst_arrives_well_before_the_first_re_ask() {
    let (sim, mut nodes) = start(3, 89).await;
    for (node, name) in nodes.iter_mut().zip(["Ida", "Jon"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let (ida, jon) = (key(&nodes[0]), key(&nodes[1]));
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 1000,
    });
    let started = tokio::time::Instant::now();
    send(&nodes[0], &jon, "one").await;
    tokio::time::sleep(LOSS).await;
    assert!(!shows(&nodes[1], &ida, "one").await, "the burst was lost");
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 0,
    });
    // The lost call's timeout, its retry's first step (at most 1 s), the
    // call, Jon's diff of her chain (a read and its page), and a set's
    // worth of slack: far under the first re-ask.
    let bound = Duration::from_millis(CALL_TIMEOUT_MS)
        + Duration::from_secs(1)
        + MESSAGE.max * 2
        + GET.max * 2
        + SET.max;
    let first_reask = Duration::from_millis(2 * (DM_ACK_DELAY_MAX_MS + CALL_TIMEOUT_MS));
    assert!(LOSS + bound < first_reask / 2);
    assert!(
        eventually(&sim, bound, || shows(&nodes[1], &ida, "one")).await,
        "Jon shows it within {bound:?} of the loss ending (after {:?})",
        started.elapsed()
    );
}
