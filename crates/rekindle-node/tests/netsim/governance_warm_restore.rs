//! The warm cache (plan E3.1 C1): a member who unlocks with the network
//! cut has every community and its last merged governance at once, from
//! the identity's `governance_entries_cache`, on the daemon as on the GUI.
//! The community intents are reviewed from that state: `CommunityLeave`
//! and `RoleUpdate` are held for review, never refused 503 "governance not
//! loaded".

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, IpcResponse};
use rekindle_ipc::protocol::{LifecycleRequest, MemberRequest};
use rekindle_netsim::Fault;

use crate::harness::{Community, PASSPHRASE};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unlock_with_the_network_cut_answers_from_the_cache() {
    let community = Community::of(2, 131).await;
    // The creator's genesis and the joiner's snapshot land in the cache.
    tokio::time::sleep(Duration::from_secs(60)).await;
    for i in [1, 0] {
        relock_offline(&community, i).await;
        reviewed_from_cache(&community.nodes[i], &community.governance_key).await;
    }
}

/// Lock node `i`, cut it off the DHT, unlock it again.
async fn relock_offline(community: &Community, i: usize) {
    let node = &community.nodes[i];
    let key = node.public_key.clone().expect("an identity");
    node.ok(IpcRequest::Lock).await;
    community.sim.fault(Fault::DhtOffline {
        node: i,
        offline: true,
    });
    node.ok(IpcRequest::Unlock {
        identity: key,
        passphrase: PASSPHRASE.into(),
    })
    .await;
}

/// `CommunityLeave` is held for review; `RoleUpdate` is not refused 503.
async fn reviewed_from_cache(node: &crate::harness::SimNode, governance_key: &str) {
    let governance_key = governance_key.to_string();
    let leave = node
        .answer(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityLeave {
                governance_key: governance_key.clone(),
            },
        )))
        .await;
    assert!(
        matches!(leave, IpcResponse::IntentRequired(_)),
        "CommunityLeave is reviewed from the cache: {leave:?}"
    );
    let update = node
        .answer(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleUpdate {
                community: governance_key,
                role_id: 0,
                name: Some("Everyone".into()),
                permissions: None,
                color: None,
                position: None,
                hoist: None,
                mentionable: None,
                self_assignable: None,
                exclusion_group: rekindle_types::governance::ExclusionGroupEdit::Unchanged,
            },
        )))
        .await;
    assert!(
        !matches!(update, IpcResponse::Error { code: 503, .. }),
        "RoleUpdate is not refused for want of governance: {update:?}"
    );
}
