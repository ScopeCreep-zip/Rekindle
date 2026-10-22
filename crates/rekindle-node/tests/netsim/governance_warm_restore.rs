//! The warm cache (plan E3.1 C1): a member who unlocks with the network
//! cut has every community and its last merged governance at once, from
//! the identity's `governance_entries_cache`, on the daemon as on the GUI.
//! The community intents are reviewed from that state: `CommunityLeave`
//! and `RoleUpdate` are held for review, never refused 503 "governance not
//! loaded".

use std::time::Duration;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_netsim::Fault;

use crate::harness::{Community, PASSPHRASE};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unlock_with_the_network_cut_answers_from_the_cache() {
    let community = Community::of(2, 131).await;
    let node = &community.nodes[1];
    let key = node.public_key.clone().expect("an identity");
    // The joiner's merge input lands in the cache before the lock.
    tokio::time::sleep(Duration::from_secs(60)).await;
    node.ok(IpcRequest::Lock).await;
    community.sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    node.ok(IpcRequest::Unlock {
        identity: key,
        passphrase: PASSPHRASE.into(),
    })
    .await;
    let governance_key = community.governance_key.clone();
    let leave = node
        .answer(IpcRequest::CommunityLeave {
            governance_key: governance_key.clone(),
        })
        .await;
    assert!(
        matches!(leave, IpcResponse::IntentRequired(_)),
        "CommunityLeave is reviewed from the cache: {leave:?}"
    );
    let update = node
        .answer(IpcRequest::RoleUpdate {
            community: governance_key,
            role_id: 0,
            name: Some("Everyone".into()),
            permissions: None,
            color: None,
        })
        .await;
    assert!(
        !matches!(update, IpcResponse::Error { code: 503, .. }),
        "RoleUpdate is not refused for want of governance: {update:?}"
    );
}
