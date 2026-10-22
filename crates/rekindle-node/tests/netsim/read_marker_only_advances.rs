//! Plan E3.1 G1 (D25): a member's channel read marker only advances. Bea
//! marks the general channel read up to Ann's second post; marking the
//! first post read afterwards leaves the marker where it was, and the
//! Host's `UnreadCounts` counts only the posts after it.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};

use crate::harness::Community;

const LIMIT: Duration = Duration::from_secs(180);
const BEA: usize = 1;

fn message_id(posted: &serde_json::Value) -> String {
    posted["messageId"]
        .as_str()
        .expect("a message id")
        .to_owned()
}

/// Ann posts `body`, and Bea reads it; its message id.
async fn delivered(community: &Community, body: &str) -> String {
    let posted = community.post(0, body).await.expect("ann posts");
    assert!(
        community.reads(BEA, body, LIMIT).await,
        "bea reads {body:?}"
    );
    message_id(&posted)
}

/// The general channel's id, from Bea's view of the community's channels.
async fn general(community: &Community) -> String {
    let info = community.info(BEA).await;
    info["channels"]
        .as_array()
        .expect("channels")
        .iter()
        .find(|ch| ch["name"] == "general")
        .and_then(|ch| ch["id"].as_str())
        .expect("a general channel")
        .to_owned()
}

async fn mark_read(community: &Community, channel: &str, up_to: &str) {
    community.nodes[BEA]
        .ok(IpcRequest::Community(CommunityRequest::ChannelMarkRead {
            community: community.governance_key.clone(),
            channel: channel.to_owned(),
            up_to: up_to.to_owned(),
        }))
        .await;
}

/// Bea's unread count on `channel`, as `UnreadCounts` answers it.
async fn unread(community: &Community, channel: &str) -> u64 {
    let counts = community.nodes[BEA]
        .ok(IpcRequest::Community(CommunityRequest::UnreadCounts {
            community: community.governance_key.clone(),
        }))
        .await;
    counts
        .as_array()
        .expect("unread counts")
        .iter()
        .find(|c| c["channelId"] == channel)
        .and_then(|c| c["unreadCount"].as_u64())
        .expect("the channel's count")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn read_marker_only_advances() {
    let community = Community::of(2, 233).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let channel = general(&community).await;

    let first = delivered(&community, "one").await;
    let second = delivered(&community, "two").await;
    delivered(&community, "three").await;
    assert_eq!(unread(&community, &channel).await, 3, "nothing is read yet");

    mark_read(&community, &channel, &second).await;
    assert_eq!(
        unread(&community, &channel).await,
        1,
        "only the post after the marker counts"
    );

    mark_read(&community, &channel, &first).await;
    assert_eq!(
        unread(&community, &channel).await,
        1,
        "an older message leaves the marker unmoved"
    );

    delivered(&community, "four").await;
    assert_eq!(unread(&community, &channel).await, 2, "a later post counts");
}
