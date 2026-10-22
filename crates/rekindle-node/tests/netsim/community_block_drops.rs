//! Plan E3.1 F3 (D26): a member blocks another by the pseudonym they post
//! under in the community; the blocker's ingest gate drops the blocked
//! member's posts, a third member still sees them, and after the unblock a
//! new post is admitted.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, Events, SimNode};

const LIMIT: Duration = Duration::from_secs(180);

fn saw_body(events: &Events, id: &str) -> bool {
    SimNode::saw(events, |event| {
        matches!(
            event,
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New { message_id, .. })
                if message_id == id
        )
    })
}

fn message_id(posted: &serde_json::Value) -> String {
    posted["messageId"]
        .as_str()
        .expect("a message id")
        .to_owned()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn community_block_drops() {
    let community = Community::of(3, 231).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let ann = community.pseudonym(0).await;
    let block = |request| IpcRequest::Community(request);
    community.nodes[1]
        .ok(block(CommunityRequest::MemberBlock {
            community: community.governance_key.clone(),
            target_pseudonym: ann.clone(),
        }))
        .await;
    let listed = community.nodes[1]
        .ok(block(CommunityRequest::MemberBlockList {
            community: community.governance_key.clone(),
        }))
        .await;
    assert_eq!(listed, serde_json::json!([ann.to_ascii_lowercase()]));
    let refused = community.nodes[1]
        .call(block(CommunityRequest::MemberBlock {
            community: community.governance_key.clone(),
            target_pseudonym: community.pseudonym(1).await,
        }))
        .await
        .expect_err("we cannot block ourselves");
    assert_eq!(refused.code, 403, "{}", refused.message);

    let (bea, cat) = (
        community.nodes[1].events().await,
        community.nodes[2].events().await,
    );
    let hidden = message_id(
        &community
            .post(0, "from ann, blocked")
            .await
            .expect("ann posts"),
    );
    assert!(
        eventually(&community.sim, LIMIT, || async { saw_body(&cat, &hidden) }).await,
        "a member who did not block ann sees the post"
    );
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(!saw_body(&bea, &hidden), "the blocker's gate drops it");

    community.nodes[1]
        .ok(block(CommunityRequest::MemberUnblock {
            community: community.governance_key.clone(),
            target_pseudonym: ann.clone(),
        }))
        .await;
    let shown = message_id(
        &community
            .post(0, "from ann, unblocked")
            .await
            .expect("ann posts"),
    );
    assert!(
        eventually(&community.sim, LIMIT, || async { saw_body(&bea, &shown) }).await,
        "after the unblock the post is admitted"
    );
}
