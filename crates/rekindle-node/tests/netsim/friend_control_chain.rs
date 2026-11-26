//! Control frames to an accepted friend ride the DM chain (plan RV2c, lead
//! decision 67, D22): a `FriendsKey` and the pairwise group kinds to a
//! friend are entries of the sender's chain, hinted by `app_message`, so a
//! friend whose calls stall (`Fault::CallStalls`: each call to its routes
//! is stuck before it is sent, a slow path, not a lost one) still reads
//! them; the receipted `app_call` they went by waited for an answer that
//! did not come. A friend offline at the send reads it from the chain at
//! its unlock's catch-up, its calls still stalled.

use std::time::Duration;

use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest};
use rekindle_netsim::Fault;
use rekindle_types::presence::SessionStatus;
use rekindle_types::subscription_events::{PresenceEvent, SubscriptionEvent};

use crate::group_dm::view;
use crate::harness::{eventually, start, Events, SimNode, PASSPHRASE};
use crate::pair::{befriend, key, talk, LIMIT};

/// Longer than any assertion here waits: a call to the stalled node never
/// leaves within the test.
const STALL: Duration = Duration::from_secs(3600);

fn saw(events: &Events, peer: &str, status: &str) -> bool {
    events.any(|event| {
        matches!(event, SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key, snapshot, ..
        }) if peer_key == peer && snapshot.status.as_deref() == Some(status))
    })
}

/// Rae, Ann and Cy, Rae a friend of both, Rae and Ann having talked (each
/// holds the other's chain record).
async fn rae_ann_cy(seed: u64) -> (rekindle_netsim::Netsim, Vec<SimNode>) {
    let (sim, mut nodes) = start(3, seed).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann", "Cy"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    befriend(&sim, &nodes[0], &nodes[2]).await;
    talk(&sim, &nodes[0], &nodes[1], "before the stall").await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    (sim, nodes)
}

/// Rae removes Cy, which rotates her presence key to Ann, and goes away.
async fn rotate_and_go_away(rae: &SimNode, cy: String) {
    rae.ok(IpcRequest::FriendRemove { public_key: cy }).await;
    rae.ok(IpcRequest::PresenceSet {
        status: SessionStatus::Away,
    })
    .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn friendskey_chain_ahead() {
    let (sim, nodes) = rae_ann_cy(201).await;
    let (rae, cy) = (key(&nodes[0]), key(&nodes[2]));
    let events = nodes[1].events().await;
    sim.fault(Fault::CallStalls {
        node: 1,
        stall: Some(STALL),
    });
    rotate_and_go_away(&nodes[0], cy).await;
    assert!(
        eventually(&sim, LIMIT, || async { saw(&events, &rae, "away") }).await,
        "Ann opens Rae's status under the key on Rae's chain, her calls stalled"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn friendskey_chain_ahead_after_unlock() {
    let (sim, nodes) = rae_ann_cy(202).await;
    let (rae, cy) = (key(&nodes[0]), key(&nodes[2]));
    let ann = nodes[1].public_key.clone().expect("an identity");
    nodes[1].ok(IpcRequest::Lock).await;
    sim.fault(Fault::CallStalls {
        node: 1,
        stall: Some(STALL),
    });
    rotate_and_go_away(&nodes[0], cy).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    nodes[1]
        .ok(IpcRequest::Unlock {
            identity: ann,
            passphrase: PASSPHRASE.into(),
        })
        .await;
    let events = nodes[1].events().await;
    assert!(
        eventually(&sim, LIMIT * 2, || async { saw(&events, &rae, "away") }).await,
        "Ann, offline at the rotation, reads the key from Rae's chain at unlock"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn group_invite_chain_ahead() {
    let (sim, nodes) = rae_ann_cy(203).await;
    sim.fault(Fault::CallStalls {
        node: 1,
        stall: Some(STALL),
    });
    let gid: String = serde_json::from_value(
        nodes[0]
            .ok(IpcRequest::GroupDm(GroupDmRequest::Create {
                name: "raid night".to_string(),
                members: vec![key(&nodes[1]), key(&nodes[2])],
                origin_peer: None,
            }))
            .await,
    )
    .expect("the group is created");
    assert!(
        eventually(&sim, LIMIT, || async {
            view(&nodes[1], &gid).await.is_some_and(|v| v.pending)
        })
        .await,
        "Ann holds the invite from Rae's chain, her calls stalled"
    );
    assert!(
        eventually(&sim, LIMIT, || async {
            view(&nodes[2], &gid).await.is_some_and(|v| v.pending)
        })
        .await,
        "Cy holds the invite"
    );
}
