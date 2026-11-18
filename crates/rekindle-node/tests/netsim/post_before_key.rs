//! Plan E3.2 S7l, finding F9-1 (A/B round 9): a joiner whose first
//! `mek_transfer` push is lost posts before it holds the community key.
//! The post is queued, not refused (it was a 500 "no MEK available"), and
//! a later post queues behind it; when the retried push lands, both are
//! sealed under the key that came and written in composition order, and
//! the inviter reads them. Everything the inviter sends is lost
//! (`Fault::DropFrom`, as `mek_transfer_retry`) until the posts are made.

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
async fn a_post_before_the_key_waits_and_is_written_when_the_key_lands() {
    let mut community = Community::create(2, 88).await;
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
    let key = community.anchor.clone();
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
    assert!(
        !holds_a_key(&community.mek_list(1).await),
        "every push so far was lost"
    );
    let first = community
        .post(1, "before the key")
        .await
        .expect("queued, not refused");
    assert_eq!(first["status"], "awaiting_key", "{first}");
    tokio::time::sleep(Duration::from_secs(1)).await;
    let second = community
        .post(1, "behind it")
        .await
        .expect("queued, not refused");
    assert_eq!(
        second["status"], "awaiting_key",
        "a later post waits behind the first: {second}"
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
    let limit = Duration::from_secs(300);
    assert!(
        community.history_holds(0, "before the key", limit).await,
        "the inviter reads the post made before the key"
    );
    assert!(
        community.history_holds(0, "behind it", limit).await,
        "and the one behind it"
    );
    let page = community.history(0, None, 50).await.expect("a page");
    let at = |body: &str| {
        page.iter()
            .find(|m| m["body"] == body)
            .and_then(|m| m["timestamp"].as_u64())
            .expect("a timestamp")
    };
    assert!(
        at("before the key") <= at("behind it"),
        "written in composition order"
    );
}
