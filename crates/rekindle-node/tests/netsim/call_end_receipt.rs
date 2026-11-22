//! A call's `CallEnd` is receipted (plan CE4, decision 22): committed by
//! the production path the desktop's hang-up takes
//! (`dm::handshake::enqueue_until`), it goes by `app_call` and its row
//! stays open until the peer's receipt. One lost on the way is sent again
//! until it lands; one whose receipt is lost is sent again, and the peer,
//! which already opened it, answers the duplicate with the same receipt
//! and changes nothing (the daemon holds no call, E4.1, so it ignores it).

use std::time::Duration;

use rekindle_codec::capnp_codec::direct::{CallClosed, DirectBody};
use rekindle_netsim::{Fault, Netsim, SIM_EPOCH_MS};
use rekindle_types::message::SendState;

use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, key, talk, LIMIT};

/// A day past the simulated epoch: no run here reaches it.
const DEADLINE_MS: i64 = SIM_EPOCH_MS as i64 + 86_400_000;

/// Commit a `CallEnd` from `from` to `to` as the desktop's hang-up does.
async fn hang_up(from: &SimNode, to: &SimNode) -> String {
    let body = DirectBody::CallEnd(CallClosed {
        call_id: "call-ce4".to_string(),
        reason: "hangup".to_string(),
    });
    let host = from.host.friend_host();
    rekindle_transport::operations::dm::handshake::enqueue_until(
        host.as_ref(),
        &key(to),
        &body,
        DEADLINE_MS,
    )
    .await
    .expect("the CallEnd row is committed")
}

/// The state and attempt count of `node`'s outbox row `client_id`, while
/// it is open (a settled row is deleted, `repo::outbox::settle`).
async fn row(node: &SimNode, client_id: &str) -> Option<(SendState, u32)> {
    let db = node
        .host
        .identity_stores()
        .db
        .current()
        .expect("an open database");
    let (owner, id) = (key(node), client_id.to_string());
    let row = db
        .call(move |conn| rekindle_db::repo::outbox::get(conn, &owner, &id))
        .await
        .expect("the outbox read")?;
    Some((row.state, row.attempts))
}

async fn settled(sim: &Netsim, node: &SimNode, client_id: &str) -> bool {
    eventually(sim, LIMIT * 20, || async {
        row(node, client_id).await.is_none()
    })
    .await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_call_end_stays_open_and_is_sent_again_until_its_receipt() {
    let (sim, mut nodes) = start(2, 241).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    // Every app message and call is lost while Ann hangs up.
    sim.fault(Fault::DropMessages { per_mille: 1000 });
    let id = hang_up(&nodes[1], &nodes[0]).await;
    tokio::time::sleep(Duration::from_secs(120)).await;
    let (state, attempts) = row(&nodes[1], &id).await.expect("the CallEnd row is open");
    assert_eq!(state, SendState::Queued, "no receipt: the row stays open");
    assert!(
        attempts >= 2,
        "the unreceipted CallEnd was sent again ({attempts})"
    );
    sim.fault(Fault::DropMessages { per_mille: 0 });
    assert!(
        settled(&sim, &nodes[1], &id).await,
        "the CallEnd was sent again until its receipt came"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_duplicate_call_end_is_receipted_and_ignored() {
    let (sim, mut nodes) = start(2, 243).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    // Rae's answers are lost: the CallEnd reaches her, its receipt does not.
    sim.fault(Fault::Unanswered {
        node: 0,
        unanswered: true,
    });
    let id = hang_up(&nodes[1], &nodes[0]).await;
    tokio::time::sleep(Duration::from_secs(120)).await;
    let (state, attempts) = row(&nodes[1], &id).await.expect("the CallEnd row is open");
    assert_eq!(state, SendState::Queued, "the receipt was lost: still open");
    assert!(attempts >= 2, "Ann sent the CallEnd again ({attempts})");
    sim.fault(Fault::Unanswered {
        node: 0,
        unanswered: false,
    });
    // The next copy is a duplicate at Rae: answered with the same receipt.
    assert!(
        settled(&sim, &nodes[1], &id).await,
        "the duplicate CallEnd was receipted"
    );
    // Ignored: the session carries messages both ways as before.
    talk(&sim, &nodes[0], &nodes[1], "after the duplicate CallEnd").await;
}
