//! A removal completes (live A/B 2026-10-23): Rae removes Ann, Ann drops
//! Rae and acknowledges, and Rae's `Unfriended` settles on that ack, so
//! it is not re-sent into a session Ann already deleted. Live, Ann framed
//! the ack after her removal deleted the session, found none, and never
//! sent it; Rae re-sent `Unfriended` until its deadline.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;

use crate::harness::{eventually, start};
use crate::logs;
use crate::pair::{befriend, is_accepted_friend, key, LIMIT};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_removed_friend_acknowledges_and_the_removal_ends() {
    let (sim, mut nodes) = start(2, 101).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[1], &nodes[0]).await;
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
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
    // Rae's Unfriended settled on Ann's ack: no row is left to discard.
    assert!(
        eventually(&sim, LIMIT, || async {
            nodes[0]
                .call(IpcRequest::OutboxDiscard {
                    client_id: format!("unfriended:{ann}"),
                })
                .await
                .is_err()
        })
        .await,
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
