//! Plan E3.2 S7l (register "MekTransfer durability", S7 review HIGH-1):
//! the inviter's push of the community key to a joiner (D16) is a
//! `mek_transfer` outbox row, retried until the joiner's `MekTransferAck`
//! receipt. While everything the inviter sends is lost (`Fault::DropFrom`
//! on it: its bootstrap answer and its first pushes), the joiner's join
//! still stands from the DHT; once the loss ends, a retry delivers the
//! key. A one-shot push left the joiner with no key for good.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_netsim::Fault;

use crate::harness::{eventually, Community};

fn holds_a_key(held: &serde_json::Value) -> bool {
    !(held.is_null()
        || held.as_array().is_some_and(Vec::is_empty)
        || held.as_object().is_some_and(serde_json::Map::is_empty))
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_key_push_to_a_joiner_is_retried_until_it_lands() {
    let mut community = Community::create(2, 87).await;
    let link = community.invite().await;
    community.sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 1000,
    });
    community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityJoin { invite: link },
        )))
        .await;
    let key = community.governance_key.clone();
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            community.nodes[1]
                .call(IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityList,
                )))
                .await
                .is_ok_and(|v| v.to_string().contains(&key))
        })
        .await,
        "the join stands from the DHT"
    );
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(
        !holds_a_key(&community.mek_list(1).await),
        "every push so far was lost"
    );
    community.sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 0,
    });
    assert!(
        eventually(&community.sim, Duration::from_secs(300), || async {
            holds_a_key(&community.mek_list(1).await)
        })
        .await,
        "a retried push delivers the joiner's first key"
    );
}
