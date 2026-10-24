//! A removal completes (live A/B 2026-10-23): Rae removes Ann, Ann drops
//! Rae and acknowledges, and Rae's `Unfriended` settles on that ack, so
//! it is not re-sent into a session Ann already deleted. Live, Ann framed
//! the ack after her removal deleted the session, found none, and never
//! sent it; Rae re-sent `Unfriended` until its deadline. Live A/B finding
//! 62: an ack sent once by app message and lost left Rae re-sending into a
//! session Ann no longer had, so the ack goes by `app_call` and is
//! attempted again until Rae's receipt.
//!
//! The ack is seen by Rae's `RemoveAcknowledged`; only after it does the
//! row probe ([`row_gone`]) run, since a Discard of a row still waiting
//! would end the removal itself.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::subscription_events::FriendEvent;

use crate::harness::{eventually, start, SimNode};
use crate::logs;
use crate::pair::{befriend, is_accepted_friend, key, LIMIT};

fn acknowledged(event: &FriendEvent) -> Option<&str> {
    match event {
        FriendEvent::RemoveAcknowledged { peer_key } => Some(peer_key),
        _ => None,
    }
}

/// Whether Rae's `Unfriended` row to `ann` is gone (no row to discard).
async fn row_gone(rae: &SimNode, ann: &str) -> bool {
    rae.call(IpcRequest::OutboxDiscard {
        client_id: format!("unfriended:{ann}"),
    })
    .await
    .is_err()
}

/// How many `Unfriended` frames were dropped for want of a session. An ack
/// attempted again after Rae forgot Ann is dropped by Rae the same way (its
/// row ends `Superseded` on her NAK), and is not counted.
fn unfriended_dropped() -> usize {
    logs::warned("ratchet step failed")
        .iter()
        .filter(|line| line.contains("Unfriended") && !line.contains("UnfriendedAck"))
        .count()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_removed_friend_acknowledges_and_the_removal_ends() {
    let (sim, mut nodes) = start(2, 101).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[1], &nodes[0]).await;
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    let events = nodes[0].events().await;
    nodes[0]
        .ok(IpcRequest::FriendRemove {
            public_key: ann.clone(),
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async {
            !is_accepted_friend(&nodes[1], &rae).await
        })
        .await,
        "Ann dropped Rae"
    );
    assert!(
        eventually(&sim, LIMIT, || async {
            events.count(&ann, acknowledged) > 0
        })
        .await,
        "Rae heard Ann's ack"
    );
    assert!(
        row_gone(&nodes[0], &ann).await,
        "Rae's removal ended on Ann's ack"
    );
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert!(
        logs::warned("UnfriendedAck not committed").is_empty(),
        "Ann framed her ack"
    );
    assert!(
        logs::warned("ratchet step failed").is_empty(),
        "no Unfriended was re-sent into a deleted session"
    );
}

/// Finding 62: everything Ann sends is lost while Rae removes her (a
/// one-direction loss, `Fault::DropFrom`): Rae's `Unfriended` arrives, Ann
/// drops Rae, and her first ack is lost. Ann attempts the ack again until
/// Rae's receipt; Rae's removal ends and nothing more is re-sent into the
/// session Ann deleted. Sent once by app message, the ack settled `sent` as
/// it left and Rae never heard it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_ack_is_attempted_again_and_the_removal_ends() {
    let (sim, mut nodes) = start(2, 102).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[1], &nodes[0]).await;
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    let events = nodes[0].events().await;
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 1000,
    });
    nodes[0]
        .ok(IpcRequest::FriendRemove {
            public_key: ann.clone(),
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async {
            !is_accepted_friend(&nodes[1], &rae).await
        })
        .await,
        "Ann dropped Rae"
    );
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(
        events.count(&ann, acknowledged),
        0,
        "the first ack was lost"
    );
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 0,
    });
    assert!(
        eventually(&sim, LIMIT * 20, || async {
            events.count(&ann, acknowledged) > 0
        })
        .await,
        "Ann attempted the ack again and Rae heard it"
    );
    assert!(
        eventually(&sim, LIMIT, || row_gone(&nodes[0], &ann)).await,
        "Rae's removal ended on Ann's ack"
    );
    let dropped = unfriended_dropped();
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(
        unfriended_dropped(),
        dropped,
        "no Unfriended was re-sent after the removal ended"
    );
    assert!(
        logs::warned("UnfriendedAck not committed").is_empty(),
        "Ann framed her ack"
    );
}
